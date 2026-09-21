use reqwest::{header, Client};

use crate::{
    calendar::{CalendarWebhookRequest, CalendarWebhookResponse, ConfirmationRequest},
    error::AgentError,
};

#[derive(Clone)]
pub struct CalendarWebhookClient {
    http: Client,
    calendar_url: url::Url,
    audio_url: url::Url,
    confirm_url: url::Url,
    secret: String,
}

impl CalendarWebhookClient {
    pub fn new(
        http: Client,
        calendar_url: url::Url,
        audio_url: url::Url,
        confirm_url: url::Url,
        secret: String,
    ) -> Self {
        Self {
            http,
            calendar_url,
            audio_url,
            confirm_url,
            secret,
        }
    }

    pub async fn submit(
        &self,
        request: CalendarWebhookRequest,
    ) -> Result<CalendarWebhookResponse, AgentError> {
        let idempotency_key = request.message_id().to_owned();
        let response = match request {
            request @ CalendarWebhookRequest::Text { .. } => {
                self.http
                    .post(self.calendar_url.clone())
                    .header(header::AUTHORIZATION, format!("Bearer {}", self.secret))
                    .header("X-Idempotency-Key", &idempotency_key)
                    .json(&request)
                    .send()
                    .await?
            }
            CalendarWebhookRequest::Audio {
                message_id,
                chat_id,
                timestamp,
                timezone,
                current_time,
                default_event_duration_minutes,
                mime_type,
                data,
            } => {
                let mime = mime_type.as_deref().unwrap_or("application/octet-stream");
                let audio = reqwest::multipart::Part::bytes(data)
                    .mime_str(mime)
                    .map_err(|error| AgentError::InvalidWebhookResponse(error.to_string()))?
                    .file_name("voice-note");
                let mut form = reqwest::multipart::Form::new()
                    .text("type", "audio")
                    .text("message_id", message_id)
                    .text("chat_id", chat_id)
                    .text("timestamp", timestamp.to_rfc3339())
                    .text("timezone", timezone)
                    .text("current_time", current_time.to_rfc3339())
                    .text(
                        "default_event_duration_minutes",
                        default_event_duration_minutes.to_string(),
                    )
                    .part("audio", audio);
                if let Some(mime_type) = mime_type {
                    form = form.text("mime_type", mime_type);
                }
                self.http
                    .post(self.audio_url.clone())
                    .header(header::AUTHORIZATION, format!("Bearer {}", self.secret))
                    .header("X-Idempotency-Key", &idempotency_key)
                    .multipart(form)
                    .send()
                    .await?
            }
        };

        let response = response.error_for_status()?;
        response
            .json()
            .await
            .map_err(|error| AgentError::InvalidWebhookResponse(error.to_string()))
    }

    pub async fn confirm(
        &self,
        request: &ConfirmationRequest,
    ) -> Result<CalendarWebhookResponse, AgentError> {
        let response = self
            .http
            .post(self.confirm_url.clone())
            .header(header::AUTHORIZATION, format!("Bearer {}", self.secret))
            .header("X-Idempotency-Key", &request.request_id)
            .json(request)
            .send()
            .await?
            .error_for_status()?;
        response
            .json()
            .await
            .map_err(|error| AgentError::InvalidWebhookResponse(error.to_string()))
    }
}
