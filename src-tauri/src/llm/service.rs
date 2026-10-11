//! The single door for every provider call: bounded concurrency with AIMD
//! adaptation, transport retries with jittered backoff, metering events,
//! cancellation. Domain-level retries (batch halving, cleanup iterations)
//! live in the pipeline — this layer only handles transport.

use std::sync::Arc;
use std::time::Duration;

use rand::RngExt;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::events::{LogLevel, LogPhase, RunEvent};
use crate::llm::error::{LlmError, CANCELLED_MSG};
use crate::llm::{LlmDriver, LlmRequest, LlmResponse};

const MAX_ATTEMPTS: u32 = 3;
const SUCCESS_STREAK_FOR_RAISE: u32 = 5;
const BASE_BACKOFF_MS: u64 = 1_000;

struct PermitState {
    limit: u32,
    in_flight: u32,
    streak: u32,
    pause_until: Option<Instant>,
}

pub struct LlmService {
    driver: Arc<dyn LlmDriver>,
    cap: u32,
    state: Mutex<PermitState>,
    notify: Notify,
    cancel: CancellationToken,
    tx: mpsc::Sender<RunEvent>,
}

impl LlmService {
    pub fn new(
        driver: Arc<dyn LlmDriver>,
        cap: u32,
        cancel: CancellationToken,
        tx: mpsc::Sender<RunEvent>,
    ) -> Self {
        let cap = cap.max(1);
        LlmService {
            driver,
            cap,
            state: Mutex::new(PermitState {
                limit: cap,
                in_flight: 0,
                streak: 0,
                pause_until: None,
            }),
            notify: Notify::new(),
            cancel,
            tx,
        }
    }

    /// Test/diagnostic view of the current AIMD limit.
    // Translate view: may surface current concurrency in the status bar.
    #[allow(dead_code)]
    pub fn current_limit(&self) -> u32 {
        self.state.try_lock().map(|s| s.limit).unwrap_or(0)
    }

    async fn acquire(&self) -> Result<(), LlmError> {
        loop {
            if self.cancel.is_cancelled() {
                return Err(LlmError::Transport(CANCELLED_MSG.into()));
            }
            // Register BEFORE checking state so a concurrent release()'s
            // notify_waiters() cannot be lost between the state check and the
            // poll (Notify stores no permit).
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let wait_until = {
                let mut s = self.state.lock().await;
                match s.pause_until {
                    Some(t) if t > Instant::now() => Some(t),
                    _ => {
                        s.pause_until = None;
                        if s.in_flight < s.limit {
                            s.in_flight += 1;
                            return Ok(());
                        }
                        None
                    }
                }
            };
            tokio::select! {
                _ = self.cancel.cancelled() => {
                    return Err(LlmError::Transport(CANCELLED_MSG.into()));
                }
                _ = async {
                    match wait_until {
                        Some(t) => tokio::time::sleep_until(t).await,
                        None => notified.as_mut().await,
                    }
                } => {}
            }
        }
    }

    async fn release(
        &self,
        outcome: &Result<LlmResponse, LlmError>,
        retry_after: Option<Duration>,
    ) {
        let mut s = self.state.lock().await;
        s.in_flight -= 1;
        match outcome {
            Ok(_) => {
                s.streak += 1;
                if s.streak >= SUCCESS_STREAK_FOR_RAISE && s.limit < self.cap {
                    s.limit += 1;
                    s.streak = 0;
                }
            }
            Err(e) if is_throttle(e) => {
                let now = Instant::now();
                // Halve only once per congestion window: 429s arriving while a
                // pause is already active are the same event.
                if s.pause_until.is_none_or(|t| t <= now) {
                    s.limit = (s.limit / 2).max(1);
                }
                s.streak = 0;
                let until = now + retry_after.unwrap_or(Duration::from_millis(BASE_BACKOFF_MS));
                // Clamp to the max: a shorter retry_after must never shorten an
                // existing pause window.
                s.pause_until = Some(s.pause_until.map_or(until, |t| t.max(until)));
            }
            Err(_) => s.streak = 0,
        }
        drop(s);
        self.notify.notify_waiters();
    }

