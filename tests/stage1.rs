use chrono::{TimeZone, Utc};
use wa_calendar_agent::{
    commands::{route, Route, TaskRoute},
    db::{MessageStatus, NewTask, StateStore},
    incoming::{CommandContent, IncomingCommand},
    keep::{GoogleKeepNoteRequest, GoogleKeepWebhookClient, GoogleKeepWebhookResponse},
};
use wiremock::{
    matchers::{body_string_contains, header, method, path},
    Mock, MockServer, ResponseTemplate,
};

fn text_command(sender_id: &str, chat_id: &str, message_id: &str, text: &str) -> IncomingCommand {
    IncomingCommand {
        message_id: message_id.to_owned(),
        chat_id: chat_id.to_owned(),
        sender_id: sender_id.to_owned(),
        timestamp: Utc.with_ymd_and_hms(2026, 9, 11, 8, 0, 0).unwrap(),
        content: CommandContent::Text(text.to_owned()),
    }
}

#[test]
fn explicit_calendar_prefix_is_a_calendar_request() {
    let command = text_command("me", "me", "message-1", "cal meeting tomorrow 6pm");
    assert_eq!(
        route(&command, "me", false),
        Route::CalendarText("cal meeting tomorrow 6pm".to_owned())
    );
}

#[test]
fn owner_filter_ignores_other_chats_and_senders() {
    let other_sender = text_command("other", "me", "message-1", "cal meeting tomorrow 6pm");
    let other_chat = text_command("me", "group", "message-2", "cal meeting tomorrow 6pm");
    assert_eq!(route(&other_sender, "me", false), Route::Ignore);
    assert_eq!(route(&other_chat, "me", false), Route::Ignore);
}

#[test]
fn owner_filter_accepts_an_owner_linked_device_sender() {
    let command = text_command(
        "100000000000001:64@lid",
        "100000000000001@lid",
        "message-3",
        "cal meeting tomorrow 6pm",
    );
    assert_eq!(
        route(&command, "100000000000001@lid", false),
        Route::CalendarText("cal meeting tomorrow 6pm".to_owned())
    );
}

#[test]
fn yes_and_no_are_confirmation_decisions() {
    let yes = text_command("me", "me", "message-1", "yes");
    let no = text_command("me", "me", "message-2", "NO");
    assert_eq!(
        route(&yes, "me", true),
        Route::Confirmation { approved: true }
    );
    assert_eq!(
        route(&no, "me", true),
        Route::Confirmation { approved: false }
    );
}

#[test]
fn task_commands_parse_with_notes_due_dates_and_reminders() {
    let command = text_command(
        "me",
        "me",
        "task-1",
        "todo submit DBMS assignment | due 2099-09-18T17:00:00+05:30 | remind 2099-09-18T16:00:00+05:30 | note upload the PDF",
    );
    assert!(matches!(
        route(&command, "me", false),
        Route::Task(TaskRoute::Create { title, notes: Some(notes), due_at: Some(_), remind_at: Some(_) })
            if title == "submit DBMS assignment" && notes == "upload the PDF"
    ));

    let completion = text_command("me", "me", "task-2", "done 7");
    assert_eq!(
        route(&completion, "me", false),
        Route::Task(TaskRoute::Complete { task_id: 7 })
    );
}

#[tokio::test]
async fn duplicate_message_id_is_not_started_twice() {
    let store = StateStore::connect("sqlite::memory:").await.unwrap();
    assert!(store.begin_message("message-1").await.unwrap());
    store
        .set_message_status("message-1", MessageStatus::Submitted)
        .await
        .unwrap();
    assert!(!store.begin_message("message-1").await.unwrap());
}

#[tokio::test]
async fn tasks_and_reminders_are_durable_in_sqlite() {
    let store = StateStore::connect("sqlite::memory:").await.unwrap();
    let due_at = chrono::DateTime::parse_from_rfc3339("2099-09-18T17:00:00+05:30")
        .unwrap()
        .with_timezone(&Utc);
    let remind_at = chrono::DateTime::parse_from_rfc3339("2099-09-18T16:00:00+05:30")
        .unwrap()
        .with_timezone(&Utc);
    let task = store
        .create_task(NewTask {
            chat_id: "me".to_owned(),
            title: "Submit DBMS assignment".to_owned(),
            notes: Some("Upload PDF".to_owned()),
            due_at: Some(due_at),
            remind_at: Some(remind_at),
            source_message_id: "task-message-1".to_owned(),
        })
        .await
        .unwrap();
    let tasks = store.list_open_tasks("me").await.unwrap();
    assert_eq!(tasks, vec![task.clone()]);
    assert!(store.complete_task("me", task.id).await.unwrap());
    assert!(store.list_open_tasks("me").await.unwrap().is_empty());

    store
        .create_reminder(
            "me",
            "Call Abhinav",
            Utc::now() - chrono::Duration::minutes(1),
        )
        .await
        .unwrap();
    let reminders = store.claim_due_reminders(Utc::now()).await.unwrap();
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0].body, "Call Abhinav");
    store
        .mark_reminder_delivered(reminders[0].id)
        .await
        .unwrap();
    assert!(store
        .claim_due_reminders(Utc::now())
        .await
        .unwrap()
        .is_empty());

    let keep_notes = store.claim_google_keep_notes(Utc::now()).await.unwrap();
    assert_eq!(keep_notes.len(), 1);
    assert_eq!(keep_notes[0].task_id, task.id);
    assert!(keep_notes[0].body.contains("Upload PDF"));
    store
        .mark_google_keep_note_synced(task.id, "notes/keep-task-1")
        .await
        .unwrap();
    assert!(store
        .claim_google_keep_notes(Utc::now())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn keep_request_uses_typed_response_and_task_idempotency() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/keep"))
        .and(header("authorization", "Bearer secret"))
        .and(header("x-idempotency-key", "keep-task-7"))
        .and(body_string_contains("Submit DBMS assignment"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "created",
            "note": {"name": "notes/keep-task-7", "title": "Submit DBMS assignment"},
            "message": "Added to Google Keep"
        })))
        .mount(&server)
        .await;
    let keep = GoogleKeepWebhookClient::new(
        reqwest::Client::new(),
        url::Url::parse(&format!("{}/keep", server.uri())).unwrap(),
        "secret".to_owned(),
    );
    let response = keep
        .create_note(&GoogleKeepNoteRequest {
            task_id: 7,
            chat_id: "me".to_owned(),
            title: "Submit DBMS assignment".to_owned(),
            body: "Upload PDF".to_owned(),
        })
        .await
        .unwrap();
    assert!(matches!(
        response,
        GoogleKeepWebhookResponse::Created { note, .. } if note.name == "notes/keep-task-7"
    ));
}
