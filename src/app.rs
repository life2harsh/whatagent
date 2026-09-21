use std::time::Duration;

use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use crate::{
    calendar::{
        CalendarWebhookClient, CalendarWebhookRequest, CalendarWebhookResponse, ConfirmationRequest,
    },
    commands::{is_owner_jids, is_owner_self_chat, route, Route, TaskRoute},
    config::Config,
    db::{MessageStatus, NewTask, PendingConfirmation, StateStore, Task},
    error::AgentError,
    incoming::IncomingCommand,
    keep::{GoogleKeepNoteRequest, GoogleKeepWebhookClient, GoogleKeepWebhookResponse},
};

pub struct Agent {
    config: Config,
    calendar: CalendarWebhookClient,
    keep: Option<GoogleKeepWebhookClient>,
    store: StateStore,
}

pub enum ProcessOutcome {
    Ignore,
    Reply(String),
}

impl Agent {
    pub fn new(
        config: Config,
        calendar: CalendarWebhookClient,
        keep: Option<GoogleKeepWebhookClient>,
        store: StateStore,
    ) -> Self {
        Self {
            config,
            calendar,
            keep,
            store,
        }
    }

    pub async fn process(&self, command: IncomingCommand) -> Result<ProcessOutcome, AgentError> {
        if !is_owner_self_chat(&command, &self.config.owner_jid) {
            debug!(message_id = %command.message_id, "ignored non-owner or non-self-chat message");
            return Ok(ProcessOutcome::Ignore);
        }
        if !self.store.begin_message(&command.message_id).await? {
            debug!(message_id = %command.message_id, "ignored duplicate message");
            return Ok(ProcessOutcome::Ignore);
        }

        let has_pending = self.store.has_pending_for_chat(&command.chat_id).await?;
        match route(&command, &self.config.owner_jid, has_pending) {
            Route::Ignore => Ok(ProcessOutcome::Ignore),
            Route::UnsupportedMedia => {
                self.store
                    .set_message_status(&command.message_id, MessageStatus::Completed)
                    .await?;
                Ok(ProcessOutcome::Ignore)
            }
            Route::Voice => self.process_voice(command).await,
            Route::Confirmation { approved } => self.process_confirmation(&command, approved).await,
            Route::Task(route) => self.process_task(&command, route).await,
            Route::TaskError(message) => {
                self.store
                    .set_message_status(&command.message_id, MessageStatus::Completed)
                    .await?;
                Ok(ProcessOutcome::Reply(message))
            }
            Route::CalendarText(text) => self.process_text(&command, text).await,
        }
    }

    pub fn owns_self_chat(&self, chat_id: &str, sender_id: &str) -> bool {
        is_owner_jids(chat_id, sender_id, &self.config.owner_jid)
    }

    pub fn reminder_poll_interval(&self) -> Duration {
        self.config.reminder_poll_interval
    }

    pub fn state_store(&self) -> StateStore {
        self.store.clone()
    }

    pub fn google_keep_sync_enabled(&self) -> bool {
        self.keep.is_some()
    }

    pub async fn sync_google_keep_notes(&self) -> Result<usize, AgentError> {
        let Some(keep) = &self.keep else {
            return Ok(0);
        };
        let notes = self.store.claim_google_keep_notes(Utc::now()).await?;
        let mut synced = 0;
        for note in notes {
            let request = GoogleKeepNoteRequest {
                task_id: note.task_id,
                chat_id: note.chat_id,
                title: note.title,
                body: note.body,
            };
            match keep.create_note(&request).await {
                Ok(GoogleKeepWebhookResponse::Created { note, .. }) => {
                    self.store
                        .mark_google_keep_note_synced(request.task_id, &note.name)
                        .await?;
                    synced += 1;
                }
                Ok(GoogleKeepWebhookResponse::Error { message }) => {
                    warn!(task_id = request.task_id, %message, "Google Keep note creation failed");
                }
                Err(error) => {
                    warn!(task_id = request.task_id, %error, "Google Keep webhook failed");
                }
            }
        }
        Ok(synced)
    }