    /// Acquire → stream → classify; transport-retryable errors retried with
    /// jittered exponential backoff, ≤3 attempts total. Auth/parse errors and
    /// cancellation bubble immediately.
    pub async fn request(&self, req: LlmRequest) -> Result<LlmResponse, LlmError> {
        let mut last_err: Option<LlmError> = None;
        for attempt in 1..=MAX_ATTEMPTS {
            if self.cancel.is_cancelled() {
                return Err(LlmError::Transport(CANCELLED_MSG.into()));
            }
            self.acquire().await?;
            let started = Instant::now();
            let outcome = tokio::select! {
                _ = self.cancel.cancelled() => Err(LlmError::Transport(CANCELLED_MSG.into())),
                r = self.driver.stream(&req) => r,
            };
            let retry_after = match &outcome {
                Err(e) if is_throttle(e) => Some(e.retry_after().unwrap_or(Duration::from_millis(
                    BASE_BACKOFF_MS * 2u64.pow(attempt - 1),
                ))),
                _ => None,
            };
            self.release(&outcome, retry_after).await;

            match outcome {
                Ok(resp) => {
                    let _ = self
                        .tx
                        .send(RunEvent::Log {
                            file: None,
                            level: LogLevel::Debug,
                            phase: LogPhase::Llm,
                            message: format!(
                                "{} ok in {:?} (tokens: {})",
                                self.driver.model(),
                                started.elapsed(),
                                resp.usage.summary()
                            ),
                        })
                        .await;
                    return Ok(resp);
                }
                Err(e) if e.is_auth() => return Err(e),
                Err(e) if !e.is_retryable() => return Err(e),
                Err(e) => {
                    if attempt < MAX_ATTEMPTS {
                        let jitter = rand::rng().random::<u64>() % 250;
                        let backoff = Duration::from_millis(
                            BASE_BACKOFF_MS * 2u64.pow(attempt - 1) + jitter,
                        );
                        tokio::select! {
                            _ = self.cancel.cancelled() => {
                                return Err(LlmError::Transport(CANCELLED_MSG.into()));
                            }
                            _ = tokio::time::sleep(backoff) => {}
                        }
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or(LlmError::Empty))
    }
}

fn is_throttle(e: &LlmError) -> bool {
    matches!(e, LlmError::Http { status: 429 | 529, .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::error::LlmError;
    use crate::llm::test_support::ScriptedDriver;
    use crate::llm::LlmRequest;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn service(driver: Arc<ScriptedDriver>, cap: u32) -> LlmService {
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        LlmService::new(driver, cap, CancellationToken::new(), tx)
    }

    fn req() -> LlmRequest {
        LlmRequest { system: "s".into(), user: "u".into() }
    }

    #[tokio::test(start_paused = true)]
    async fn retries_transport_errors_then_succeeds() {
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Transport("reset".into())),
            Err(LlmError::Transport("reset".into())),
            Ok("ok".into()),
        ]);
        let s = service(d.clone(), 2);
        let resp = s.request(req()).await.unwrap();
        assert_eq!(resp.text, "ok");
        assert_eq!(d.call_count(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_after_three_attempts() {
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Transport("x".into())),
            Err(LlmError::Transport("x".into())),
            Err(LlmError::Transport("x".into())),
        ]);
        let s = service(d.clone(), 2);
        assert!(s.request(req()).await.is_err());
        assert_eq!(d.call_count(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn auth_errors_do_not_retry() {
        let d = ScriptedDriver::new(vec![Err(LlmError::Http {
            status: 401,
            body: "no".into(),
            retry_after: None,
        })]);
        let s = service(d.clone(), 2);
        let err = s.request(req()).await.unwrap_err();
        assert!(err.is_auth());
        assert_eq!(d.call_count(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_halves_limit_and_success_streak_restores() {
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Http { status: 429, body: "slow down".into(), retry_after: None }),
            Ok("1".into()),
            Ok("2".into()),
            Ok("3".into()),
            Ok("4".into()),
            Ok("5".into()),
            Ok("6".into()),
        ]);
        let s = service(d, 4);
        assert_eq!(s.current_limit(), 4);
        let _ = s.request(req()).await.unwrap(); // 429 then retry-success
        assert_eq!(s.current_limit(), 2); // halved by the throttle
        for _ in 0..5 {
            let _ = s.request(req()).await.unwrap();
        }
        assert_eq!(s.current_limit(), 3); // +1 after 5-streak
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_aborts_waiting_requests() {
        let d = ScriptedDriver::new(vec![]);
        let cancel = CancellationToken::new();
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let s = LlmService::new(d, 1, cancel.clone(), tx);
        cancel.cancel();
        assert!(s.request(req()).await.is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_requests_respect_limit() {
        // cap 1: two concurrent requests must serialize (2 calls total, both ok).
        let d = ScriptedDriver::new(vec![Ok("a".into()), Ok("b".into())]);
        let s = Arc::new(service(d.clone(), 1));
        let (s1, s2) = (s.clone(), s.clone());
        let (r1, r2) = tokio::join!(s1.request(req()), s2.request(req()));
        assert!(r1.is_ok() && r2.is_ok());
        assert_eq!(d.call_count(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_pause_honors_retry_after_header() {
        let d = ScriptedDriver::new(vec![
            Err(LlmError::Http { status: 429, body: "slow".into(), retry_after: Some(7) }),
            Ok("ok".into()),
        ]);
        let s = service(d, 2);
        let t0 = Instant::now();
        let resp = s.request(req()).await.unwrap();
        assert_eq!(resp.text, "ok");
        // The retry's acquire must wait out the server-mandated 7s pause window;
        // computed backoff alone would be ~1s.
        assert!(t0.elapsed() >= Duration::from_secs(7), "elapsed {:?}", t0.elapsed());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn no_lost_wakeup_under_contention() {
        // cap 1, 8 concurrent requests — all must finish (no hang).
        let d = ScriptedDriver::new((0..8).map(|i| Ok(format!("r{i}"))).collect());
        let (tx, _rx) = tokio::sync::mpsc::channel(64);
        let s = std::sync::Arc::new(LlmService::new(d, 1, CancellationToken::new(), tx));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let s = s.clone();
            handles.push(tokio::spawn(async move {
                s.request(LlmRequest { system: "s".into(), user: "u".into() }).await
            }));
        }
        for h in handles {
            assert!(
                tokio::time::timeout(std::time::Duration::from_secs(10), h)
                    .await
                    .expect("no hang")
                    .unwrap()
                    .is_ok()
            );
        }
    }
}
