//! LLM driver error type. Mirrors `llm/anthropic.py:is_retryable_error`.

use thiserror::Error;

use crate::events::ErrorKind;

/// Message every request fails with (as [`LlmError::Transport`]) once the
/// run's CancellationToken trips. Pipelines match on it to tell "stopped
/// because the run stopped" apart from real transport failures.
pub const CANCELLED_MSG: &str = "run cancelled";

#[derive(Debug, Error)]
pub enum LlmError {
    /// Non-2xx HTTP response, carrying the status + a body snippet.
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String, retry_after: Option<u64> },
    /// Transport-level failure (timeout, connection reset, DNS).
    #[error("request error: {0}")]
    Transport(String),
    /// Response received but could not be parsed into the expected shape.
    #[error("failed to parse response: {0}")]
    Parse(String),
    /// 2xx response with no usable text content.
    #[error("empty response from LLM")]
    Empty,
    /// 2xx response the provider deliberately ended without an answer — a
    /// safety block, or a server-side tool loop that never finished. Shown
    /// as-is (no "failed to parse" framing) and never retried: re-sending
    /// gets the same verdict, and would re-pay for any web searches.
    #[error("{0}")]
    Declined(String),
}

impl LlmError {
    /// True for transient failures worth retrying (timeouts, 429, 5xx). An
    /// account out of credit is never transient, even when it comes as a 429.
    pub fn is_retryable(&self) -> bool {
        if self.kind() == ErrorKind::Billing {
            return false;
        }
        match self {
            LlmError::Http { status, .. } => {
                matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 529)
            }
            LlmError::Transport(_) => true,
            LlmError::Empty => true,
            LlmError::Parse(_) | LlmError::Declined(_) => false,
        }
    }

    /// True for auth/endpoint errors that won't be fixed by retrying.
    pub fn is_auth(&self) -> bool {
        matches!(self, LlmError::Http { status, .. } if matches!(status, 401 | 403 | 404))
    }

    /// What kind of failure this is, so the UI can say what to do next.
    pub fn kind(&self) -> ErrorKind {
        match self {
            LlmError::Http { status, body, .. } => http_kind(*status, body),
            LlmError::Transport(msg) if msg == CANCELLED_MSG => ErrorKind::Other,
            LlmError::Transport(_) => ErrorKind::Network,
            LlmError::Parse(_) | LlmError::Empty | LlmError::Declined(_) => ErrorKind::Other,
        }
    }

    /// Errors no retry and no smaller batch will fix: a rejected key, an
    /// account out of credit, a model the provider doesn't know. A run stops
    /// on these instead of grinding through every remaining line.
    pub fn is_fatal(&self) -> bool {
        matches!(self.kind(), ErrorKind::Auth | ErrorKind::Billing | ErrorKind::Model)
    }

    /// True when this error is the service's cancellation signal — a
    /// consequence of an abort/cancel, not a cause worth recording.
    pub fn is_cancelled(&self) -> bool {
        matches!(self, LlmError::Transport(msg) if msg == CANCELLED_MSG)
    }

    /// Server-provided `Retry-After`, when the throttling response carried one.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            LlmError::Http { retry_after: Some(s), .. } => {
                Some(std::time::Duration::from_secs(*s))
            }
            _ => None,
        }
    }
}

