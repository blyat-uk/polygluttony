//! Why a run stopped. The run's cancellation token plus the first reason it
//! tripped: the user's Stop, or an error no retry fixes (a rejected key, an
//! account out of credit, an unknown model). Every file task shares one, so
//! the file that hit the error and the files it took down with it all end
//! the same honest way: stopped, with one sentence saying why.

use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use crate::config::Step;
use crate::events::{ErrorKind, StopInfo};

/// An error that stopped the run, with what's needed to say so plainly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopCause {
    pub kind: ErrorKind,
    pub step: Step,
    /// Who answered: "Anthropic", or "The server" for a custom URL.
    pub who: String,
    pub model: String,
    /// A model on the user's own computer: there's no key to blame.
    pub local: bool,
}

impl StopCause {
    /// One plain sentence for the banner and the stored reason.
    pub fn sentence(&self) -> String {
        let (who, step, model) = (&self.who, self.step.label(), &self.model);
        match self.kind {
            ErrorKind::Auth if self.local => format!(
                "{who} refused the request for {step}. Check the address, and that the model is installed."
            ),
            ErrorKind::Auth => format!("{who} rejected the API key used for {step}."),
            ErrorKind::Billing => {
                format!("{who} says the account used for {step} is out of credit.")
            }
            ErrorKind::Model => {
                format!("{who} doesn't recognise the model “{model}” used for {step}.")
            }
            ErrorKind::RateLimit => format!("{who} kept asking us to slow down during {step}."),
            ErrorKind::Network => format!("Couldn't reach {who} for {step}."),
            ErrorKind::Server => format!("{who} kept failing on its side during {step}."),
            ErrorKind::Other => format!("{step} hit an error it couldn't get past."),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Reason {
    User,
    Error(StopCause),
}

/// Shared by the run, its file tasks and the Stop command. Cloning shares it.
#[derive(Debug, Clone, Default)]
pub struct RunStop {
    token: CancellationToken,
    reason: Arc<Mutex<Option<Reason>>>,
}

impl RunStop {
    #[cfg(test)]
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            reason: Arc::default(),
        }
    }

    /// The token every request and file task watches.
    pub fn token(&self) -> &CancellationToken {
        &self.token
    }

    pub fn is_stopped(&self) -> bool {
        self.token.is_cancelled()
    }

    /// The user pressed Stop.
    pub fn by_user(&self) {
        self.stop(Reason::User);
    }

    /// An error stopped the run. Only the first reason counts: the files the
    /// stop takes down fail with cancellations, not causes.
    pub fn by_error(&self, cause: StopCause) {
        self.stop(Reason::Error(cause));
    }

    fn stop(&self, reason: Reason) {
        let mut slot = self.reason.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() && !self.token.is_cancelled() {
            *slot = Some(reason);
        }
        drop(slot);
        self.token.cancel();
    }

    fn reason(&self) -> Option<Reason> {
        self.reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The error that stopped the run, if one did.
    pub fn cause(&self) -> Option<StopCause> {
        match self.reason() {
            Some(Reason::Error(cause)) => Some(cause),
            _ => None,
        }
    }

    /// The `Stopped` event's payload, once the run has stopped. A token
    /// cancelled with no recorded reason counts as the user's Stop.
    pub fn info(&self) -> Option<StopInfo> {
        if !self.is_stopped() {
            return None;
        }
        Some(match self.cause() {
            Some(cause) => StopInfo {
                by_user: false,
                message: cause.sentence(),
                kind: Some(cause.kind),
                step: Some(cause.step),
            },
            None => StopInfo {
                by_user: true,
                message: String::new(),
                kind: None,
                step: None,
            },
        })
    }

    /// Why an unfinished episode stopped, for its stored record.
    pub fn reason_sentence(&self) -> String {
        match self.cause() {
            Some(cause) => cause.sentence(),
            None => "You stopped the run.".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cause(kind: ErrorKind, step: Step) -> StopCause {
        StopCause {
            kind,
            step,
            who: "Anthropic".into(),
            model: "claude-x".into(),
            local: false,
        }
    }

    #[test]
    fn a_local_model_is_never_blamed_for_a_key() {
        let mut c = cause(ErrorKind::Auth, Step::Translate);
        c.who = "Ollama".into();
        c.local = true;
        assert_eq!(
            c.sentence(),
            "Ollama refused the request for Translate. Check the address, and that the model is installed."
        );
    }

    #[test]
    fn sentences_name_who_what_and_which_step() {
        assert_eq!(
            cause(ErrorKind::Auth, Step::Translate).sentence(),
            "Anthropic rejected the API key used for Translate."
        );
        assert_eq!(
            cause(ErrorKind::Billing, Step::Verify).sentence(),
            "Anthropic says the account used for Verify is out of credit."
        );
        assert_eq!(
            cause(ErrorKind::Model, Step::Translate).sentence(),
            "Anthropic doesn't recognise the model “claude-x” used for Translate."
        );
    }

    #[test]
    fn the_first_reason_wins() {
        let stop = RunStop::default();
        assert!(stop.info().is_none());
        stop.by_error(cause(ErrorKind::Auth, Step::Translate));
        stop.by_user();
        stop.by_error(cause(ErrorKind::Billing, Step::Verify));
        let info = stop.info().unwrap();
        assert!(!info.by_user);
        assert_eq!(info.kind, Some(ErrorKind::Auth));
        assert_eq!(info.step, Some(Step::Translate));
        assert_eq!(
            info.message,
            "Anthropic rejected the API key used for Translate."
        );
        assert!(stop.token().is_cancelled());
    }

    #[test]
    fn a_user_stop_says_so() {
        let stop = RunStop::default();
        let shared = stop.clone();
        shared.by_user();
        let info = stop.info().unwrap();
        assert!(info.by_user && info.message.is_empty() && info.kind.is_none());
        assert_eq!(stop.reason_sentence(), "You stopped the run.");
        // A late error doesn't rewrite history.
        stop.by_error(cause(ErrorKind::Auth, Step::Translate));
        assert!(stop.info().unwrap().by_user);
    }

    #[test]
    fn a_bare_cancel_counts_as_the_user() {
        let token = CancellationToken::new();
        let stop = RunStop::new(token.clone());
        token.cancel();
        assert!(stop.info().unwrap().by_user);
    }
}
