use std::time::Duration;

use base64::{engine::general_purpose, Engine};
use chrono::{DateTime, FixedOffset};
use serde::Deserialize;
use serde_json::json;
use tracing::warn;

use crate::error::AgentError;

pub const DEFAULT_BASE_URL: &str = "https://generativelanguage.googleapis.com";

#[derive(Clone)]
pub struct GeminiClient {
    http: reqwest::Client,
    base_url: url::Url,
    api_key: String,
    text_models: Vec<String>,
    audio_models: Vec<String>,
    retry_delay: Duration,
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
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub remind: Option<String>,
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
        text_models: Vec<String>,
        audio_models: Vec<String>,
    ) -> Self {
        Self {
            http,
            base_url,
            api_key,
            text_models,
            audio_models,
            retry_delay: Duration::from_secs(3),
        }
    }

    pub fn with_retry_delay(mut self, retry_delay: Duration) -> Self {
        self.retry_delay = retry_delay;
        self
    }

    pub async fn extract_event(
        &self,
        input: ExtractionInput,
        context: &ExtractionContext,
    ) -> Result<ExtractedEvent, AgentError> {
        let (models, user_parts) = match input {
            ExtractionInput::Text(text) => (&self.text_models, json!([{ "text": text }])),
            ExtractionInput::Audio { mime_type, data } => (
                &self.audio_models,
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

        let mut last_error = None;
        for round in 0..2 {
            if round > 0 {
                tokio::time::sleep(self.retry_delay).await;
            }
            for model in models {
                match self.generate(model, &body).await {
                    Ok(event) => return Ok(event),
                    Err(error) if worth_another_model(&error) => {
                        warn!(%model, round, %error, "Gemini attempt failed; trying the next model");
                        last_error = Some(error);
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Err(last_error
            .unwrap_or_else(|| AgentError::Config("no Gemini models are configured".to_owned())))
    }

    pub async fn check(&self) -> Result<String, AgentError> {
        let model = self
            .text_models
            .first()
            .ok_or_else(|| AgentError::Config("no Gemini models are configured".to_owned()))?;
        let mut url = self.base_url.clone();
        url.set_path(&format!("/v1beta/models/{model}"));
        let response = self
            .http
            .get(url)
            .header("x-goog-api-key", &self.api_key)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::upstream_status("Gemini", status, &body));
        }
        Ok(model.clone())
    }

    async fn generate(
        &self,
        model: &str,
        body: &serde_json::Value,
    ) -> Result<ExtractedEvent, AgentError> {
        let mut url = self.base_url.clone();
        url.set_path(&format!("/v1beta/models/{model}:generateContent"));
        let response = self
            .http
            .post(url)
            .header("x-goog-api-key", &self.api_key)
            .json(body)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::upstream_status("Gemini", status, &body));
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

fn worth_another_model(error: &AgentError) -> bool {
    error.is_transient()
        || matches!(
            error,
            AgentError::Upstream {
                status: None | Some(404),
                ..
            }
        )
}

fn system_prompt(context: &ExtractionContext) -> String {
    format!(
        "Current date and time: {current_time}\n\
         User timezone: {timezone}\n\
         Default event duration: {duration} minutes\n\n\
         Read the user's request and choose one intent. Do not invent details. \
         create_calendar_event: a meeting or event at a specific time, for example \"dentist at 5pm\" \
         or \"call with Sam Friday 2pm\". Fill in title, start, end, and the other event fields. \
         If a start time exists but an end time is absent, use the default duration. \
         If time is uncertain or vague, set needs_confirmation to true and explain why in ambiguity_reason, \
         but still fill in your best guess for start and end. \
         If the user explicitly requests repetition and the cadence is clear, set recurrence to an RFC 5545 rule \
         without the RRULE: prefix, for example FREQ=WEEKLY;BYDAY=WE. Otherwise set recurrence to null. \
         create_task: any to-do, errand, shopping item, or thing to remember that is not tied to a specific \
         meeting time, even when no time is given, for example \"buy earbuds\", \"remind me to buy milk\", or \
         \"submit the report by Friday\". A short imperative or action item is a task. Put a short task name in \
         title and any details in description. Set due only if the user gave a deadline, and remind only if the \
         user wants to be reminded at a time; otherwise leave both null. Leave start, end, location, and \
         recurrence null. \
         not_a_calendar_request: only when the message is neither an event nor a to-do, for example a bare link, \
         a greeting, a question, or a plain statement with nothing to do. When a message is actionable but you \
         are unsure whether it is an event or a task, prefer create_task. \
         Resolve relative dates using the supplied current time and timezone. \
         All times (start, end, due, remind) must be RFC 3339 timestamps with a timezone offset. \
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
            "intent": { "type": "STRING", "enum": ["create_calendar_event", "create_task", "not_a_calendar_request"] },
            "confidence": { "type": "NUMBER", "description": "0 to 1" },
            "title": nullable_string("Short event or task title"),
            "description": nullable_string("Agenda or details supplied by the user"),
            "start": nullable_string("RFC 3339 timestamp with timezone offset"),
            "end": nullable_string("RFC 3339 timestamp with timezone offset"),
            "location": nullable_string("Location, if given"),
            "recurrence": nullable_string("RFC 5545 rule without RRULE: prefix, e.g. FREQ=WEEKLY;BYDAY=WE"),
            "needs_confirmation": { "type": "BOOLEAN" },
            "ambiguity_reason": nullable_string("Why the request is ambiguous"),
            "transcript": nullable_string("Verbatim transcript for voice notes"),
            "due": nullable_string("RFC 3339 deadline for a task, if the user gave one"),
            "remind": nullable_string("RFC 3339 time to remind for a task, if requested"),
        },
        "required": [
            "intent", "confidence", "title", "description", "start", "end", "location",
            "recurrence", "needs_confirmation", "ambiguity_reason", "transcript", "due", "remind"
        ],
        "propertyOrdering": [
            "intent", "transcript", "title", "description", "start", "end", "location",
            "recurrence", "due", "remind", "needs_confirmation", "ambiguity_reason", "confidence"
        ],
    })
}
