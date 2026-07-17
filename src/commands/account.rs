use std::path::Path;

use serde_json::{Value, json};

use crate::account as domain;
use crate::config::{AccountEntry, AuthFlow, Config};
use crate::context::AccountContext;
use crate::error::CliError;
use crate::secrets::{AccountSecrets, SecretStore};

pub struct AddArgs {
    pub alias: String,
    pub company_id: String,
    pub flow: AuthFlow,
    pub client_id: String,
    pub client_secret: String,
    pub user_id: Option<String>,
    pub entity_id: Option<String>,
}

pub fn add(config_path: &Path, store: &dyn SecretStore, args: AddArgs) -> Result<Value, CliError> {
    let AuthFlow::ClientCredentials = args.flow else {
        return Err(CliError::Auth(
            "auth-code accounts arrive in a later task".into(),
        ));
    };
    let user_id = args.user_id.as_deref().ok_or_else(|| {
        CliError::Usage("--user-id is required for the client-credentials flow".into())
    })?;
    let username = domain::username_for(user_id, &args.company_id, args.entity_id.as_deref());

    // Write the config entry before touching the keychain: if the secrets write below fails,
    // the config still points at an alias with no stored credentials, which is a self-describing
    // and re-runnable state ("no credentials stored for '<alias>'; run account add"). The
    // reverse order can leave secrets under an alias the config never learns about.
    let is_default = write_account_entry(config_path, &args, user_id)?;
    store.set(
        &args.alias,
        &AccountSecrets::ClientCredentials {
            client_id: args.client_id.clone(),
            client_secret: args.client_secret.clone(),
            username,
        },
    )?;

    Ok(json!({
        "alias": args.alias,
        "companyId": args.company_id,
        "flow": "client-credentials",
        "default": is_default,
    }))
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

fn write_account_entry(
    config_path: &Path,
    args: &AddArgs,
    user_id: &str,
) -> Result<bool, CliError> {
    let mut config = Config::load(config_path)?;
    config.accounts.insert(
        args.alias.clone(),
        AccountEntry {
            company_id: args.company_id.clone(),
            user_id: Some(user_id.to_string()),
            entity_id: args.entity_id.clone(),
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
