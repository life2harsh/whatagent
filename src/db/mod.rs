use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::AgentError;

#[derive(Clone)]
pub struct StateStore {
    connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    Received,
    Submitted,
    Completed,
    Failed,
}

impl MessageStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Received => "received",
            // Stored value predates the n8n removal; kept for existing databases.
            Self::Submitted => "sent_to_n8n",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingConfirmation {
    pub request_id: String,
    pub chat_id: String,
    pub original_message_id: String,
    pub candidate_json: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: i64,
    pub chat_id: String,
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub remind_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTask {
    pub chat_id: String,
    pub title: String,
    pub notes: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub remind_at: Option<DateTime<Utc>>,
    pub source_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueReminder {
    pub id: i64,
    pub chat_id: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingGoogleKeepNote {
    pub task_id: i64,
    pub chat_id: String,
    pub title: String,
    pub body: String,
}

impl StateStore {
    pub async fn connect(database_url: &str) -> Result<Self, AgentError> {
        let path = database_path(database_url)?;
        let connection = tokio::task::spawn_blocking(move || {
            let connection = Connection::open(path)?;
            connection.execute_batch(include_str!(
                "../../migrations/001_pending_confirmations.sql"
            ))?;
            connection
                .execute_batch(include_str!("../../migrations/002_tasks_and_reminders.sql"))?;
            connection.execute_batch(include_str!("../../migrations/003_google_keep_sync.sql"))?;
            Ok::<_, rusqlite::Error>(connection)
        })
        .await
        .map_err(join_error)??;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub async fn begin_message(&self, message_id: &str) -> Result<bool, AgentError> {
        let connection = Arc::clone(&self.connection);
        let message_id = message_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let now = Utc::now().to_rfc3339();
            let rows = connection.lock().map_err(lock_error)?.execute(
                "INSERT OR IGNORE INTO processed_messages (message_id, status, created_at, updated_at) VALUES (?, ?, ?, ?)",
                params![message_id, MessageStatus::Received.as_str(), now, now],
            )?;
            Ok(rows == 1)
        }).await.map_err(join_error)?
    }

    pub async fn set_message_status(
        &self,
        message_id: &str,
        status: MessageStatus,
    ) -> Result<(), AgentError> {
        let connection = Arc::clone(&self.connection);
        let message_id = message_id.to_owned();
        tokio::task::spawn_blocking(move || {
            connection.lock().map_err(lock_error)?.execute(
                "UPDATE processed_messages SET status = ?, updated_at = ? WHERE message_id = ?",
                params![status.as_str(), Utc::now().to_rfc3339(), message_id],
            )?;
            Ok(())
        })
        .await
        .map_err(join_error)?
    }

    pub async fn save_pending(&self, pending: &PendingConfirmation) -> Result<(), AgentError> {
        let connection = Arc::clone(&self.connection);
        let pending = pending.clone();
        tokio::task::spawn_blocking(move || {
            connection.lock().map_err(lock_error)?.execute(
                "INSERT OR REPLACE INTO pending_confirmations (request_id, chat_id, original_message_id, candidate_json, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)",
                params![pending.request_id, pending.chat_id, pending.original_message_id, pending.candidate_json, Utc::now().to_rfc3339(), pending.expires_at.to_rfc3339()],
            )?;
            Ok(())
        }).await.map_err(join_error)?
    }

    pub async fn take_pending_for_chat(
        &self,
        chat_id: &str,
    ) -> Result<Option<PendingConfirmation>, AgentError> {
        let connection = Arc::clone(&self.connection);
        let chat_id = chat_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let mut connection = connection.lock().map_err(lock_error)?;
            let transaction = connection.transaction()?;
            transaction.execute("DELETE FROM pending_confirmations WHERE expires_at <= ?", params![Utc::now().to_rfc3339()])?;
            let pending = transaction.query_row(
                "SELECT request_id, chat_id, original_message_id, candidate_json, expires_at FROM pending_confirmations WHERE chat_id = ? ORDER BY expires_at DESC LIMIT 1",
                params![chat_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, String>(4)?)),
            ).optional()?;
            if pending.is_some() {
                transaction.execute("DELETE FROM pending_confirmations WHERE chat_id = ?", params![chat_id])?;
            }
            transaction.commit()?;
            pending.map(to_pending_confirmation).transpose()
        }).await.map_err(join_error)?
    }

    pub async fn has_pending_for_chat(&self, chat_id: &str) -> Result<bool, AgentError> {
        let connection = Arc::clone(&self.connection);
        let chat_id = chat_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().map_err(lock_error)?;
            connection.execute(
                "DELETE FROM pending_confirmations WHERE expires_at <= ?",
                params![Utc::now().to_rfc3339()],
            )?;
            let count: i64 = connection.query_row(
                "SELECT COUNT(*) FROM pending_confirmations WHERE chat_id = ?",
                params![chat_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
        .await
        .map_err(join_error)?
    }

    pub async fn create_task(&self, task: NewTask) -> Result<Task, AgentError> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let now = Utc::now().to_rfc3339();
            let mut connection = connection.lock().map_err(lock_error)?;
            let transaction = connection.transaction()?;
            transaction.execute(
                "INSERT INTO tasks (chat_id, title, notes, due_at, remind_at, status, source_message_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'open', ?, ?, ?)",
                params![task.chat_id, task.title, task.notes, task.due_at.map(|value| value.to_rfc3339()), task.remind_at.map(|value| value.to_rfc3339()), task.source_message_id, now, now],
            )?;
            let id = transaction.last_insert_rowid();
            if let Some(remind_at) = task.remind_at {
                transaction.execute(
                    "INSERT INTO reminder_outbox (task_id, chat_id, body, due_at, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'pending', ?, ?)",
                    params![id, task.chat_id, format!("Reminder: {}", task.title), remind_at.to_rfc3339(), now, now],
                )?;
            }
            transaction.execute(
                "INSERT INTO google_keep_outbox (task_id, chat_id, title, body, status, created_at, updated_at) VALUES (?, ?, ?, ?, 'pending', ?, ?)",
                params![id, task.chat_id, task.title, keep_note_body(&task.notes, task.due_at, task.remind_at), now, now],
            )?;
            transaction.commit()?;
            Ok(Task {
                id,
                chat_id: task.chat_id,
                title: task.title,
                notes: task.notes,
                due_at: task.due_at,
                remind_at: task.remind_at,
            })
        })
        .await
        .map_err(join_error)?
    }

    pub async fn list_open_tasks(&self, chat_id: &str) -> Result<Vec<Task>, AgentError> {
        let connection = Arc::clone(&self.connection);
        let chat_id = chat_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let connection = connection.lock().map_err(lock_error)?;
            let mut statement = connection.prepare(
                "SELECT id, chat_id, title, notes, due_at, remind_at FROM tasks WHERE chat_id = ? AND status = 'open' ORDER BY due_at IS NULL, due_at, id",
            )?;
            let tasks = statement
                .query_map(params![chat_id], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                })?
                .map(|row| row.map_err(AgentError::from).and_then(to_task))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(tasks)
        })
        .await
        .map_err(join_error)?
    }

    pub async fn complete_task(&self, chat_id: &str, task_id: i64) -> Result<bool, AgentError> {
        let connection = Arc::clone(&self.connection);
        let chat_id = chat_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let now = Utc::now().to_rfc3339();
            let mut connection = connection.lock().map_err(lock_error)?;
            let transaction = connection.transaction()?;
            let changed = transaction.execute(
                "UPDATE tasks SET status = 'completed', completed_at = ?, updated_at = ? WHERE id = ? AND chat_id = ? AND status = 'open'",
                params![now, now, task_id, chat_id],
            )?;
            if changed == 1 {
                transaction.execute(
                    "UPDATE reminder_outbox SET status = 'failed', updated_at = ? WHERE task_id = ? AND status = 'pending'",
                    params![Utc::now().to_rfc3339(), task_id],
                )?;
            }
            transaction.commit()?;
            Ok(changed == 1)
        })
        .await
        .map_err(join_error)?
    }

    pub async fn create_reminder(
        &self,
        chat_id: &str,
        body: &str,
        due_at: DateTime<Utc>,
    ) -> Result<(), AgentError> {
        let connection = Arc::clone(&self.connection);
        let chat_id = chat_id.to_owned();
        let body = body.to_owned();
        tokio::task::spawn_blocking(move || {
            let now = Utc::now().to_rfc3339();
            connection.lock().map_err(lock_error)?.execute(
                "INSERT INTO reminder_outbox (chat_id, body, due_at, status, created_at, updated_at) VALUES (?, ?, ?, 'pending', ?, ?)",
                params![chat_id, body, due_at.to_rfc3339(), now, now],
            )?;
            Ok(())
        })
        .await
        .map_err(join_error)?
    }

    pub async fn claim_due_reminders(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<DueReminder>, AgentError> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let mut connection = connection.lock().map_err(lock_error)?;
            let transaction = connection.transaction()?;
            let claimed_at = now.to_rfc3339();
            transaction.execute(
                "UPDATE reminder_outbox SET status = 'pending', claimed_at = NULL, updated_at = ? WHERE status = 'sending' AND claimed_at <= ?",
                params![claimed_at, (now - chrono::Duration::minutes(5)).to_rfc3339()],
            )?;
            let reminders = transaction
                .prepare("SELECT id, chat_id, body FROM reminder_outbox WHERE status = 'pending' AND due_at <= ? ORDER BY due_at, id")?
                .query_map(params![claimed_at], |row| {
                    Ok(DueReminder {
                        id: row.get(0)?,
                        chat_id: row.get(1)?,
                        body: row.get(2)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for reminder in &reminders {
                transaction.execute(
                    "UPDATE reminder_outbox SET status = 'sending', attempts = attempts + 1, claimed_at = ?, updated_at = ? WHERE id = ?",
                    params![claimed_at, claimed_at, reminder.id],
                )?;
            }
            transaction.commit()?;
            Ok(reminders)
        })
        .await
        .map_err(join_error)?
    }

    pub async fn mark_reminder_delivered(&self, reminder_id: i64) -> Result<(), AgentError> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let now = Utc::now().to_rfc3339();
            connection.lock().map_err(lock_error)?.execute(
                "UPDATE reminder_outbox SET status = 'delivered', delivered_at = ?, updated_at = ? WHERE id = ? AND status = 'sending'",
                params![now, now, reminder_id],
            )?;
            Ok(())
        })
        .await
        .map_err(join_error)?
    }

    pub async fn claim_google_keep_notes(
        &self,
        now: DateTime<Utc>,
    ) -> Result<Vec<PendingGoogleKeepNote>, AgentError> {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            let mut connection = connection.lock().map_err(lock_error)?;
            let transaction = connection.transaction()?;
            let claimed_at = now.to_rfc3339();
            transaction.execute(
                "UPDATE google_keep_outbox SET status = 'pending', claimed_at = NULL, updated_at = ? WHERE status = 'sending' AND claimed_at <= ?",
                params![claimed_at, (now - chrono::Duration::minutes(5)).to_rfc3339()],
            )?;
            let notes = transaction
                .prepare("SELECT task_id, chat_id, title, body FROM google_keep_outbox WHERE status = 'pending' ORDER BY task_id")?
                .query_map([], |row| {
                    Ok(PendingGoogleKeepNote {
                        task_id: row.get(0)?,
                        chat_id: row.get(1)?,
                        title: row.get(2)?,
                        body: row.get(3)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for note in &notes {
                transaction.execute(
                    "UPDATE google_keep_outbox SET status = 'sending', attempts = attempts + 1, claimed_at = ?, updated_at = ? WHERE task_id = ?",
                    params![claimed_at, claimed_at, note.task_id],
                )?;
            }
            transaction.commit()?;
            Ok(notes)
        })
        .await
        .map_err(join_error)?
    }

    pub async fn mark_google_keep_note_synced(
        &self,
        task_id: i64,
        keep_note_name: &str,
    ) -> Result<(), AgentError> {
        let connection = Arc::clone(&self.connection);
        let keep_note_name = keep_note_name.to_owned();
        tokio::task::spawn_blocking(move || {
            let now = Utc::now().to_rfc3339();
            connection.lock().map_err(lock_error)?.execute(
                "UPDATE google_keep_outbox SET status = 'synced', keep_note_name = ?, updated_at = ? WHERE task_id = ? AND status = 'sending'",
                params![keep_note_name, now, task_id],
            )?;
            Ok(())
        })
        .await
        .map_err(join_error)?
    }
}

fn database_path(database_url: &str) -> Result<String, AgentError> {
    if database_url == "sqlite::memory:" {
        return Ok(":memory:".to_owned());
    }
    database_url
        .strip_prefix("sqlite://")
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            AgentError::Config("DATABASE_URL must use sqlite:// or sqlite::memory:".to_owned())
        })
}

