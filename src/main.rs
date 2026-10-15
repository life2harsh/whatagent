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
    #[command(about = "Link WhatsApp and print the OWNER_JID for your self-chat")]
    Pair,
    #[command(about = "Run the daemon")]
    Run,
    #[command(about = "Verify the Gemini key and Google Calendar access (if set up), then exit")]
    Check,
    #[command(
        about = "Process one message as if it came from the self-chat (creates a real event if Calendar is set up)"
    )]
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
    let google_calendar = match &config.google_calendar {
        Some(settings) => Some(GoogleCalendarClient::new(
            http.clone(),
            config.google_calendar_base_url.clone(),
            settings.calendar_id.clone(),
            Arc::new(ServiceAccountAuth::from_file(
                http.clone(),
                &settings.service_account_key_path,
                CALENDAR_EVENTS_SCOPE,
            )?),
        )),
        None => None,
    };
    if matches!(&cli.command, Command::Check) {
        let calendar_id = config
            .google_calendar
            .as_ref()
            .map(|s| s.calendar_id.as_str());
        return check(&gemini, google_calendar.as_ref().zip(calendar_id)).await;
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
    calendar: Option<(&GoogleCalendarClient, &str)>,
) -> anyhow::Result<()> {
    let mut failed = false;
    match gemini.check().await {
        Ok(model) => println!("ok    Gemini API key works ({model})"),
        Err(error) => {
            failed = true;
            println!(
                "FAIL  Gemini: {}\n      {}",
                error.user_message(),
                brief(&error)
            );
        }
    }
    match calendar {
        None => println!("skip  Google Calendar isn't set up; events become WhatsApp reminders"),
        Some((calendar, calendar_id)) => match calendar.check_access().await {
            Ok(()) => println!("ok    Google Calendar access to {calendar_id}"),
            Err(error) => {
                failed = true;
                println!(
                    "FAIL  Google Calendar: {}\n      {}",
                    error.user_message(),
                    brief(&error)
                );
            }
        },
    }
    if failed {
        anyhow::bail!("setup check failed");
    }
    Ok(())
}

/// The first line of an error, shortened: enough to search for, without
/// dumping Google's whole JSON response on someone setting this up.
fn brief(error: &impl std::fmt::Display) -> String {
    let text = error.to_string();
    // Google's JSON body starts with "{"; the status before it says enough.
    let line = text.lines().next().unwrap_or_default();
    let line = line.split('{').next().unwrap_or_default().trim_end();
    let line = line.trim_end_matches(':');
    match line.char_indices().nth(160) {
        Some((cut, _)) => format!("{}...", &line[..cut]),
        None => line.to_owned(),
    }
}
