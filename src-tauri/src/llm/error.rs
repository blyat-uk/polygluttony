//! LLM driver error type. Mirrors `llm/anthropic.py:is_retryable_error`.

use thiserror::Error;

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
    /// True for transient failures worth retrying (timeouts, 429, 5xx).
    pub fn is_retryable(&self) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

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
