use base64::{engine::general_purpose, Engine};
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use serde_json::json;

use crate::error::AgentError;

pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";

#[derive(Clone)]
pub struct GeminiClient {
    http: reqwest::Client,
    base_url: url::Url,
    api_key: String,
    text_model: String,
    audio_model: String,
}

pub enum ExtractionInput {
    Text(String),
    Audio {
        mime_type: Option<String>,
        data: Vec<u8>,
    },
}

pub struct ExtractionContext {
    pub current_time: DateTime<FixedOffset>,
    pub timezone: String,
    pub default_event_duration_minutes: u32,
}

/// The model's structured reading of a request, before validation.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ExtractedEvent {
    pub intent: String,
    pub confidence: f64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub start: Option<String>,
    #[serde(default)]
    pub end: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub recurrence: Option<String>,
    pub needs_confirmation: bool,
    #[serde(default)]
    pub ambiguity_reason: Option<String>,
    #[serde(default)]
    pub transcript: Option<String>,
}

#[derive(Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    content: Option<Content>,
}

#[derive(Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct Part {
    text: Option<String>,
    #[serde(default)]
    thought: bool,
}

impl GeminiClient {
    pub fn new(
        http: reqwest::Client,
        base_url: url::Url,
        api_key: String,
        text_model: String,
        audio_model: String,
    ) -> Self {
        Self {
            http,
            base_url,
            api_key,
            text_model,
            audio_model,
        }
    }

    pub async fn extract_event(
        &self,
        input: ExtractionInput,
        context: &ExtractionContext,
    ) -> Result<ExtractedEvent, AgentError> {
        let (model, user_parts) = match input {
            ExtractionInput::Text(text) => (&self.text_model, json!([{ "text": text }])),
            ExtractionInput::Audio { mime_type, data } => (
                &self.audio_model,
                json!([
                    { "text": "The request is in this WhatsApp voice note." },
                    {
                        "inlineData": {
                            "mimeType": mime_type.as_deref().unwrap_or("audio/ogg; codecs=opus"),
                            "data": general_purpose::STANDARD.encode(data),
                        }
                    }
                ]),
            ),
        };
        let body = json!({
            "systemInstruction": { "parts": [{ "text": system_prompt(context) }] },
            "contents": [{ "role": "user", "parts": user_parts }],
            "generationConfig": {
                "temperature": 0,
                "responseMimeType": "application/json",
                "responseSchema": response_schema(),
            },
        });

        let mut url = self.base_url.clone();
        url.set_path(&format!("/v1beta/models/{model}:generateContent"));
        let response = self
            .http
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::upstream("Gemini", format!("{status}: {body}")));
        }
        let response: GenerateContentResponse = response
            .json()
            .await
            .map_err(|error| AgentError::upstream("Gemini", error.to_string()))?;
        let text: String = response
            .candidates
            .into_iter()
            .next()
            .and_then(|candidate| candidate.content)
            .map(|content| {
                content
                    .parts
                    .into_iter()
                    .filter(|part| !part.thought)
                    .filter_map(|part| part.text)
                    .collect()
            })
            .unwrap_or_default();
        if text.trim().is_empty() {
            return Err(AgentError::upstream("Gemini", "empty response"));
        }
        serde_json::from_str(&text).map_err(|error| {
            AgentError::upstream(
                "Gemini",
                format!("response was not the expected JSON: {error}"),
            )
        })
    }
}

fn system_prompt(context: &ExtractionContext) -> String {
    format!(
        "Current date and time: {current_time}\n\
         User timezone: {timezone}\n\
         Default event duration: {duration} minutes\n\n\
         Convert the user's request into exactly one Google Calendar event. Do not invent details. \
         If the message is clearly not a request to schedule something (for example a bare link or a random note), \
         set intent to not_a_calendar_request. \
         Put any agenda, note, details, or content supplied by the user in description. \
         If the user explicitly requests repetition and the cadence is clear, set recurrence to an RFC 5545 rule \
         without the RRULE: prefix, for example FREQ=WEEKLY;BYDAY=WE. Otherwise set recurrence to null. \
         If time is uncertain or vague, set needs_confirmation to true and explain why in ambiguity_reason, \
         but still fill in your best guess for start and end. \
         If a start time exists but an end time is absent, use the default duration. \
         Resolve relative dates using the supplied current time and timezone. \
         start and end must be RFC 3339 timestamps with a timezone offset. \
         If the request is a voice note, put the verbatim transcript in transcript; otherwise set transcript to null.",
        current_time = context.current_time.to_rfc3339(),
        timezone = context.timezone,
        duration = context.default_event_duration_minutes,
    )
}

fn response_schema() -> serde_json::Value {
    let nullable_string = |description: &str| json!({ "type": "STRING", "nullable": true, "description": description });
    json!({
        "type": "OBJECT",
        "properties": {
            "intent": { "type": "STRING", "enum": ["create_calendar_event", "not_a_calendar_request"] },
            "confidence": { "type": "NUMBER", "description": "0 to 1" },
            "title": nullable_string("Short event title"),
            "description": nullable_string("Agenda or details supplied by the user"),
            "start": nullable_string("RFC 3339 timestamp with timezone offset"),
            "end": nullable_string("RFC 3339 timestamp with timezone offset"),
            "location": nullable_string("Location, if given"),
            "recurrence": nullable_string("RFC 5545 rule without RRULE: prefix, e.g. FREQ=WEEKLY;BYDAY=WE"),
            "needs_confirmation": { "type": "BOOLEAN" },
            "ambiguity_reason": nullable_string("Why the request is ambiguous"),
            "transcript": nullable_string("Verbatim transcript for voice notes"),
        },
        "required": [
            "intent", "confidence", "title", "description", "start", "end", "location",
            "recurrence", "needs_confirmation", "ambiguity_reason", "transcript"
        ],
        "propertyOrdering": [
            "intent", "transcript", "title", "description", "start", "end", "location",
            "recurrence", "needs_confirmation", "ambiguity_reason", "confidence"
        ],
    })
}
