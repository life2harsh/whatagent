use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine};
use ring::{
    rand::SystemRandom,
    signature::{RsaKeyPair, RSA_PKCS1_SHA256},
};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::error::AgentError;

pub const CALENDAR_EVENTS_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";

/// Supplies OAuth bearer tokens for Google APIs.
#[async_trait]
pub trait AccessTokenSource: Send + Sync {
    async fn access_token(&self) -> Result<String, AgentError>;
}

/// A fixed token, used by tests and local experiments.
pub struct StaticToken(pub String);

#[async_trait]
impl AccessTokenSource for StaticToken {
    async fn access_token(&self) -> Result<String, AgentError> {
        Ok(self.0.clone())
    }
}

#[derive(Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    token_uri: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

/// Exchanges a signed service-account JWT for short-lived access tokens.
/// Unlike a user OAuth refresh token, this never needs reconnecting.
pub struct ServiceAccountAuth {
    http: reqwest::Client,
    client_email: String,
    token_uri: String,
    scope: String,
    key_pair: Arc<RsaKeyPair>,
    cached: Mutex<Option<(String, Instant)>>,
}

impl ServiceAccountAuth {
    pub fn from_file(
        http: reqwest::Client,
        path: impl AsRef<Path>,
        scope: &str,
    ) -> Result<Self, AgentError> {
        let path = path.as_ref();
        let json = std::fs::read_to_string(path).map_err(|error| {
            AgentError::Config(format!(
                "could not read service account key {}: {error}",
                path.display()
            ))
        })?;
        Self::from_json(http, &json, scope)
    }

    pub fn from_json(http: reqwest::Client, json: &str, scope: &str) -> Result<Self, AgentError> {
        let key: ServiceAccountKey = serde_json::from_str(json).map_err(|error| {
            AgentError::Config(format!("service account key is not valid JSON: {error}"))
        })?;
        let der = pem_to_der(&key.private_key)?;
        let key_pair = RsaKeyPair::from_pkcs8(&der).map_err(|error| {
            AgentError::Config(format!("service account private key is invalid: {error}"))
        })?;
        Ok(Self {
            http,
            client_email: key.client_email,
            token_uri: key.token_uri,
            scope: scope.to_owned(),
            key_pair: Arc::new(key_pair),
            cached: Mutex::new(None),
        })
    }

    fn signed_assertion(&self) -> Result<String, AgentError> {
        let now = chrono::Utc::now().timestamp();
        let header = serde_json::json!({ "alg": "RS256", "typ": "JWT" });
        let claims = serde_json::json!({
            "iss": self.client_email,
            "scope": self.scope,
            "aud": self.token_uri,
            "iat": now,
            "exp": now + 3600,
        });
        let encode = |value: &serde_json::Value| {
            general_purpose::URL_SAFE_NO_PAD.encode(value.to_string().as_bytes())
        };
        let signing_input = format!("{}.{}", encode(&header), encode(&claims));
        let mut signature = vec![0; self.key_pair.public().modulus_len()];
        self.key_pair
            .sign(
                &RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                signing_input.as_bytes(),
                &mut signature,
            )
            .map_err(|_| AgentError::Config("could not sign service account JWT".to_owned()))?;
        Ok(format!(
            "{signing_input}.{}",
            general_purpose::URL_SAFE_NO_PAD.encode(signature)
        ))
    }
}

#[async_trait]
impl AccessTokenSource for ServiceAccountAuth {
    async fn access_token(&self) -> Result<String, AgentError> {
        let mut cached = self.cached.lock().await;
        if let Some((token, expires_at)) = cached.as_ref() {
            if Instant::now() < *expires_at {
                return Ok(token.clone());
            }
        }
        let assertion = self.signed_assertion()?;
        let response = self
            .http
            .post(&self.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AgentError::upstream(
                "Google auth",
                format!("{status}: {body}"),
            ));
        }
        let token: TokenResponse = response
            .json()
            .await
            .map_err(|error| AgentError::upstream("Google auth", error.to_string()))?;
        // Refresh a minute early so a token never expires mid-request.
        let lifetime = Duration::from_secs(token.expires_in.saturating_sub(60));
        *cached = Some((token.access_token.clone(), Instant::now() + lifetime));
        Ok(token.access_token)
    }
}

fn pem_to_der(pem: &str) -> Result<Vec<u8>, AgentError> {
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .map(str::trim)
        .collect();
    general_purpose::STANDARD.decode(body).map_err(|error| {
        AgentError::Config(format!(
            "service account private key is not valid PEM: {error}"
        ))
    })
}
