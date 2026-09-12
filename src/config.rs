use std::{env, time::Duration};

use chrono_tz::Tz;
use url::Url;

use crate::error::AgentError;

#[derive(Debug, Clone)]
pub struct Config {
    pub owner_jid: String,
    pub calendar_webhook_url: Url,
    pub audio_webhook_url: Url,
    pub confirm_webhook_url: Url,
    pub keep_webhook_url: Option<Url>,
    pub webhook_secret: String,
    pub request_timeout: Duration,
    pub timezone: Tz,
    pub default_event_duration_minutes: u32,
    pub confirmation_ttl: Duration,
    pub reminder_poll_interval: Duration,
    pub database_url: String,
    pub whatsapp_session_database_path: String,
}

impl Config {
    pub fn whatsapp_session_database_path_from_env() -> Result<String, AgentError> {
        Ok(env::var("WHATSAPP_SESSION_DATABASE_PATH")
            .unwrap_or_else(|_| "wa-session.db".to_owned()))
    }

    pub fn from_env() -> Result<Self, AgentError> {
        let base_url = required("N8N_BASE_URL")?;
        let calendar_path = required("N8N_CALENDAR_WEBHOOK_PATH")?;
        let audio_path = required("N8N_AUDIO_WEBHOOK_PATH")?;
        let confirm_path = required("N8N_CONFIRM_WEBHOOK_PATH")?;
        let timezone = required("USER_TIMEZONE")?.parse::<Tz>().map_err(|_| {
            AgentError::Config("USER_TIMEZONE must be a valid IANA timezone".to_owned())
        })?;

        Ok(Self {
            owner_jid: required("OWNER_JID")?,
            calendar_webhook_url: join_url(&base_url, &calendar_path)?,
            audio_webhook_url: join_url(&base_url, &audio_path)?,
            confirm_webhook_url: join_url(&base_url, &confirm_path)?,
            keep_webhook_url: optional("N8N_KEEP_WEBHOOK_PATH")
                .map(|path| join_url(&base_url, &path))
                .transpose()?,
            webhook_secret: required("N8N_WEBHOOK_SECRET")?,
            request_timeout: Duration::from_secs(parse("REQUEST_TIMEOUT_SECONDS")?),
            timezone,
            default_event_duration_minutes: parse("DEFAULT_EVENT_DURATION_MINUTES")?,
            confirmation_ttl: Duration::from_secs(
                u64::from(parse::<u32>("CONFIRMATION_TTL_MINUTES")?) * 60,
            ),
            reminder_poll_interval: Duration::from_secs(optional_parse(
                "REMINDER_POLL_SECONDS",
                15,
            )?),
            database_url: required("DATABASE_URL")?,
            whatsapp_session_database_path: Self::whatsapp_session_database_path_from_env()?,
        })
    }
}

fn optional(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn optional_parse<T>(key: &str, default: T) -> Result<T, AgentError>
where
    T: std::str::FromStr,
{
    match env::var(key) {
        Ok(value) => value
            .parse()
            .map_err(|_| AgentError::Config(format!("{key} has an invalid value"))),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(AgentError::Config(format!(
            "{key} could not be read: {error}"
        ))),
    }
}

fn required(key: &str) -> Result<String, AgentError> {
    env::var(key).map_err(|_| AgentError::Config(format!("{key} must be set")))
}

fn parse<T>(key: &str) -> Result<T, AgentError>
where
    T: std::str::FromStr,
{
    required(key)?
        .parse()
        .map_err(|_| AgentError::Config(format!("{key} has an invalid value")))
}

fn join_url(base: &str, path: &str) -> Result<Url, AgentError> {
    let mut url = Url::parse(base)
        .map_err(|error| AgentError::Config(format!("N8N_BASE_URL is invalid: {error}")))?;
    url.set_path(path);
    Ok(url)
}
