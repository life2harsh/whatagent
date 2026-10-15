use std::sync::Arc;

use serde::Deserialize;
use serde_json::json;

use crate::{
    calendar::{CalendarEvent, EventCandidate},
    error::AgentError,
    google::auth::AccessTokenSource,
};

pub const DEFAULT_BASE_URL: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Clone)]
pub struct GoogleCalendarClient {
    http: reqwest::Client,
    base_url: url::Url,
    calendar_id: String,
    tokens: Arc<dyn AccessTokenSource>,
}

pub enum InsertOutcome {
    Created(CalendarEvent),
    AlreadyExists,
}

#[derive(Deserialize)]
struct EventResource {
    id: String,
    summary: Option<String>,
    start: EventTime,
    end: EventTime,
    #[serde(rename = "htmlLink")]
    html_link: Option<String>,
}

#[derive(Deserialize)]
struct EventTime {
    #[serde(rename = "dateTime")]
    date_time: chrono::DateTime<chrono::FixedOffset>,
}

impl GoogleCalendarClient {
    pub fn new(
        http: reqwest::Client,
        base_url: url::Url,
        calendar_id: String,
        tokens: Arc<dyn AccessTokenSource>,
    ) -> Self {
        Self {
            http,
            base_url,
            calendar_id,
            tokens,
        }
    }

    pub async fn insert_event(
        &self,
        event_id: &str,
        candidate: &EventCandidate,
        timezone: &str,
    ) -> Result<InsertOutcome, AgentError> {
        let url = self.events_url()?;
        let mut body = json!({
            "id": event_id,
            "summary": candidate.title,
            "start": { "dateTime": candidate.start.to_rfc3339(), "timeZone": timezone },
            "end": { "dateTime": candidate.end.to_rfc3339(), "timeZone": timezone },
        });
        if let Some(description) = &candidate.description {
            body["description"] = json!(description);
        }
        if let Some(location) = &candidate.location {
            body["location"] = json!(location);
        }
        if let Some(rule) = &candidate.recurrence {
            body["recurrence"] = json!([format!("RRULE:{rule}")]);
        }

        let token = self.tokens.access_token().await?;
        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if status == reqwest::StatusCode::CONFLICT {
            return Ok(InsertOutcome::AlreadyExists);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::upstream_status(
                "Google Calendar",
                status,
                &body,
            ));
        }
        let event: EventResource = response
            .json()
            .await
            .map_err(|error| AgentError::upstream("Google Calendar", error.to_string()))?;
        Ok(InsertOutcome::Created(CalendarEvent {
            id: event.id,
            title: event.summary.unwrap_or_else(|| candidate.title.clone()),
            start: event.start.date_time,
            end: event.end.date_time,
            calendar: self.calendar_id.clone(),
            html_url: event.html_link,
        }))
    }

    pub async fn check_access(&self) -> Result<(), AgentError> {
        let mut url = self.events_url()?;
        url.query_pairs_mut().append_pair("maxResults", "1");
        let token = self.tokens.access_token().await?;
        let response = self.http.get(url).bearer_auth(token).send().await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::upstream_status(
                "Google Calendar",
                status,
                &body,
            ));
        }
        Ok(())
    }

    fn events_url(&self) -> Result<url::Url, AgentError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|_| AgentError::Config("Google Calendar base URL is invalid".to_owned()))?
            .pop_if_empty()
            .extend(["calendars", &self.calendar_id, "events"]);
        Ok(url)
    }
}

pub fn event_id_for(message_id: &str) -> String {
    message_id
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
