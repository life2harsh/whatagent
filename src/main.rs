use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use wa_calendar_agent::{
    app::Agent,
    calendar::CalendarWebhookClient,
    config::Config,
    db::StateStore,
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
    Pair,
    Run,
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
        let session_database_path = Config::whatsapp_session_database_path_from_env()?;
        wa_calendar_agent::whatsapp::pair(session_database_path).await?;
        return Ok(());
    }

    let config = Config::from_env()?;
    let http = reqwest::Client::builder()
        .timeout(config.request_timeout)
        .build()?;
    let calendar = CalendarWebhookClient::new(
        http.clone(),
        config.calendar_webhook_url.clone(),
        config.audio_webhook_url.clone(),
        config.confirm_webhook_url.clone(),
        config.webhook_secret.clone(),
    );
    let keep = config
        .keep_webhook_url
        .clone()
        .map(|url| GoogleKeepWebhookClient::new(http, url, config.webhook_secret.clone()));
    let store = StateStore::connect(&config.database_url).await?;
    let owner_jid = config.owner_jid.clone();
    let session_database_path = config.whatsapp_session_database_path.clone();
    let agent = Arc::new(Agent::new(config, calendar, keep, store));

    match cli.command {
        Command::Pair => unreachable!(),
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
