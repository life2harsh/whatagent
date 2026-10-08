use std::{env, path::PathBuf, time::Duration};

use chrono_tz::Tz;
use url::Url;

use crate::{
    error::AgentError,
    google::{calendar, gemini},
};

#[derive(Debug, Clone)]
pub struct Config {
    pub owner_jid: String,
    pub gemini_api_key: String,
    pub gemini_base_url: Url,
    pub gemini_text_models: Vec<String>,
    pub gemini_audio_models: Vec<String>,
    pub google_service_account_key_path: PathBuf,
    pub google_calendar_id: String,
    pub google_calendar_base_url: Url,
    pub keep_bridge_url: Option<Url>,
    pub keep_bridge_token: Option<String>,
    pub request_timeout: Duration,
    pub timezone: Tz,
    pub default_event_duration_minutes: u32,
    pub confirmation_ttl: Duration,
    pub reminder_poll_interval: Duration,
    pub database_url: String,
    pub whatsapp_session_database_path: String,
}

impl Config {
    pub fn whatsapp_session_database_path_from_env() -> String {
        env::var("WHATSAPP_SESSION_DATABASE_PATH").unwrap_or_else(|_| "wa-session.db".to_owned())
    }

    pub fn from_env() -> Result<Self, AgentError> {
        let timezone = required("USER_TIMEZONE")?.parse::<Tz>().map_err(|_| {
            AgentError::Config("USER_TIMEZONE must be a valid IANA timezone".to_owned())
        })?;
        let text_model =
            optional("GEMINI_MODEL").unwrap_or_else(|| "gemini-3.5-flash-lite".to_owned());
        let audio_model = optional("GEMINI_AUDIO_MODEL").unwrap_or_else(|| text_model.clone());
        let fallbacks: Vec<String> = optional("GEMINI_FALLBACK_MODELS")
            .unwrap_or_else(|| DEFAULT_GEMINI_FALLBACK_MODELS.to_owned())
            .split(',')
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        let keep_bridge_url = optional("KEEP_BRIDGE_URL")
            .map(|value| url("KEEP_BRIDGE_URL", &value))
            .transpose()?;
        let keep_bridge_token = optional("KEEP_BRIDGE_TOKEN");
        if keep_bridge_url.is_some() && keep_bridge_token.is_none() {
            return Err(AgentError::Config(
                "KEEP_BRIDGE_TOKEN must be set when KEEP_BRIDGE_URL is set".to_owned(),
            ));
        }

        Ok(Self {
            owner_jid: required("OWNER_JID")?,
            gemini_api_key: required("GEMINI_API_KEY")?,
            gemini_base_url: url(
                "GEMINI_BASE_URL",
                &optional("GEMINI_BASE_URL").unwrap_or_else(|| gemini::DEFAULT_BASE_URL.to_owned()),
            )?,
            gemini_text_models: model_chain(&text_model, &fallbacks),
            gemini_audio_models: model_chain(&audio_model, &fallbacks),
            google_service_account_key_path: required("GOOGLE_SERVICE_ACCOUNT_KEY_PATH")?.into(),
            google_calendar_id: required("GOOGLE_CALENDAR_ID")?,
            google_calendar_base_url: url(
                "GOOGLE_CALENDAR_BASE_URL",
                &optional("GOOGLE_CALENDAR_BASE_URL")
                    .unwrap_or_else(|| calendar::DEFAULT_BASE_URL.to_owned()),
            )?,
            keep_bridge_url,
            keep_bridge_token,
            request_timeout: Duration::from_secs(optional_parse("REQUEST_TIMEOUT_SECONDS", 30)?),
            timezone,
            default_event_duration_minutes: optional_parse("DEFAULT_EVENT_DURATION_MINUTES", 60)?,
            confirmation_ttl: Duration::from_secs(
                u64::from(optional_parse::<u32>("CONFIRMATION_TTL_MINUTES", 30)?) * 60,
            ),
            reminder_poll_interval: Duration::from_secs(optional_parse(
                "REMINDER_POLL_SECONDS",
                15,
            )?),
            database_url: required("DATABASE_URL")?,
            whatsapp_session_database_path: Self::whatsapp_session_database_path_from_env(),
        })
    }
}

/// Tried in order after the primary model when Gemini is overloaded.
const DEFAULT_GEMINI_FALLBACK_MODELS: &str =
    "gemini-3.6-flash,gemini-3.5-flash-lite,gemini-flash-latest";

/// The primary model followed by the fallbacks, without duplicates.
fn model_chain(primary: &str, fallbacks: &[String]) -> Vec<String> {
    let mut chain = vec![primary.to_owned()];
    for model in fallbacks {
        if !chain.contains(model) {
            chain.push(model.clone());
        }
    }
    chain
}

fn optional(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn optional_parse<T>(key: &str, default: T) -> Result<T, AgentError>
where
    T: std::str::FromStr,
{
    match optional(key) {
        Some(value) => value
            .parse()
            .map_err(|_| AgentError::Config(format!("{key} has an invalid value"))),
        None => Ok(default),
    }
}

fn required(key: &str) -> Result<String, AgentError> {
    optional(key).ok_or_else(|| AgentError::Config(format!("{key} must be set")))
}

fn url(key: &str, value: &str) -> Result<Url, AgentError> {
    Url::parse(value).map_err(|error| AgentError::Config(format!("{key} is invalid: {error}")))
}
