use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("{service} returned an unexpected response: {message}")]
    Upstream {
        service: &'static str,
        message: String,
    },
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("WhatsApp media download failed: {0}")]
    MediaDownload(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl AgentError {
    pub fn upstream(service: &'static str, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.len() > 300 {
            let cut = (0..=300)
                .rev()
                .find(|&index| message.is_char_boundary(index))
                .unwrap_or(0);
            message.truncate(cut);
            message.push('…');
        }
        Self::Upstream { service, message }
    }
}
