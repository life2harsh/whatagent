use chrono::{DateTime, Utc};

use crate::{calendar::EventCandidate, google::gemini::ExtractedEvent};

const MIN_CONFIDENCE: f64 = 0.85;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Validation {
    Ready(EventCandidate),
    Confirm {
        candidate: EventCandidate,
        reason: Option<String>,
    },
    Reject(String),
}

pub fn validate(extracted: &ExtractedEvent, now: DateTime<Utc>) -> Validation {
    if extracted.intent != "create_calendar_event" {
        return Validation::Reject("That doesn't look like an event or a todo.".to_owned());
    }
    let reason = non_empty(extracted.ambiguity_reason.as_deref());
    let Some(title) = non_empty(extracted.title.as_deref()) else {
        return Validation::Reject("I could not determine the event title.".to_owned());
    };
    let (Some(start), Some(end)) = (
        parse_time(extracted.start.as_deref()),
        parse_time(extracted.end.as_deref()),
    ) else {
        return Validation::Reject(match reason {
            Some(reason) => format!("I could not determine the event time: {reason}"),
            None => "I could not determine the event time.".to_owned(),
        });
    };
    if end <= start {
        return Validation::Reject("I could not determine a valid event time range.".to_owned());
    }
    if start < now - chrono::Duration::hours(24) {
        return Validation::Reject("The event time is too far in the past.".to_owned());
    }
    let recurrence = match non_empty(extracted.recurrence.as_deref()) {
        None => None,
        Some(rule) => match normalize_recurrence(&rule) {
            Some(rule) => Some(rule),
            None => {
                return Validation::Reject(
                    "I could not determine a valid recurrence pattern.".to_owned(),
                )
            }
        },
    };

    let candidate = EventCandidate {
        title,
        start,
        end,
        description: non_empty(extracted.description.as_deref()),
        location: non_empty(extracted.location.as_deref()),
        recurrence,
    };
    if extracted.needs_confirmation || extracted.confidence < MIN_CONFIDENCE {
        Validation::Confirm { candidate, reason }
    } else {
        Validation::Ready(candidate)
    }
}

pub fn normalize_recurrence(value: &str) -> Option<String> {
    let rule = value.trim().to_ascii_uppercase();
    let rule = rule.strip_prefix("RRULE:").unwrap_or(&rule).to_owned();
    let mut parts = rule.split(';');
    let freq = parts.next()?;
    if !matches!(
        freq,
        "FREQ=DAILY" | "FREQ=WEEKLY" | "FREQ=MONTHLY" | "FREQ=YEARLY"
    ) {
        return None;
    }
    for part in parts {
        let (key, value) = part.split_once('=')?;
        if value.is_empty() {
            return None;
        }
        let valid = match key {
            "INTERVAL" | "COUNT" => value.parse::<u32>().is_ok_and(|number| number >= 1),
            "UNTIL" => is_until(value),
            "BYDAY" => value
                .split(',')
                .all(|day| matches!(day, "MO" | "TU" | "WE" | "TH" | "FR" | "SA" | "SU")),
            "BYMONTHDAY" => value.split(',').all(|day| {
                day.parse::<i32>()
                    .is_ok_and(|day| day != 0 && (-31..=31).contains(&day))
            }),
            "BYMONTH" => value.split(',').all(|month| {
                month
                    .parse::<u32>()
                    .is_ok_and(|month| (1..=12).contains(&month))
            }),
            _ => false,
        };
        if !valid {
            return None;
        }
    }
    Some(rule)
}

fn is_until(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 16
        && bytes[8] == b'T'
        && bytes[15] == b'Z'
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[9..15].iter().all(u8::is_ascii_digit)
}

fn parse_time(value: Option<&str>) -> Option<DateTime<chrono::FixedOffset>> {
    DateTime::parse_from_rfc3339(value?.trim()).ok()
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
