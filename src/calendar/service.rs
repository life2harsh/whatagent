use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::{
    calendar::{
        validate::{validate, Validation},
        CalendarOutcome, EventCandidate,
    },
    error::AgentError,
    google::{
        calendar::{event_id_for, GoogleCalendarClient, InsertOutcome},
        gemini::{ExtractionContext, ExtractionInput, GeminiClient},
    },
};

/// Turns a text or voice request into a Google Calendar event.
pub struct CalendarService {
    gemini: GeminiClient,
    calendar: GoogleCalendarClient,
    timezone: Tz,
    default_event_duration_minutes: u32,
}

impl CalendarService {
    pub fn new(
        gemini: GeminiClient,
        calendar: GoogleCalendarClient,
        timezone: Tz,
        default_event_duration_minutes: u32,
    ) -> Self {
        Self {
            gemini,
            calendar,
            timezone,
            default_event_duration_minutes,
        }
    }

    pub async fn handle(
        &self,
        message_id: &str,
        input: ExtractionInput,
    ) -> Result<CalendarOutcome, AgentError> {
        let now = Utc::now();
        let context = ExtractionContext {
            current_time: now.with_timezone(&self.timezone).fixed_offset(),
            timezone: self.timezone.name().to_owned(),
            default_event_duration_minutes: self.default_event_duration_minutes,
        };
        let extracted = self.gemini.extract_event(input, &context).await?;
        let heard = extracted
            .transcript
            .as_deref()
            .map(str::trim)
            .filter(|transcript| !transcript.is_empty())
            .map(|transcript| format!("Heard: \"{transcript}\"\n"))
            .unwrap_or_default();

        match validate(&extracted, now) {
            Validation::Ready(candidate) => self.create(message_id, &candidate).await,
            Validation::Confirm { candidate, reason } => {
                let mut message = format!(
                    "{heard}I understood this as: {}",
                    describe(&candidate, self.timezone)
                );
                if let Some(reason) = reason {
                    message.push_str(&format!("\n({reason})"));
                }
                Ok(CalendarOutcome::NeedsConfirmation {
                    request_id: message_id.to_owned(),
                    candidate,
                    message,
                })
            }
            Validation::Reject(reason) => Ok(CalendarOutcome::Error {
                message: format!("{heard}{reason}"),
            }),
        }
    }

    /// Creates the event. The ID derives from the source message, so retrying
    /// the same message can never create a second event.
    pub async fn create(
        &self,
        source_message_id: &str,
        candidate: &EventCandidate,
    ) -> Result<CalendarOutcome, AgentError> {
        let outcome = self
            .calendar
            .insert_event(
                &event_id_for(source_message_id),
                candidate,
                self.timezone.name(),
            )
            .await?;
        Ok(match outcome {
            InsertOutcome::Created(event) => CalendarOutcome::Created {
                message: format!(
                    "Added to Google Calendar: {}",
                    describe(candidate, self.timezone)
                ),
                event,
            },
            InsertOutcome::AlreadyExists => CalendarOutcome::AlreadyAdded {
                message: format!("Already in Google Calendar: {}", candidate.title),
            },
        })
    }
}

fn describe(candidate: &EventCandidate, timezone: Tz) -> String {
    let mut text = format!(
        "{}, {} to {}",
        candidate.title,
        format_time(candidate.start, timezone),
        format_time(candidate.end, timezone)
    );
    if let Some(rule) = &candidate.recurrence {
        text.push_str(&format!(" (repeats {rule})"));
    }
    text
}

pub(crate) fn format_time<T: TimeZone>(time: DateTime<T>, timezone: Tz) -> String {
    time.with_timezone(&timezone)
        .format("%-d %b, %-I:%M %p")
        .to_string()
}
