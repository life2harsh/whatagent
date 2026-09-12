use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("n8n returned an invalid response: {0}")]
    InvalidWebhookResponse(String),
    #[error("n8n request failed: {0}")]
    Webhook(#[from] reqwest::Error),
    #[error("WhatsApp media download failed: {0}")]
    MediaDownload(String),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}