    async fn process_text(
        &self,
        command: &IncomingCommand,
        text: String,
    ) -> Result<ProcessOutcome, AgentError> {
        let request = self.text_request(command, text);
        self.store
            .set_message_status(&command.message_id, MessageStatus::SentToN8n)
            .await?;
        match self.calendar.submit(request).await {
            Ok(response) => self.finish_response(command, response).await,
            Err(error) => {
                self.store
                    .set_message_status(&command.message_id, MessageStatus::Failed)
                    .await?;
                warn!(message_id = %command.message_id, error = %error, "calendar webhook failed");
                Ok(ProcessOutcome::Reply(
                    "Couldn't add that to Calendar right now. Please try again.".to_owned(),
                ))
            }
        }
    }

    async fn process_voice(
        &self,
        mut command: IncomingCommand,
    ) -> Result<ProcessOutcome, AgentError> {
        let content = std::mem::replace(
            &mut command.content,
            crate::incoming::CommandContent::UnsupportedMedia,
        );
        let crate::incoming::CommandContent::Voice { mime_type, data } = content else {
            return Ok(ProcessOutcome::Ignore);
        };
        let request = self.audio_request(&command, mime_type, data);
        self.store
            .set_message_status(&command.message_id, MessageStatus::SentToN8n)
            .await?;
        match self.calendar.submit(request).await {
            Ok(response) => self.finish_response(&command, response).await,
            Err(error) => {
                self.store
                    .set_message_status(&command.message_id, MessageStatus::Failed)
                    .await?;
                warn!(message_id = %command.message_id, error = %error, "voice calendar webhook failed");
                Ok(ProcessOutcome::Reply(
                    "Couldn't add that voice note to Calendar right now. Please try again."
                        .to_owned(),
                ))
            }
        }
    }

    async fn process_confirmation(
        &self,
        command: &IncomingCommand,
        approved: bool,
    ) -> Result<ProcessOutcome, AgentError> {
        let Some(pending) = self.store.take_pending_for_chat(&command.chat_id).await? else {
            self.store
                .set_message_status(&command.message_id, MessageStatus::Completed)
                .await?;
            return Ok(ProcessOutcome::Reply(
                "There is no pending calendar request to confirm.".to_owned(),
            ));
        };
        self.store
            .set_message_status(&command.message_id, MessageStatus::SentToN8n)
            .await?;
        let request = ConfirmationRequest {
            request_id: pending.request_id,
            approved,
        };
        match self.calendar.confirm(&request).await {
            Ok(response) => self.finish_response(command, response).await,
            Err(error) => {
                self.store
                    .set_message_status(&command.message_id, MessageStatus::Failed)
                    .await?;
                warn!(message_id = %command.message_id, error = %error, "confirmation webhook failed");
                Ok(ProcessOutcome::Reply(
                    "Couldn't update that calendar request. Please try again.".to_owned(),
                ))
            }
        }
    }

    async fn process_task(
        &self,
        command: &IncomingCommand,
        route: TaskRoute,
    ) -> Result<ProcessOutcome, AgentError> {
        let reply = match route {
            TaskRoute::Create {
                title,
                notes,
                due_at,
                remind_at,
            } => {
                let task = self
                    .store
                    .create_task(NewTask {
                        chat_id: command.chat_id.clone(),
                        title,
                        notes,
                        due_at,
                        remind_at,
                        source_message_id: command.message_id.clone(),
                    })
                    .await?;
                let synced = self.sync_google_keep_notes().await?;
                format_task_created(&task, self.config.timezone, self.keep.is_some(), synced > 0)
            }
            TaskRoute::List => {
                let tasks = self.store.list_open_tasks(&command.chat_id).await?;
                format_task_list(&tasks, self.config.timezone)
            }
            TaskRoute::Complete { task_id } => {
                if self.store.complete_task(&command.chat_id, task_id).await? {
                    format!("Completed task {task_id}.")
                } else {
                    format!("I couldn't find an open task {task_id}.")
                }
            }
            TaskRoute::CreateReminder { body, due_at } => {
                self.store
                    .create_reminder(&command.chat_id, &body, due_at)
                    .await?;
                format!(
                    "Reminder set for {}.",
                    format_time(due_at, self.config.timezone)
                )
            }
        };
        self.store
            .set_message_status(&command.message_id, MessageStatus::Completed)
            .await?;
        Ok(ProcessOutcome::Reply(reply))
    }

