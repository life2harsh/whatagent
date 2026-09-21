use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CalendarWebhookRequest {
    Text {
        message_id: String,
        chat_id: String,
        timestamp: DateTime<FixedOffset>,
        timezone: String,
        current_time: DateTime<FixedOffset>,
        default_event_duration_minutes: u32,
        text: String,
    },
    Audio {
        message_id: String,
        chat_id: String,
        timestamp: DateTime<FixedOffset>,
        timezone: String,
        current_time: DateTime<FixedOffset>,
        default_event_duration_minutes: u32,
        mime_type: Option<String>,
        #[serde(skip_serializing)]
        data: Vec<u8>,
    },
}

impl CalendarWebhookRequest {
    pub fn message_id(&self) -> &str {
        match self {
            Self::Text { message_id, .. } | Self::Audio { message_id, .. } => message_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfirmationRequest {
    pub request_id: String,
    pub approved: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CalendarWebhookResponse {
    Created {
        event: CalendarEvent,
        message: String,
    },
    NeedsConfirmation {
        request_id: String,
        candidate: EventCandidate,
        message: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub calendar: String,
    pub html_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EventCandidate {
    pub title: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
}
