use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("{service} returned an unexpected response: {message}")]
    Upstream {
        service: &'static str,
        status: Option<u16>,
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
    /// An upstream failure that is not an HTTP status (bad JSON, empty reply, ...).
    pub fn upstream(service: &'static str, message: impl Into<String>) -> Self {
        Self::Upstream {
            service,
            status: None,
            message: truncate(message.into()),
        }
    }

    /// An upstream non-success HTTP response.
    pub fn upstream_status(service: &'static str, status: reqwest::StatusCode, body: &str) -> Self {
        Self::Upstream {
            service,
            status: Some(status.as_u16()),
            message: truncate(format!("{status}: {body}")),
        }
    }

    /// Overloads, rate limits, server errors and network failures that may
    /// succeed on another attempt or another model.
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Upstream {
                status: Some(status),
                ..
            } => matches!(status, 408 | 429 | 500 | 502 | 503 | 504),
            Self::Http(error) => error.is_timeout() || error.is_connect() || error.is_request(),
            _ => false,
        }
    }

    /// A short, human-readable explanation for the WhatsApp reply. The full
    /// error (with Google's raw response) goes to the logs instead.
    pub fn user_message(&self) -> String {
        let text = match self {
            Self::Upstream {
                service: "Gemini", ..
            } if self.is_transient() => {
                "Gemini is overloaded right now (tried every model). Send it again in a minute."
            }
            Self::Upstream {
                service: "Gemini",
                status: Some(400),
                ..
            } => "Gemini couldn't process that message. Try rephrasing it.",
            Self::Upstream {
                service: "Gemini",
                status: Some(401 | 403),
                ..
            } => "Gemini rejected the API key. Check GEMINI_API_KEY on the server.",
            Self::Upstream {
                service: "Gemini",
                status: None,
                ..
            } => "I couldn't make sense of that one. Try rephrasing it.",
            Self::Upstream {
                service: "Google Calendar",
                status: Some(403),
                message,
            } if message.contains("has not been used") || message.contains("is disabled") => {
                "The Google Calendar API is turned off for the service account's project. Enable it in Google Cloud Console."
            }
            Self::Upstream {
                service: "Google Calendar",
                status: Some(403 | 404),
                ..
            } => "I don't have access to your calendar. Check it's shared with the service account (\"Make changes to events\") and that GOOGLE_CALENDAR_ID is right.",
            Self::Upstream {
                service: "Google Calendar",
                status: Some(400),
                ..
            } => "Google Calendar rejected the event details. Try rephrasing it.",
            Self::Upstream {
                service: "Google Calendar",
                ..
            } if self.is_transient() => "Google Calendar is having trouble right now. Send it again in a minute.",
            Self::Upstream {
                service: "Google auth",
                ..
            } => "I couldn't sign in to Google. The service account key on the server may be invalid or deleted.",
            Self::Http(error) if error.is_timeout() => "Google took too long to answer. Send it again in a minute.",
            Self::Http(_) => "I couldn't reach Google right now. Send it again in a minute.",
            Self::MediaDownload(_) => "I couldn't download that voice note. Try sending it again.",
            Self::Config(_) => "The bot is misconfigured. Check the server logs.",
            _ => "Something went wrong on my side. Check the server logs.",
        };
        text.to_owned()
    }
}

fn truncate(mut message: String) -> String {
    const LIMIT: usize = 600;
    if message.len() > LIMIT {
        let cut = (0..=LIMIT)
            .rev()
            .find(|&index| message.is_char_boundary(index))
            .unwrap_or(0);
        message.truncate(cut);
        message.push('…');
    }
    message
}
