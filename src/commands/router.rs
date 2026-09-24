use chrono::{DateTime, FixedOffset, Utc};

use crate::incoming::{CommandContent, IncomingCommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Ignore,
    CalendarText(String),
    Confirmation { approved: bool },
    Task(TaskRoute),
    TaskError(String),
    Voice,
    UnsupportedMedia,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRoute {
    Create {
        title: String,
        notes: Option<String>,
        due_at: Option<DateTime<Utc>>,
        remind_at: Option<DateTime<Utc>>,
    },
    List,
    Complete {
        task_id: i64,
    },
    CreateReminder {
        body: String,
        due_at: DateTime<Utc>,
    },
}

pub fn route(command: &IncomingCommand, owner_jid: &str, _has_pending_confirmation: bool) -> Route {
    if !is_owner_self_chat(command, owner_jid) {
        return Route::Ignore;
    }

    match &command.content {
        CommandContent::Text(text) => {
            let normalized = text.trim();
            if normalized.eq_ignore_ascii_case("yes") {
                return Route::Confirmation { approved: true };
            }
            if normalized.eq_ignore_ascii_case("no") {
                return Route::Confirmation { approved: false };
            }
            if normalized.eq_ignore_ascii_case("tasks")
                || normalized.eq_ignore_ascii_case("todo list")
            {
                return Route::Task(TaskRoute::List);
            }
            if let Some(value) = normalized
                .strip_prefix("done ")
                .or_else(|| normalized.strip_prefix("todo done "))
            {
                return match value.trim().parse() {
                    Ok(task_id) if task_id > 0 => Route::Task(TaskRoute::Complete { task_id }),
                    _ => {
                        Route::TaskError("Use `done <task number>` from the tasks list.".to_owned())
                    }
                };
            }
            if let Some(value) = normalized
                .strip_prefix("todo ")
                .or_else(|| normalized.strip_prefix("task "))
            {
                return parse_task(value);
            }
            if let Some(value) = normalized.strip_prefix("remind ") {
                return parse_reminder(value);
            }
            Route::CalendarText(normalized.to_owned())
        }
        CommandContent::Voice { .. } => Route::Voice,
        CommandContent::UnsupportedMedia => Route::UnsupportedMedia,
    }
}

fn parse_task(value: &str) -> Route {
    let mut title = None;
    let mut notes = None;
    let mut due_at = None;
    let mut remind_at = None;

    for (index, part) in value.split('|').map(str::trim).enumerate() {
        if index == 0 {
            if part.is_empty() {
                return Route::TaskError("A task needs a title.".to_owned());
            }
            title = Some(part.to_owned());
            continue;
        }
        if let Some(note) = part.strip_prefix("note ") {
            if note.trim().is_empty() {
                return Route::TaskError("A task note cannot be empty.".to_owned());
            }
            notes = Some(note.trim().to_owned());
            continue;
        }
        if let Some(value) = part.strip_prefix("due ") {
            match parse_time(value) {
                Ok(time) => due_at = Some(time),
                Err(message) => return Route::TaskError(message),
            }
            continue;
        }
        if let Some(value) = part.strip_prefix("remind ") {
            match parse_time(value) {
                Ok(time) => remind_at = Some(time),
                Err(message) => return Route::TaskError(message),
            }
            continue;
        }
        return Route::TaskError("Use `note`, `due`, or `remind` after `|`.".to_owned());
    }

    let Some(title) = title else {
        return Route::TaskError("A task needs a title.".to_owned());
    };
    if let Some(remind_at) = remind_at {
        if remind_at <= Utc::now() {
            return Route::TaskError("The reminder time must be in the future.".to_owned());
        }
    }
    Route::Task(TaskRoute::Create {
        title,
        notes,
        due_at,
        remind_at,
    })
}

fn parse_reminder(value: &str) -> Route {
    let Some((time, body)) = value.split_once('|') else {
        return Route::TaskError("Use `remind <RFC3339 time> | <message>`.".to_owned());
    };
    let due_at = match parse_time(time) {
        Ok(time) if time > Utc::now() => time,
        Ok(_) => return Route::TaskError("The reminder time must be in the future.".to_owned()),
        Err(message) => return Route::TaskError(message),
    };
    let body = body.trim();
    if body.is_empty() {
        return Route::TaskError("A reminder needs a message.".to_owned());
    }
    Route::Task(TaskRoute::CreateReminder {
        body: body.to_owned(),
        due_at,
    })
}

fn parse_time(value: &str) -> Result<DateTime<Utc>, String> {
    DateTime::<FixedOffset>::parse_from_rfc3339(value.trim())
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| {
            "Use an ISO-8601 time with a timezone, for example 2026-09-18T16:00:00+05:30."
                .to_owned()
        })
}

pub fn is_owner_self_chat(command: &IncomingCommand, owner_jid: &str) -> bool {
    is_owner_jids(&command.chat_id, &command.sender_id, owner_jid)
}

pub fn is_owner_jids(chat_id: &str, sender_id: &str, owner_jid: &str) -> bool {
    same_account_jid(chat_id, owner_jid) && same_account_jid(sender_id, owner_jid)
}

fn same_account_jid(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }

    let Some((left_user, left_domain)) = left.split_once('@') else {
        return false;
    };
    let Some((right_user, right_domain)) = right.split_once('@') else {
        return false;
    };

    left_domain == right_domain
        && left_user
            .split_once(':')
            .map_or(left_user, |(user, _)| user)
            == right_user
                .split_once(':')
                .map_or(right_user, |(user, _)| user)
}
