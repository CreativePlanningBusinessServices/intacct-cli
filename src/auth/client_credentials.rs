use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::auth::{TokenProvider, TokenResponse};
use crate::error::CliError;
use crate::secrets::{CachedToken, SecretStore};

pub struct ClientCredentialsConfig {
    pub token_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub username: String,
}

pub struct ClientCredentialsProvider {
    http: reqwest::Client,
    alias: String,
    config: ClientCredentialsConfig,
    store: Arc<dyn SecretStore>,
}

impl ClientCredentialsProvider {
    pub fn new(
        http: reqwest::Client,
        alias: String,
        config: ClientCredentialsConfig,
        store: Arc<dyn SecretStore>,
    ) -> Self {
        ClientCredentialsProvider {
            http,
            alias,
            config,
            store,
        }
    }

    async fn fetch_fresh_token(&self) -> Result<String, CliError> {
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let response = self
            .http
            .post(&self.config.token_url)
            .json(&json!({
                "grant_type": "client_credentials",
                "client_id": self.config.client_id,
                "client_secret": self.config.client_secret,
                "username": self.config.username,
            }))
            .send()
            .await
            .map_err(|send_error| {
                CliError::Network(format!("token request failed: {send_error}"))
            })?;

        let status = response.status();
        let body = response.text().await.map_err(|read_error| {
            CliError::Network(format!("reading token response failed: {read_error}"))
        })?;
        if !status.is_success() {
            return Err(CliError::Auth(format!(
                "token endpoint returned {status}: {body}"
            )));
        }
        let token: TokenResponse = serde_json::from_str(&body)
            .map_err(|parse_error| CliError::Auth(format!("bad token response: {parse_error}")))?;
        self.store.set_token(
            &self.alias,
            &CachedToken {
                access_token: token.access_token.clone(),
                expires_at_epoch: now_epoch + token.expires_in,
            },
        )?;
        Ok(token.access_token)
    }
}

impl TokenProvider for ClientCredentialsProvider {
    fn access_token<'life>(
        &'life self,
    ) -> Pin<Box<dyn Future<Output = Result<String, CliError>> + Send + 'life>> {
        Box::pin(async move {
            let now_epoch = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            if let Some(cached) = self.store.get_token(&self.alias)?
                && cached.is_valid_at(now_epoch)
            {
                return Ok(cached.access_token);
            }
            self.fetch_fresh_token().await
        })
    }

    fn invalidate(&self) {
        let _ = self.store.delete_token(&self.alias);
    }
}
