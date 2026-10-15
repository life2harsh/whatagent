use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarOutcome {
    Created {
        event: CalendarEvent,
        message: String,
    },
    Task {
        title: String,
        notes: Option<String>,
        due_at: Option<DateTime<Utc>>,
        remind_at: Option<DateTime<Utc>>,
        heard: String,
        note: Option<String>,
    },
    AlreadyAdded {
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
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub recurrence: Option<String>,
}
