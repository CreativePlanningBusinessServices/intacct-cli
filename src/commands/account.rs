use std::io::{IsTerminal, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::account as domain;
use crate::auth::authcode;
use crate::config::{AccountEntry, AuthFlow, Config};
use crate::context::AccountContext;
use crate::error::CliError;
use crate::secrets::{AccountSecrets, CachedToken, SecretStore};

pub struct AddArgs {
    pub alias: String,
    pub company_id: String,
    pub flow: AuthFlow,
    pub credentials: CredentialSource,
    pub user_id: Option<String>,
    pub entity_id: Option<String>,
    pub port: u16,
    pub paste: bool,
}

/// Where an account's client id/secret come from: given inline at add time (and copied
/// into the account's keychain entry), or resolved through a named app registered with
/// `intacct-cli app add` (the account stores only a reference, so rotating the app's
/// secret covers every referencing account).
pub enum CredentialSource {
    Inline {
        client_id: String,
        client_secret: String,
    },
    App {
        name: String,
        client_id: String,
        client_secret: String,
    },
}

impl CredentialSource {
    fn client_id(&self) -> &str {
        match self {
            CredentialSource::Inline { client_id, .. }
            | CredentialSource::App { client_id, .. } => client_id,
        }
    }

    fn client_secret(&self) -> &str {
        match self {
            CredentialSource::Inline { client_secret, .. }
            | CredentialSource::App { client_secret, .. } => client_secret,
        }
    }

    fn app_name(&self) -> Option<&str> {
        match self {
            CredentialSource::Inline { .. } => None,
            CredentialSource::App { name, .. } => Some(name),
        }
    }
}

pub async fn add(
    config_path: &Path,
    store: &dyn SecretStore,
    http: &reqwest::Client,
    args: AddArgs,
) -> Result<Value, CliError> {
    match args.flow {
        AuthFlow::ClientCredentials => add_client_credentials(config_path, store, args),
        AuthFlow::AuthCode => add_auth_code(config_path, store, http, args).await,
    }
}

fn add_client_credentials(
    config_path: &Path,
    store: &dyn SecretStore,
    args: AddArgs,
) -> Result<Value, CliError> {
    let user_id = args.user_id.as_deref().ok_or_else(|| {
        CliError::Usage("--user-id is required for the client-credentials flow".into())
    })?;
    let username = domain::username_for(user_id, &args.company_id, args.entity_id.as_deref());

    // Write the config entry before touching the keychain: if the secrets write below fails,
    // the config still points at an alias with no stored credentials, which is a self-describing
    // and re-runnable state ("no credentials stored for '<alias>'; run account add"). The
    // reverse order can leave secrets under an alias the config never learns about.
    let is_default = write_account_entry(config_path, &args)?;
    let secrets = match &args.credentials {
        CredentialSource::Inline {
            client_id,
            client_secret,
        } => AccountSecrets::ClientCredentials {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            username,
        },
        CredentialSource::App { name, .. } => AccountSecrets::ClientCredentialsApp {
            app: name.clone(),
            username,
        },
    };
    store.set(&args.alias, &secrets)?;

    Ok(add_result(&args, "client-credentials", is_default))
}

fn add_result(args: &AddArgs, flow: &str, is_default: bool) -> Value {
    let mut result = json!({
        "alias": args.alias,
        "companyId": args.company_id,
        "flow": flow,
        "default": is_default,
    });
    if let Some(app) = args.credentials.app_name() {
        result["app"] = json!(app);
    }
    result
}

/// The auth-code flow has no offline `--user-id`/secret pair to validate up front — the only
/// way to know the credentials work is to actually run the interactive login. So the config
/// entry is written first (same ordering rationale as the client-credentials path above), then
/// `run_login_flow` drives the browser + loopback listener, and only a successful login gets
/// its refresh token written to the keychain.
async fn add_auth_code(
    config_path: &Path,
    store: &dyn SecretStore,
    http: &reqwest::Client,
    args: AddArgs,
) -> Result<Value, CliError> {
    let is_default = write_account_entry(config_path, &args)?;
    let token = authcode::run_login_flow(
        http,
        args.credentials.client_id(),
        args.credentials.client_secret(),
        authcode::LoginOptions {
            port: args.port,
            paste: args.paste,
        },
    )
    .await?;

    let secrets = match &args.credentials {
        CredentialSource::Inline {
            client_id,
            client_secret,
        } => AccountSecrets::AuthCode {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            refresh_token: token.refresh_token.clone(),
        },
        CredentialSource::App { name, .. } => AccountSecrets::AuthCodeApp {
            app: name.clone(),
            refresh_token: token.refresh_token.clone(),
        },
    };
    store.set(&args.alias, &secrets)?;

    Ok(add_result(&args, "auth-code", is_default))
}

pub fn list(config_path: &Path) -> Result<Value, CliError> {
    let config = Config::load(config_path)?;
    let accounts: Vec<Value> = config
        .accounts
        .iter()
        .map(|(alias, entry)| {
            let mut account = json!({
                "alias": alias,
                "companyId": entry.company_id,
                "flow": serde_json::to_value(entry.flow).expect("flow is serializable"),
                "default": config.default_account.as_deref() == Some(alias.as_str()),
            });
            if let Some(user_id) = &entry.user_id {
                account["userId"] = json!(user_id);
            }
            if let Some(entity_id) = &entry.entity_id {
                account["entityId"] = json!(entity_id);
            }
            if let Some(app) = &entry.app {
                account["app"] = json!(app);
            }
            account
        })
        .collect();
    Ok(json!({"count": accounts.len(), "accounts": accounts}))
}

pub fn set_default(config_path: &Path, alias: &str) -> Result<Value, CliError> {
    let mut config = Config::load(config_path)?;
    if !config.accounts.contains_key(alias) {
        return Err(CliError::Usage(format!(
            "unknown account alias '{alias}'; run `intacct-cli account list`"
        )));
    }
    config.default_account = Some(alias.to_string());
    config.save(config_path)?;
    Ok(json!({"defaultAccount": alias}))
}

pub fn remove(config_path: &Path, store: &dyn SecretStore, alias: &str) -> Result<Value, CliError> {
    let mut config = Config::load(config_path)?;
    if config.accounts.remove(alias).is_none() {
        return Err(CliError::Usage(format!(
            "unknown account alias '{alias}'; run `intacct-cli account list`"
        )));
    }
    if config.default_account.as_deref() == Some(alias) {
        config.default_account = None;
    }
    // Save the config removal before deleting keychain secrets: if the secrets delete below
    // fails after this, the alias is already gone from config (invisible to the CLI, and a
    // future `account add` for the same alias just overwrites the orphaned keychain entry).
    // The reverse order can leave the alias in config with no secrets behind it, which then
    // surfaces as a confusing "no credentials stored" error on unrelated commands later.
    config.save(config_path)?;
    store.delete(alias)?;
    Ok(json!({"removed": alias}))
}

pub async fn test(context: &AccountContext) -> Result<Value, CliError> {
    context
        .client
        .request(
            reqwest::Method::GET,
            "/services/core/model",
            &[
                ("name", "company-config/user".to_string()),
                ("schema", "false".to_string()),
            ],
            &[],
            None,
        )
        .await?;
    Ok(json!({"ok": true, "alias": context.alias, "companyId": context.company_id}))
}

/// `--reauth` re-runs the interactive login flow for an auth-code account, overwrites the
/// stored refresh token (and pre-seeds the access-token cache from the login response so the
/// immediately-following `test` call doesn't need a second network round trip), then runs the
/// normal test call. Client-credentials accounts have no browser flow to re-run.
pub async fn test_with_reauth(
    config_path: &Path,
    store: &dyn SecretStore,
    http: &reqwest::Client,
    context: &AccountContext,
    port: u16,
    paste: bool,
) -> Result<Value, CliError> {
    let config = Config::load(config_path)?;
    let entry = config.accounts.get(&context.alias).ok_or_else(|| {
        CliError::Usage(format!(
            "unknown account alias '{}'; run `intacct-cli account list`",
            context.alias
        ))
    })?;
    if !matches!(entry.flow, AuthFlow::AuthCode) {
        return Err(CliError::Usage(
            "--reauth is only valid for auth-code accounts".into(),
        ));
    }
    let Some(AccountSecrets::AuthCode {
        client_id,
        client_secret,
        ..
    }) = store.get(&context.alias)?
    else {
        return Err(CliError::Auth(format!(
            "no credentials stored for '{}'; run `intacct-cli account add`",
            context.alias
        )));
    };

    let token = authcode::run_login_flow(
        http,
        &client_id,
        &client_secret,
        authcode::LoginOptions { port, paste },
    )
    .await?;

    store.set(
        &context.alias,
        &AccountSecrets::AuthCode {
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            refresh_token: token.refresh_token.clone(),
        },
    )?;
    let now_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    store.set_token(
        &context.alias,
        &CachedToken {
            access_token: token.access_token.clone(),
            expires_at_epoch: now_epoch + token.expires_in,
        },
    )?;

    test(context).await
}

/// Revokes ALL of the account's Intacct API tokens at the authorization server, not just the
/// one cached locally by this CLI. Interactive by default (must type the alias back) because
/// that's a wide blast radius; `--yes` skips the prompt for scripted use. Only the cached
/// access token is cleared locally — the stored refresh token/config entry are left in place so
/// the account is still visible and its next use fails fast with a clear "run account add"
/// error instead of silently vanishing from `account list`.
pub async fn revoke(
    config_path: &Path,
    store: Arc<dyn SecretStore>,
    alias: &str,
    http: &reqwest::Client,
    yes: bool,
) -> Result<Value, CliError> {
    let config = Config::load(config_path)?;
    let entry = config.accounts.get(alias).ok_or_else(|| {
        CliError::Usage(format!(
            "unknown account alias '{alias}'; run `intacct-cli account list`"
        ))
    })?;

    confirm_revoke(alias, yes)?;

    let provider = crate::context::provider_for(alias, entry, store.clone())?;
    let access_token = provider.access_token().await?;

    let response = http
        .post(domain::revoke_url())
        .form(&[("token", access_token.as_str())])
        .send()
        .await
        .map_err(|send_error| CliError::Network(format!("revoke request failed: {send_error}")))?;
    let status = response.status();
    let body_text = response.text().await.map_err(|read_error| {
        CliError::Network(format!("reading revoke response failed: {read_error}"))
    })?;
    if !status.is_success() {
        return Err(CliError::Api {
            status: status.as_u16(),
            message: body_text,
            details: vec![],
            support_id: None,
        });
    }
    let parsed: Value = serde_json::from_str(&body_text)
        .map_err(|parse_error| CliError::Auth(format!("bad revoke response: {parse_error}")))?;

    store.delete_token(alias)?;

    Ok(parsed)
}

fn confirm_revoke(alias: &str, yes: bool) -> Result<(), CliError> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError::Usage(
            "revoke requires interactive confirmation; pass --yes to run non-interactively".into(),
        ));
    }
    eprint!(
        "This revokes ALL Intacct API tokens for this user/company, not just this CLI's. Type the alias to confirm: "
    );
    let _ = std::io::stderr().flush();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|read_error| {
            CliError::Usage(format!("failed to read confirmation: {read_error}"))
        })?;
    if input.trim() != alias {
        return Err(CliError::Usage(
            "confirmation did not match alias; revoke aborted".into(),
        ));
    }
    Ok(())
}

fn write_account_entry(config_path: &Path, args: &AddArgs) -> Result<bool, CliError> {
    let mut config = Config::load(config_path)?;
    config.accounts.insert(
        args.alias.clone(),
        AccountEntry {
            company_id: args.company_id.clone(),
            user_id: args.user_id.clone(),
            entity_id: args.entity_id.clone(),
            app: args.credentials.app_name().map(str::to_string),
            flow: args.flow,
        },
    );
    if config.default_account.is_none() {
        config.default_account = Some(args.alias.clone());
    }
    let is_default = config.default_account.as_deref() == Some(args.alias.as_str());
    config.save(config_path)?;
    Ok(is_default)
}
