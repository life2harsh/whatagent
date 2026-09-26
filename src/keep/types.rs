use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GoogleKeepNoteRequest {
    pub task_id: i64,
    pub chat_id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum GoogleKeepWebhookResponse {
    Created {
        note: GoogleKeepNote,
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GoogleKeepNote {
    pub name: String,
    pub title: String,
}