    async fn finish_response(
        &self,
        command: &IncomingCommand,
        response: CalendarWebhookResponse,
    ) -> Result<ProcessOutcome, AgentError> {
        let reply = match response {
            CalendarWebhookResponse::Created { message, .. } => message,
            CalendarWebhookResponse::Error { message } => {
                format!("Couldn't add that to Calendar: {message}")
            }
            CalendarWebhookResponse::NeedsConfirmation {
                request_id,
                candidate,
                message,
            } => {
                let pending = PendingConfirmation {
                    request_id,
                    chat_id: command.chat_id.clone(),
                    original_message_id: command.message_id.clone(),
                    candidate_json: serde_json::to_string(&candidate)?,
                    expires_at: Utc::now()
                        + chrono::Duration::from_std(self.config.confirmation_ttl)
                            .map_err(|error| AgentError::Config(error.to_string()))?,
                };
                self.store.save_pending(&pending).await?;
                format!("{message}\n\nReply \"yes\" to add it or \"no\" to cancel.")
            }
        };
        self.store
            .set_message_status(&command.message_id, MessageStatus::Completed)
            .await?;
        info!(message_id = %command.message_id, "calendar command completed");
        Ok(ProcessOutcome::Reply(reply))
    }

    pub fn text_request(&self, command: &IncomingCommand, text: String) -> CalendarWebhookRequest {
        let timestamp = in_timezone(command.timestamp, self.config.timezone);
        let current_time = in_timezone(Utc::now(), self.config.timezone);
        CalendarWebhookRequest::Text {
            message_id: command.message_id.clone(),
            chat_id: command.chat_id.clone(),
            timestamp,
            timezone: self.config.timezone.name().to_owned(),
            current_time,
            default_event_duration_minutes: self.config.default_event_duration_minutes,
            text,
        }
    }

    fn audio_request(
        &self,
        command: &IncomingCommand,
        mime_type: Option<String>,
        data: Vec<u8>,
    ) -> CalendarWebhookRequest {
        let timestamp = in_timezone(command.timestamp, self.config.timezone);
        let current_time = in_timezone(Utc::now(), self.config.timezone);
        CalendarWebhookRequest::Audio {
            message_id: command.message_id.clone(),
            chat_id: command.chat_id.clone(),
            timestamp,
            timezone: self.config.timezone.name().to_owned(),
            current_time,
            default_event_duration_minutes: self.config.default_event_duration_minutes,
            mime_type,
            data,
        }
    }
}

fn in_timezone(time: DateTime<Utc>, timezone: chrono_tz::Tz) -> DateTime<chrono::FixedOffset> {
    time.with_timezone(&timezone).fixed_offset()
}

fn format_task_created(
    task: &Task,
    timezone: chrono_tz::Tz,
    keep_enabled: bool,
    keep_synced: bool,
) -> String {
    let mut message = format!("Added task {}: {}", task.id, task.title);
    if let Some(due_at) = task.due_at {
        message.push_str(&format!("\nDue: {}", format_time(due_at, timezone)));
    }
    if let Some(remind_at) = task.remind_at {
        message.push_str(&format!("\nReminder: {}", format_time(remind_at, timezone)));
    }
    if keep_enabled {
        message.push_str(if keep_synced {
            "\nSynced to Google Keep."
        } else {
            "\nGoogle Keep sync pending."
        });
    }
    message
}

fn format_task_list(tasks: &[Task], timezone: chrono_tz::Tz) -> String {
    if tasks.is_empty() {
        return "No open tasks.".to_owned();
    }
    let mut lines = vec!["Open tasks:".to_owned()];
    for task in tasks {
        let due = task
            .due_at
            .map(|time| format!(" — due {}", format_time(time, timezone)))
            .unwrap_or_default();
        lines.push(format!("{}. {}{}", task.id, task.title, due));
    }
    lines.join("\n")
}

fn format_time(time: DateTime<Utc>, timezone: chrono_tz::Tz) -> String {
    time.with_timezone(&timezone)
        .format("%-d %b, %-I:%M %p")
        .to_string()
}
