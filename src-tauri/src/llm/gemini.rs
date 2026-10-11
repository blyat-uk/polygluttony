//! Native Gemini API driver — placeholder so `Driver::Gemini` connections
//! compile and fail clearly until the real driver replaces this file.

use async_trait::async_trait;

use crate::config::Connection;
use crate::llm::error::LlmError;
use crate::llm::{LlmDriver, LlmRequest, LlmResponse};

pub struct GeminiDriver {
    conn: Connection,
}

impl GeminiDriver {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }
}

fn not_implemented() -> LlmError {
    LlmError::Transport("Gemini driver not implemented yet".into())
}

#[async_trait]
impl LlmDriver for GeminiDriver {
    async fn complete(&self, _system: &str, _user: &str) -> Result<String, LlmError> {
        Err(not_implemented())
    }

    async fn stream(&self, _req: &LlmRequest) -> Result<LlmResponse, LlmError> {
        Err(not_implemented())
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Err(not_implemented())
    }

    fn model(&self) -> &str {
        &self.conn.model
    }
}
