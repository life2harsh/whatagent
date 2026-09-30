use reqwest::{header, Client};

use crate::{
    error::AgentError,
    keep::{GoogleKeepNoteRequest, GoogleKeepWebhookResponse},
};

#[derive(Clone)]
pub struct GoogleKeepWebhookClient {
    http: Client,
    url: url::Url,
    secret: String,
}

impl GoogleKeepWebhookClient {
    pub fn new(http: Client, url: url::Url, secret: String) -> Self {
        Self { http, url, secret }
    }

    pub async fn create_note(
        &self,
        request: &GoogleKeepNoteRequest,
    ) -> Result<GoogleKeepWebhookResponse, AgentError> {
        let response = self
            .http
            .post(self.url.clone())
            .header(header::AUTHORIZATION, format!("Bearer {}", self.secret))
            .header(
                "X-Idempotency-Key",
                format!("keep-task-{}", request.task_id),
            )
            .json(request)
            .send()
            .await?
            .error_for_status()?;
        response
            .json()
            .await
            .map_err(|error| AgentError::upstream("Keep bridge", error.to_string()))
    }
}
