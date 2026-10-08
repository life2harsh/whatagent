use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use whatsapp_rust::{
    bot::Bot,
    prelude::{Client, Jid, SqliteStore},
};

use crate::{
    app::{Agent, ProcessOutcome},
    db::StateStore,
    error::AgentError,
    whatsapp::normalize::normalize,
};

pub async fn run(agent: Arc<Agent>, session_database_path: String) -> Result<(), AgentError> {
    let sent_message_ids = Arc::new(Mutex::new(HashSet::new()));
    let reminder_dispatcher_started = Arc::new(AtomicBool::new(false));
    let store = SqliteStore::new(&session_database_path)
        .await
        .map_err(|error| AgentError::Config(format!("WhatsApp session store failed: {error}")))?;
    let handler_agent = Arc::clone(&agent);
    let handler_sent_message_ids = Arc::clone(&sent_message_ids);
    let connected_agent = Arc::clone(&agent);
    let connected_sent_message_ids = Arc::clone(&sent_message_ids);
    let connected_reminder_dispatcher_started = Arc::clone(&reminder_dispatcher_started);

    let bot = Bot::builder()
        .with_backend(store)
        .on_qr_code(|code, timeout| async move {
            info!(
                expires_in_seconds = timeout.as_secs(),
                "WhatsApp pairing QR code received"
            );
            print_qr_code(&code);
        })
        .on_connected(move |client| {
            let agent = Arc::clone(&connected_agent);
            let sent_message_ids = Arc::clone(&connected_sent_message_ids);
            let reminder_dispatcher_started = Arc::clone(&connected_reminder_dispatcher_started);
            async move {
            let pn_jid = client.pn().map(|jid| jid.to_string());
            let lid_jid = client.lid().map(|jid| jid.to_string());
            info!(
                ?pn_jid,
                ?lid_jid,
                "WhatsApp connected; set OWNER_JID to the self-chat JID observed for your account"
            );
            if !reminder_dispatcher_started.swap(true, Ordering::SeqCst) {
                tokio::spawn(dispatch_reminders(
                    client,
                    agent.state_store(),
                    sent_message_ids,
                    agent.reminder_poll_interval(),
                ));
            }
            }
        })
        .on_logged_out(|_| async move {
            warn!("WhatsApp logged out; re-pairing is required");
        })
        .on_message(move |context| {
            let agent = Arc::clone(&handler_agent);
            let sent_message_ids = Arc::clone(&handler_sent_message_ids);
            async move {
                if context.info.is_offline {
                    return;
                }
                let chat_id = context.info.source.chat.to_string();
                let sender_id = context.info.source.sender.to_string();
                if !agent.owns_self_chat(&chat_id, &sender_id) {
                    debug!(message_id = %context.info.id, "ignored non-owner or non-self-chat WhatsApp message");
                    return;
                }
                let command = match normalize(&context).await {
                    Ok(command) => command,
                    Err(error) => {
                        warn!(message_id = %context.info.id, error = %error, "WhatsApp voice-note download failed");
                        if let Ok(result) = context.reply(error.user_message()).await {
                            sent_message_ids.lock().await.insert(result.message_id);
                        }
                        return;
                    }
                };
                info!(message_id = %command.message_id, "received supported self-chat command");
                if sent_message_ids.lock().await.remove(&command.message_id) {
                    return;
                }
                match agent.process(command).await {
                    Ok(ProcessOutcome::Ignore) => {}
                    Ok(ProcessOutcome::Reply(message)) => match context.reply(message).await {
                        Ok(result) => {
                            sent_message_ids.lock().await.insert(result.message_id);
                        }
                        Err(error) => error!(error = %error, "WhatsApp reply failed"),
                    },
                    Err(error) => error!(error = %error, "WhatsApp command processing failed"),
                }
            }
        })
        .build()
        .await
        .map_err(|error| AgentError::Config(format!("WhatsApp client setup failed: {error}")))?;

    bot.run().await;
    Ok(())
}

async fn dispatch_reminders(
    client: Arc<Client>,
    store: StateStore,
    sent_message_ids: Arc<Mutex<HashSet<String>>>,
    poll_interval: std::time::Duration,
) {
    loop {
        match store.claim_due_reminders(chrono::Utc::now()).await {
            Ok(reminders) => {
                for reminder in reminders {
                    let jid = match reminder.chat_id.parse::<Jid>() {
                        Ok(jid) => jid,
                        Err(error) => {
                            warn!(reminder_id = reminder.id, error = %error, "reminder has an invalid chat JID");
                            continue;
                        }
                    };
                    match client.send_text(jid, reminder.body).await {
                        Ok(result) => {
                            sent_message_ids.lock().await.insert(result.message_id);
                            if let Err(error) = store.mark_reminder_delivered(reminder.id).await {
                                error!(reminder_id = reminder.id, error = %error, "reminder delivery could not be recorded");
                            } else {
                                info!(reminder_id = reminder.id, "WhatsApp reminder delivered");
                            }
                        }
                        Err(error) => {
                            warn!(reminder_id = reminder.id, error = %error, "WhatsApp reminder delivery failed")
                        }
                    }
                }
            }
            Err(error) => warn!(error = %error, "due reminder lookup failed"),
        }
        sleep(poll_interval).await;
    }
}

fn print_qr_code(payload: &str) {
    match qrcode::QrCode::new(payload.as_bytes()) {
        Ok(code) => {
            let image = code.render::<char>().quiet_zone(true).build();
            println!("Scan this QR code in WhatsApp > Linked devices > Link a device:\n{image}");
        }
        Err(error) => error!(error = %error, "WhatsApp pairing QR could not be rendered"),
    }
}

pub async fn pair(session_database_path: String) -> Result<(), AgentError> {
    let store = SqliteStore::new(&session_database_path)
        .await
        .map_err(|error| AgentError::Config(format!("WhatsApp session store failed: {error}")))?;
    let bot = Bot::builder()
        .with_backend(store)
        .on_qr_code(|code, timeout| async move {
            info!(
                expires_in_seconds = timeout.as_secs(),
                "WhatsApp pairing QR code received"
            );
            print_qr_code(&code);
        })
        .on_connected(|client| async move {
            let pn_jid = client.pn().map(|jid| jid.to_string());
            let lid_jid = client.lid().map(|jid| jid.to_string());
            info!(
                ?pn_jid,
                ?lid_jid,
                "WhatsApp paired; choose the self-chat identity for OWNER_JID"
            );
        })
        .on_logged_out(|_| async move {
            warn!("WhatsApp logged out; re-pairing is required");
        })
        .build()
        .await
        .map_err(|error| AgentError::Config(format!("WhatsApp client setup failed: {error}")))?;
    bot.run().await;
    Ok(())
}