/// Pure: an HTTP failure's kind from its status and body. Providers disagree
/// on statuses (Anthropic says "out of credit" with a 400, OpenAI with a 429,
/// Gemini says "bad key" with a 400), so the body breaks the ties.
fn http_kind(status: u16, body: &str) -> ErrorKind {
    let body = body.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| body.contains(n));
    // Gemini's per-minute limits also say "exceeded your current quota …
    // billing", so a 429 counts as billing only when it names credit outright.
    let billing = match status {
        402 => true,
        429 => has(&["insufficient_quota", "credit"]),
        400 | 403 => has(&["credit", "quota", "billing", "payment"]),
        _ => false,
    };
    if billing {
        return ErrorKind::Billing;
    }
    match status {
        401 | 403 => ErrorKind::Auth,
        404 => ErrorKind::Model,
        400 if has(&["api key", "api_key"]) => ErrorKind::Auth,
        400 if body.contains("model")
            && has(&["not found", "does not exist", "unknown", "invalid model", "not supported"]) =>
        {
            ErrorKind::Model
        }
        408 => ErrorKind::Network,
        429 => ErrorKind::RateLimit,
        500..=599 => ErrorKind::Server,
        _ => ErrorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, body: &str) -> LlmError {
        LlmError::Http { status, body: body.into(), retry_after: None }
    }

    #[test]
    fn kinds_from_statuses() {
        assert_eq!(http(401, "invalid x-api-key").kind(), ErrorKind::Auth);
        assert_eq!(http(403, "permission_error").kind(), ErrorKind::Auth);
        assert_eq!(http(402, "").kind(), ErrorKind::Billing);
        assert_eq!(http(404, "model: claude-x").kind(), ErrorKind::Model);
        assert_eq!(http(429, "rate_limit_error").kind(), ErrorKind::RateLimit);
        assert_eq!(http(408, "").kind(), ErrorKind::Network);
        for s in [500u16, 502, 503, 529] {
            assert_eq!(http(s, "overloaded").kind(), ErrorKind::Server, "{s}");
        }
        assert_eq!(http(422, "bad").kind(), ErrorKind::Other);
    }

    #[test]
    fn kinds_from_bodies() {
        // Anthropic: out of credit comes as a 400.
        let anthropic = http(
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#,
        );
        assert_eq!(anthropic.kind(), ErrorKind::Billing);
        // OpenAI: out of quota comes as a 429 with its own code.
        let openai = http(429, r#"{"error":{"code":"insufficient_quota","message":"You exceeded your current quota"}}"#);
        assert_eq!(openai.kind(), ErrorKind::Billing);
        // Gemini's per-minute limit mentions quota and billing but is a rate limit.
        let gemini = http(
            429,
            "You exceeded your current quota, please check your plan and billing details. Please retry in 41s.",
        );
        assert_eq!(gemini.kind(), ErrorKind::RateLimit);
        // Gemini: a bad key comes as a 400.
        assert_eq!(http(400, "API key not valid. API_KEY_INVALID").kind(), ErrorKind::Auth);
        assert_eq!(http(400, "The model `gpt-9` does not exist").kind(), ErrorKind::Model);
        assert_eq!(http(400, "max_tokens too large").kind(), ErrorKind::Other);
    }

    #[test]
    fn kinds_of_non_http_errors() {
        assert_eq!(LlmError::Transport("dns error".into()).kind(), ErrorKind::Network);
        assert_eq!(LlmError::Transport(CANCELLED_MSG.into()).kind(), ErrorKind::Other);
        assert_eq!(LlmError::Parse("x".into()).kind(), ErrorKind::Other);
        assert_eq!(LlmError::Empty.kind(), ErrorKind::Other);
    }

    #[test]
    fn fatal_errors_are_key_credit_and_model() {
        assert!(http(401, "").is_fatal());
        assert!(http(402, "").is_fatal());
        assert!(http(404, "").is_fatal());
        assert!(http(400, "API_KEY_INVALID").is_fatal());
        assert!(!http(429, "slow down").is_fatal());
        assert!(!http(503, "").is_fatal());
        assert!(!LlmError::Transport("reset".into()).is_fatal());
    }

    #[test]
    fn billing_is_never_retried() {
        assert!(!http(429, "insufficient_quota").is_retryable());
        assert!(http(429, "slow down").is_retryable());
    }

    #[test]
    fn auth_errors_are_not_retryable() {
        for s in [401u16, 403, 404] {
            let e = LlmError::Http { status: s, body: "x".into(), retry_after: None };
            assert!(!e.is_retryable(), "{s} should not retry");
            assert!(e.is_auth(), "{s} is auth");
        }
    }

    #[test]
    fn transient_errors_are_retryable() {
        for s in [429u16, 500, 502, 503, 504] {
            assert!(LlmError::Http { status: s, body: "x".into(), retry_after: None }.is_retryable());
        }
        assert!(LlmError::Transport("timed out".into()).is_retryable());
    }

    #[test]
    fn parse_errors_are_not_retryable() {
        assert!(!LlmError::Parse("bad json".into()).is_retryable());
        assert!(!LlmError::Declined("blocked".into()).is_retryable());
    }

    #[test]
    fn retry_after_helper_maps_seconds() {
        let e = LlmError::Http { status: 429, body: "x".into(), retry_after: Some(7) };
        assert_eq!(e.retry_after(), Some(std::time::Duration::from_secs(7)));
        let none = LlmError::Http { status: 429, body: "x".into(), retry_after: None };
        assert_eq!(none.retry_after(), None);
        assert_eq!(LlmError::Empty.retry_after(), None);
    }
}
