use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;

use crate::{
    calendar::{
        validate::{validate, Validation},
        CalendarOutcome, EventCandidate,
    },
    error::AgentError,
    google::{
        calendar::{event_id_for, GoogleCalendarClient, InsertOutcome},
        gemini::{ExtractedEvent, ExtractionContext, ExtractionInput, GeminiClient},
    },
};

/// How long before an event its WhatsApp reminder goes out when there is no
/// Google Calendar (the same default Google Calendar uses).
const EVENT_REMINDER_MINUTES: i64 = 30;

/// Turns a text or voice request into a Google Calendar event, or into a task
/// with a reminder when Google Calendar isn't set up.
pub struct CalendarService {
    gemini: GeminiClient,
    calendar: Option<GoogleCalendarClient>,
    timezone: Tz,
    default_event_duration_minutes: u32,
}

impl CalendarService {
    pub fn new(
        gemini: GeminiClient,
        calendar: Option<GoogleCalendarClient>,
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

        if extracted.intent == "create_task" {
            return Ok(build_task(&extracted, heard, now));
        }

        match validate(&extracted, now) {
            Validation::Ready(candidate) if self.calendar.is_none() => {
                Ok(event_as_task(&candidate, heard, now))
            }
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
    /// the same message can never create a second event. Without Google
    /// Calendar it returns a task with a reminder instead.
    pub async fn create(
        &self,
        source_message_id: &str,
        candidate: &EventCandidate,
    ) -> Result<CalendarOutcome, AgentError> {
        let Some(calendar) = &self.calendar else {
            return Ok(event_as_task(candidate, String::new(), Utc::now()));
        };
        let outcome = calendar
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

/// Turns a create_task extraction into a task outcome for the agent to store.
/// A reminder time in the past is dropped rather than firing immediately.
fn build_task(extracted: &ExtractedEvent, heard: String, now: DateTime<Utc>) -> CalendarOutcome {
    let Some(title) = non_empty(extracted.title.as_deref()) else {
        return CalendarOutcome::Error {
            message: format!("{heard}I couldn't tell what the task should be. Try again."),
        };
    };
    CalendarOutcome::Task {
        title,
        notes: non_empty(extracted.description.as_deref()),
        due_at: parse_utc(extracted.due.as_deref()),
        remind_at: parse_utc(extracted.remind.as_deref()).filter(|time| *time > now),
        heard,
        note: None,
    }
}

/// Without Google Calendar, an event becomes a task due when it starts, with a
/// WhatsApp reminder shortly before (or at the start, if that is too soon).
fn event_as_task(candidate: &EventCandidate, heard: String, now: DateTime<Utc>) -> CalendarOutcome {
    let start = candidate.start.to_utc();
    let remind_at = [start - Duration::minutes(EVENT_REMINDER_MINUTES), start]
        .into_iter()
        .find(|time| *time > now);
    let location = non_empty(candidate.location.as_deref()).map(|place| format!("At {place}"));
    let notes = [non_empty(candidate.description.as_deref()), location]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    CalendarOutcome::Task {
        title: candidate.title.clone(),
        notes: (!notes.is_empty()).then(|| notes.join("\n")),
        due_at: Some(start),
        remind_at,
        heard,
        note: candidate.recurrence.as_ref().map(|_| {
            "Repeating reminders need Google Calendar, so only the first one is set.".to_owned()
        }),
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_utc(value: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value?.trim())
        .ok()
        .map(|time| time.with_timezone(&Utc))
}
