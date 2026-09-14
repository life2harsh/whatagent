use chrono::{DateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingCommand {
    pub message_id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub timestamp: DateTime<Utc>,
    pub content: CommandContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandContent {
    Text(String),
    Voice {
        mime_type: Option<String>,
        data: Vec<u8>,
    },
    UnsupportedMedia,
}
