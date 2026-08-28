use std::sync::Arc;

use crate::account;
use crate::auth::TokenProvider;
use crate::auth::authcode::AuthCodeProvider;
use crate::auth::client_credentials::{ClientCredentialsConfig, ClientCredentialsProvider};
use crate::client::IaClient;
use crate::config::{AccountEntry, AuthFlow, Config};
use crate::error::CliError;
use crate::secrets::{AccountSecrets, KeyringStore, ResolvingStore, SecretStore};

pub struct AccountContext {
    pub alias: String,
    pub company_id: String,
    pub client: IaClient,
}

pub fn context_for(
    alias_flag: Option<&str>,
    entity_flag: Option<&str>,
) -> Result<AccountContext, CliError> {
    let config = Config::load(&crate::config::default_config_path())?;
    let env_alias = std::env::var("INTACCT_ACCOUNT").ok();
    let alias = config.resolve_alias(alias_flag, env_alias.as_deref())?;
    let entry = &config.accounts[&alias];
    let store: Arc<dyn SecretStore> = Arc::new(ResolvingStore::new(KeyringStore));
    let provider = provider_for(&alias, entry, store)?;
    let entity = entity_flag
        .map(str::to_string)
        .or_else(|| entry.entity_id.clone());
    let client = IaClient::new(
        reqwest::Client::new(),
        account::api_base(),
        provider,
        entity,
    );
    Ok(AccountContext {
        alias,
        company_id: entry.company_id.clone(),
        client,
    })
}

pub(crate) fn provider_for(
    alias: &str,
    entry: &AccountEntry,
    store: Arc<dyn SecretStore>,
) -> Result<Arc<dyn TokenProvider>, CliError> {
    let secrets = store.get(alias)?.ok_or_else(|| {
        CliError::Auth(format!(
            "no credentials stored for '{alias}'; run `intacct-cli account add`"
        ))
    })?;
    match (entry.flow, secrets) {
        (
            AuthFlow::ClientCredentials,
            AccountSecrets::ClientCredentials {
                client_id,
                client_secret,
                username,
            },
        ) => {
            let config = ClientCredentialsConfig {
                token_url: account::token_url(),
                client_id,
                client_secret,
                username,
            };
            Ok(Arc::new(ClientCredentialsProvider::new(
                reqwest::Client::new(),
                alias.to_string(),
                config,
                store,
            )))
        }
        (AuthFlow::AuthCode, AccountSecrets::AuthCode { .. }) => {
            Ok(Arc::new(AuthCodeProvider::new(
                reqwest::Client::new(),
                alias.to_string(),
                account::token_url(),
                store,
            )))
        }
        _ => Err(CliError::Auth(format!(
            "stored credentials for '{alias}' do not match its configured flow"
        ))),
    }
}
