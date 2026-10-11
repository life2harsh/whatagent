use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use wa_calendar_agent::{
    app::Agent,
    calendar::CalendarService,
    config::Config,
    db::StateStore,
    google::{
        auth::{ServiceAccountAuth, CALENDAR_EVENTS_SCOPE},
        calendar::GoogleCalendarClient,
        gemini::GeminiClient,
    },
    incoming::{CommandContent, IncomingCommand},
    keep::GoogleKeepWebhookClient,
};

#[derive(Parser)]
#[command(name = "wa-calendar-agent")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Link WhatsApp and print the OWNER_JID for your self-chat.
    Pair,
    /// Run the daemon.
    Run,
    /// Verify the Gemini key and Google Calendar access, then exit.
    Check,
    /// Process one message as if it came from the self-chat (creates a real event).
    TestCalendar { text: String },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();
    if matches!(&cli.command, Command::Pair) {
        let session_database_path = Config::whatsapp_session_database_path_from_env();
        wa_calendar_agent::whatsapp::pair(session_database_path).await?;
        return Ok(());
    }

    let config = Config::from_env()?;
    let http = reqwest::Client::builder()
        .timeout(config.request_timeout)
        .build()?;
    let gemini = GeminiClient::new(
        http.clone(),
        config.gemini_base_url.clone(),
        config.gemini_api_key.clone(),
        config.gemini_text_models.clone(),
        config.gemini_audio_models.clone(),
    );
    let google_auth = ServiceAccountAuth::from_file(
        http.clone(),
        &config.google_service_account_key_path,
        CALENDAR_EVENTS_SCOPE,
    )?;
    let google_calendar = GoogleCalendarClient::new(
        http.clone(),
        config.google_calendar_base_url.clone(),
        config.google_calendar_id.clone(),
        Arc::new(google_auth),
    );
    if matches!(&cli.command, Command::Check) {
        return check(&gemini, &google_calendar, &config.google_calendar_id).await;
    }
    let calendar = CalendarService::new(
        gemini,
        google_calendar,
        config.timezone,
        config.default_event_duration_minutes,
    );
    let keep = config
        .keep_bridge_url
        .clone()
        .zip(config.keep_bridge_token.clone())
        .map(|(url, token)| GoogleKeepWebhookClient::new(http, url, token));
    let store = StateStore::connect(&config.database_url).await?;
    let owner_jid = config.owner_jid.clone();
    let session_database_path = config.whatsapp_session_database_path.clone();
    let agent = Arc::new(Agent::new(config, calendar, keep, store));

    match cli.command {
        Command::Pair | Command::Check => unreachable!(),
        Command::Run => {
            if agent.google_keep_sync_enabled() {
                let keep_agent = Arc::clone(&agent);
                tokio::spawn(async move {
                    loop {
                        if let Err(error) = keep_agent.sync_google_keep_notes().await {
                            tracing::warn!(%error, "Google Keep sync loop failed");
                        }
                        tokio::time::sleep(keep_agent.reminder_poll_interval()).await;
                    }
                });
            }
            wa_calendar_agent::whatsapp::run(agent, session_database_path).await?
        }
        Command::TestCalendar { text } => {
            let command = IncomingCommand {
                message_id: format!("cli-{}", Uuid::new_v4()),
                chat_id: owner_jid.clone(),
                sender_id: owner_jid,
                timestamp: chrono::Utc::now(),
                content: CommandContent::Text(text),
            };
            match agent.process(command).await? {
                wa_calendar_agent::app::ProcessOutcome::Ignore => println!("ignored"),
                wa_calendar_agent::app::ProcessOutcome::Reply(message) => println!("{message}"),
            }
        }
    }
    Ok(())
}

async fn check(
    gemini: &GeminiClient,
    calendar: &GoogleCalendarClient,
    calendar_id: &str,
) -> anyhow::Result<()> {
    let mut failed = false;
    match gemini.check().await {
        Ok(model) => println!("ok    Gemini API key works ({model})"),
        Err(error) => {
            failed = true;
            println!("FAIL  Gemini: {}\n      {error}", error.user_message());
        }
    }
    match calendar.check_access().await {
        Ok(()) => println!("ok    Google Calendar access to {calendar_id}"),
        Err(error) => {
            failed = true;
            println!(
                "FAIL  Google Calendar: {}\n      {error}",
                error.user_message()
            );
        }
    }
    if failed {
        anyhow::bail!("setup check failed");
    }
    Ok(())
}