fn to_pending_confirmation(
    row: (String, String, String, String, String),
) -> Result<PendingConfirmation, AgentError> {
    let (request_id, chat_id, original_message_id, candidate_json, expires_at) = row;
    let expires_at = DateTime::parse_from_rfc3339(&expires_at)
        .map_err(|error| AgentError::Config(format!("invalid stored expiry: {error}")))?
        .with_timezone(&Utc);
    Ok(PendingConfirmation {
        request_id,
        chat_id,
        original_message_id,
        candidate_json,
        expires_at,
    })
}

fn to_task(
    row: (
        i64,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ),
) -> Result<Task, AgentError> {
    let (id, chat_id, title, notes, due_at, remind_at) = row;
    Ok(Task {
        id,
        chat_id,
        title,
        notes,
        due_at: due_at.map(parse_stored_time).transpose()?,
        remind_at: remind_at.map(parse_stored_time).transpose()?,
    })
}

fn parse_stored_time(value: String) -> Result<DateTime<Utc>, AgentError> {
    DateTime::parse_from_rfc3339(&value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|error| AgentError::Config(format!("invalid stored timestamp: {error}")))
}

fn keep_note_body(
    notes: &Option<String>,
    due_at: Option<DateTime<Utc>>,
    remind_at: Option<DateTime<Utc>>,
) -> String {
    let mut lines = Vec::new();
    if let Some(notes) = notes {
        lines.push(notes.clone());
    }
    if let Some(due_at) = due_at {
        lines.push(format!("Due: {}", due_at.to_rfc3339()));
    }
    if let Some(remind_at) = remind_at {
        lines.push(format!("Reminder: {}", remind_at.to_rfc3339()));
    }
    if lines.is_empty() {
        "Created from WhatsApp assistant.".to_owned()
    } else {
        lines.join("\n")
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> AgentError {
    AgentError::Config("SQLite state lock is poisoned".to_owned())
}

fn join_error(error: tokio::task::JoinError) -> AgentError {
    AgentError::Config(format!("SQLite state task failed: {error}"))
}
