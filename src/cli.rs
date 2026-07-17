use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::commands::{self, account, describe, job, object, query, raw};
use crate::config::AuthFlow;
use crate::context::context_for;
use crate::error::CliError;
use crate::output;
use crate::secrets::KeyringStore;

#[derive(Parser)]
#[command(
    name = "intacct-cli",
    version,
    about = "Sage Intacct REST API CLI for AI agents",
    propagate_version = true,
    color = clap::ColorChoice::Never
)]
pub struct Cli {
    /// Account alias (falls back to $INTACCT_ACCOUNT, then the configured default)
    #[arg(long, global = true)]
    pub account: Option<String>,
    /// Entity id for entity-scoped Intacct accounts (top-level companies ignore this)
    #[arg(long, global = true)]
    pub entity: Option<String>,
    /// Pretty-print JSON output
    #[arg(long, global = true)]
    pub pretty: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage stored account credentials and aliases
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Generic CRUD against any Intacct REST object (application/object,
    /// application/document::Type, or platform-apps/nsp::name)
    Object {
        #[command(subcommand)]
        action: ObjectAction,
    },
    /// Read-only query service: structured filters/ordering, or a raw --body
    #[command(
        after_help = "Example: intacct-cli query accounts-payable/vendor --fields key,id --filter '{\"$eq\":{\"status\":\"active\"}}' --all"
    )]
    Query {
        /// Object path, e.g. accounts-payable/vendor (omit when --body is given)
        object: Option<String>,
        /// Comma-separated field list (aggregates like sum:amount allowed); required unless --body is given
        #[arg(long)]
        fields: Option<String>,
        /// A filter object, e.g. {"$eq":{"status":"active"}}; may be repeated
        #[arg(long = "filter")]
        filters: Vec<String>,
        #[arg(long = "filter-expression")]
        filter_expression: Option<String>,
        /// An orderBy entry, e.g. {"id":"asc"}; may be repeated
        #[arg(long = "order-by")]
        order_by: Vec<String>,
        #[arg(long)]
        start: Option<u64>,
        #[arg(long)]
        size: Option<u64>,
        /// Page through the full result set, merging pages into one envelope
        #[arg(long)]
        all: bool,
        #[arg(long = "as-of-date")]
        as_of_date: Option<String>,
        #[arg(long = "case-sensitive")]
        case_sensitive: bool,
        #[arg(long = "include-private")]
        include_private: bool,
        #[arg(long = "include-hierarchy-fields")]
        include_hierarchy_fields: bool,
        /// Raw request body (JSON, @file, or - for stdin); mutually exclusive with the structured flags
        #[arg(long)]
        body: Option<String>,
    },
    /// Look up model metadata for an object/service/workflow, or list what's available
    #[command(
        after_help = "Examples:\n  intacct-cli describe accounts-payable/vendor --schema\n  intacct-cli describe --list --type object --filter '^accounts-payable/'"
    )]
    Describe {
        /// Resource name, e.g. accounts-payable/vendor (omit when --list is given)
        #[arg(conflicts_with = "list")]
        name: Option<String>,
        /// Return the full schema instead of the short model summary
        #[arg(long)]
        schema: bool,
        /// Bypass the local metadata cache
        #[arg(long)]
        refresh: bool,
        /// List available resources instead of describing one
        #[arg(long)]
        list: bool,
        /// Resource type to list; only valid with --list
        #[arg(long, value_enum, default_value = "object", requires = "list")]
        r#type: ResourceType,
        /// Regex filtering the listed resource names; only valid with --list
        #[arg(long, requires = "list")]
        filter: Option<String>,
    },
    /// Send an arbitrary request to any Intacct REST endpoint
    #[command(
        after_help = "Example: intacct-cli raw GET /services/core/model --query type=service"
    )]
    Raw {
        #[arg(value_enum, ignore_case = true)]
        method: HttpMethodArg,
        path: String,
        /// Repeatable key=value query parameter
        #[arg(long = "query")]
        query: Vec<String>,
        /// Repeatable 'Name: value' header
        #[arg(long = "header")]
        header: Vec<String>,
        #[arg(long)]
        data: Option<String>,
    },
    /// Bulk asynchronous job service: submit a batch, then poll status or fetch results
    Job {
        #[command(subcommand)]
        action: JobAction,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ResourceType {
    Object,
    Service,
    Workflow,
}

impl ResourceType {
    fn as_str(self) -> &'static str {
        match self {
            ResourceType::Object => "object",
            ResourceType::Service => "service",
            ResourceType::Workflow => "workflow",
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum HttpMethodArg {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl HttpMethodArg {
    fn to_method(self) -> reqwest::Method {
        match self {
            HttpMethodArg::Get => reqwest::Method::GET,
            HttpMethodArg::Post => reqwest::Method::POST,
            HttpMethodArg::Put => reqwest::Method::PUT,
            HttpMethodArg::Patch => reqwest::Method::PATCH,
            HttpMethodArg::Delete => reqwest::Method::DELETE,
        }
    }
}

#[derive(Subcommand)]
pub enum ObjectAction {
    /// Fetch one or more objects by key (comma-separated for multiple)
    #[command(after_help = "Example: intacct-cli object get accounts-payable/vendor 42")]
    Get { object_path: String, keys: String },
    /// List objects, optionally paging through the full result set
    #[command(
        after_help = "Examples:\n  intacct-cli object list accounts-payable/vendor\n  intacct-cli object list general-ledger/account --start 1 --size 100 --all"
    )]
    List {
        object_path: String,
        #[arg(long)]
        start: Option<u64>,
        #[arg(long)]
        size: Option<u64>,
        #[arg(long)]
        all: bool,
    },
    /// Create one object (JSON object) or a batch (JSON array, capped at 500 by the API)
    #[command(
        after_help = "Examples:\n  intacct-cli object create accounts-payable/vendor --data '{\"name\":\"Acme\"}'\n  intacct-cli object create accounts-payable/vendor --data @vendors.json --atomic --idempotency-key abc-123"
    )]
    Create {
        object_path: String,
        #[arg(long)]
        data: String,
        #[arg(long)]
        atomic: bool,
        #[arg(long = "idempotency-key")]
        idempotency_key: Option<String>,
    },
    /// Update one object by key, or a batch (JSON array, each item carrying its own key)
    #[command(
        after_help = "Examples:\n  intacct-cli object update accounts-payable/vendor 42 --data '{\"name\":\"Acme Inc\"}'\n  intacct-cli object update accounts-payable/vendor --data '[{\"key\":\"42\",\"name\":\"Acme\"},{\"key\":\"43\",\"name\":\"Beta\"}]'"
    )]
    Update {
        object_path: String,
        key: Option<String>,
        #[arg(long)]
        data: String,
        #[arg(long)]
        atomic: bool,
        #[arg(long = "idempotency-key")]
        idempotency_key: Option<String>,
    },
    /// Delete one or more objects by key (comma-separated for multiple)
    #[command(after_help = "Example: intacct-cli object delete accounts-payable/vendor 42,43")]
    Delete { object_path: String, keys: String },
}

#[derive(Subcommand)]
pub enum JobAction {
    /// Submit a batch of objects as an asynchronous bulk job
    #[command(
        after_help = "Examples:\n  intacct-cli job submit accounts-payable/vendor create --data '[{\"name\":\"Acme\"}]'\n  intacct-cli job submit accounts-payable/vendor update --data @vendors.json --callback-url https://example.com/hook"
    )]
    Submit {
        object_name: String,
        #[arg(value_enum)]
        operation: JobOperation,
        /// A JSON array of objects (inline JSON, @file, or - for stdin)
        #[arg(long)]
        data: String,
        #[arg(long = "callback-url")]
        callback_url: Option<String>,
    },
    /// Check a bulk job's status
    #[command(after_help = "Example: intacct-cli job status 88.JOB1")]
    Status { job_id: String },
    /// Fetch a bulk job's full per-operation results (status with download=true)
    #[command(after_help = "Example: intacct-cli job result 88.JOB1")]
    Result { job_id: String },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum JobOperation {
    Create,
    Update,
    Delete,
}

impl JobOperation {
    fn as_str(self) -> &'static str {
        match self {
            JobOperation::Create => "create",
            JobOperation::Update => "update",
            JobOperation::Delete => "delete",
        }
    }
}

#[derive(Subcommand)]
pub enum AccountAction {
    /// Add or overwrite an account alias; the first account added becomes the default
    #[command(
        after_help = "Examples:\n  intacct-cli account add prod --company-id creativeplanning --flow client-credentials --client-id CID --user-id svc_api\n  intacct-cli account add sandbox --company-id creativeplanning --flow client-credentials --client-id CID --user-id svc_api --entity-id CentralUS-35\n  intacct-cli account add prod --company-id creativeplanning --flow auth-code --client-id CID\n\nThe client secret is never a flag: set INTACCT_CLI_CLIENT_SECRET, or run interactively to be prompted for it."
    )]
    Add {
        alias: String,
        #[arg(long = "company-id")]
        company_id: String,
        #[arg(long, value_enum)]
        flow: AuthFlow,
        #[arg(long = "client-id")]
        client_id: String,
        /// Required for the client-credentials flow
        #[arg(long = "user-id")]
        user_id: Option<String>,
        #[arg(long = "entity-id")]
        entity_id: Option<String>,
        /// Loopback port for the auth-code browser redirect
        #[arg(long, default_value_t = 8899)]
        port: u16,
        /// Skip the loopback listener; paste the redirect URL instead (auth-code flow only)
        #[arg(long)]
        paste: bool,
    },
    /// List configured account aliases (never prints secrets)
    List,
    /// Change which alias is used when --account/$INTACCT_ACCOUNT is not given
    #[command(after_help = "Example: intacct-cli account set-default prod")]
    SetDefault { alias: String },
    /// Remove an account alias and its stored secrets
    #[command(after_help = "Example: intacct-cli account remove prod")]
    Remove { alias: String },
    /// Revoke ALL of this account's Intacct API tokens at the authorization server
    #[command(
        after_help = "Example: intacct-cli account revoke prod\n  intacct-cli account revoke prod --yes"
    )]
    Revoke {
        alias: String,
        /// Skip the interactive confirmation prompt
        #[arg(long)]
        yes: bool,
    },
    /// Verify stored credentials by calling the metadata catalog
    #[command(
        after_help = "Example: intacct-cli account test\n  intacct-cli account test --account prod\n  intacct-cli account test --account prod --reauth"
    )]
    Test {
        /// Re-run the interactive login flow first (auth-code accounts only)
        #[arg(long)]
        reauth: bool,
        /// Loopback port for the auth-code browser redirect, used with --reauth
        #[arg(long, default_value_t = 8899)]
        port: u16,
        /// Skip the loopback listener; paste the redirect URL instead, used with --reauth
        #[arg(long)]
        paste: bool,
    },
}

pub async fn cli_main() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(clap_error) => return handle_clap_error(&clap_error),
    };
    match dispatch(&cli).await {
        Ok(result) => {
            output::print_json(&result, cli.pretty);
            0
        }
        Err(error) => {
            output::print_error(&error);
            error.exit_code() as i32
        }
    }
}

/// clap's own help/version/error rendering is human-readable text, not our stdout-JSON /
/// stderr-JSON contract. `--help`/`--version` are a documented exception (human text on stdout,
/// exit 0, matching what every other CLI does); every other parse failure (bad flag, missing
/// required arg, invalid enum value, etc.) is folded into the same `CliError::Usage` envelope
/// every other invocation error goes through, so an agent never has to special-case clap's output.
fn handle_clap_error(clap_error: &clap::Error) -> i32 {
    use clap::error::ErrorKind;
    match clap_error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
            print!("{clap_error}");
            0
        }
        _ => {
            let usage_error = CliError::Usage(clap_error.render().to_string().trim().to_string());
            output::print_error(&usage_error);
            usage_error.exit_code() as i32
        }
    }
}

async fn dispatch(cli: &Cli) -> Result<serde_json::Value, CliError> {
    match &cli.command {
        Command::Account { action } => dispatch_account(cli, action).await,
        Command::Object { action } => dispatch_object(cli, action).await,
        Command::Query {
            object,
            fields,
            filters,
            filter_expression,
            order_by,
            start,
            size,
            all,
            as_of_date,
            case_sensitive,
            include_private,
            include_hierarchy_fields,
            body,
        } => {
            dispatch_query(
                cli,
                object,
                fields,
                filters,
                filter_expression,
                order_by,
                *start,
                *size,
                *all,
                as_of_date,
                *case_sensitive,
                *include_private,
                *include_hierarchy_fields,
                body,
            )
            .await
        }
        Command::Describe {
            name,
            schema,
            refresh,
            list,
            r#type,
            filter,
        } => dispatch_describe(cli, name, *schema, *refresh, *list, *r#type, filter).await,
        Command::Raw {
            method,
            path,
            query,
            header,
            data,
        } => {
            let query_pairs = commands::parse_key_value_pairs(query, "--query")?;
            let header_pairs = commands::parse_header_pairs(header)?;
            let body = data
                .as_ref()
                .map(|d| commands::read_data_arg(d))
                .transpose()?;
            let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
            raw::call(
                &context.client,
                method.to_method(),
                path,
                &query_pairs,
                &header_pairs,
                body,
            )
            .await
        }
        Command::Job { action } => dispatch_job(cli, action).await,
    }
}

/// Structured flags and `--body` are mutually exclusive: either give an object path with the
/// structured flags, or a raw body (`--all` is the one flag that composes with both).
#[allow(clippy::too_many_arguments)]
async fn dispatch_query(
    cli: &Cli,
    object: &Option<String>,
    fields: &Option<String>,
    filters: &[String],
    filter_expression: &Option<String>,
    order_by: &[String],
    start: Option<u64>,
    size: Option<u64>,
    all: bool,
    as_of_date: &Option<String>,
    case_sensitive: bool,
    include_private: bool,
    include_hierarchy_fields: bool,
    body: &Option<String>,
) -> Result<serde_json::Value, CliError> {
    let structured_flags_set = object.is_some()
        || fields.is_some()
        || !filters.is_empty()
        || filter_expression.is_some()
        || !order_by.is_empty()
        || start.is_some()
        || size.is_some()
        || as_of_date.is_some()
        || case_sensitive
        || include_private
        || include_hierarchy_fields;

    let request_body = match body {
        Some(raw_body) => {
            if structured_flags_set {
                return Err(CliError::Usage(
                    "pass either --body or the structured flags, not both".into(),
                ));
            }
            commands::read_data_arg(raw_body)?
        }
        None => {
            let object = object.clone().ok_or_else(|| {
                CliError::Usage("OBJECT is required unless --body is given".into())
            })?;
            query::build_query_body(&query::QueryArgs {
                object,
                fields: fields.clone(),
                filters: filters.to_vec(),
                filter_expression: filter_expression.clone(),
                order_by: order_by.to_vec(),
                start,
                size,
                as_of_date: as_of_date.clone(),
                case_sensitive,
                include_private,
                include_hierarchy_fields,
            })?
        }
    };

    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    query::run(&context.client, request_body, all).await
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_describe(
    cli: &Cli,
    name: &Option<String>,
    schema: bool,
    refresh: bool,
    list: bool,
    resource_type: ResourceType,
    filter: &Option<String>,
) -> Result<serde_json::Value, CliError> {
    if !list && name.is_none() {
        return Err(CliError::Usage("pass a resource name or --list".into()));
    }
    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    if list {
        return describe::list_resources(&context.client, resource_type.as_str(), filter.clone())
            .await;
    }
    let name = name.as_deref().expect("checked above");
    let config = crate::config::Config::load(&crate::config::default_config_path())?;
    let cache_ttl = Duration::from_secs(config.cache_ttl_hours.unwrap_or(24) * 3600);
    let cache_dir = crate::config::default_cache_dir()
        .join("metadata")
        .join(&context.alias);
    describe::describe_resource(
        &context.client,
        name,
        schema,
        &cache_dir,
        refresh,
        cache_ttl,
    )
    .await
}

async fn dispatch_object(cli: &Cli, action: &ObjectAction) -> Result<serde_json::Value, CliError> {
    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    match action {
        ObjectAction::Get { object_path, keys } => {
            object::get(&context.client, object_path, keys).await
        }
        ObjectAction::List {
            object_path,
            start,
            size,
            all,
        } => object::list(&context.client, object_path, *start, *size, *all).await,
        ObjectAction::Create {
            object_path,
            data,
            atomic,
            idempotency_key,
        } => {
            object::create(
                &context.client,
                object_path,
                commands::read_data_arg(data)?,
                *atomic,
                idempotency_key.clone(),
            )
            .await
        }
        ObjectAction::Update {
            object_path,
            key,
            data,
            atomic,
            idempotency_key,
        } => {
            object::update(
                &context.client,
                object_path,
                key.as_deref(),
                commands::read_data_arg(data)?,
                *atomic,
                idempotency_key.clone(),
            )
            .await
        }
        ObjectAction::Delete { object_path, keys } => {
            object::delete(&context.client, object_path, keys).await
        }
    }
}

async fn dispatch_job(cli: &Cli, action: &JobAction) -> Result<serde_json::Value, CliError> {
    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    match action {
        JobAction::Submit {
            object_name,
            operation,
            data,
            callback_url,
        } => {
            job::submit(
                &context.client,
                object_name,
                operation.as_str(),
                commands::read_data_arg(data)?,
                callback_url.clone(),
            )
            .await
        }
        JobAction::Status { job_id } => job::status(&context.client, job_id, false).await,
        JobAction::Result { job_id } => job::status(&context.client, job_id, true).await,
    }
}

async fn dispatch_account(
    cli: &Cli,
    action: &AccountAction,
) -> Result<serde_json::Value, CliError> {
    let config_path = crate::config::default_config_path();
    let store = KeyringStore;
    match action {
        AccountAction::Add {
            alias,
            company_id,
            flow,
            client_id,
            user_id,
            entity_id,
            port,
            paste,
        } => {
            let client_secret = resolve_client_secret()?;
            let http = reqwest::Client::new();
            account::add(
                &config_path,
                &store,
                &http,
                account::AddArgs {
                    alias: alias.clone(),
                    company_id: company_id.clone(),
                    flow: *flow,
                    client_id: client_id.clone(),
                    client_secret,
                    user_id: user_id.clone(),
                    entity_id: entity_id.clone(),
                    port: *port,
                    paste: *paste,
                },
            )
            .await
        }
        AccountAction::List => account::list(&config_path),
        AccountAction::SetDefault { alias } => account::set_default(&config_path, alias),
        AccountAction::Remove { alias } => account::remove(&config_path, &store, alias),
        AccountAction::Revoke { alias, yes } => {
            let http = reqwest::Client::new();
            account::revoke(&config_path, Arc::new(store), alias, &http, *yes).await
        }
        AccountAction::Test {
            reauth,
            port,
            paste,
        } => {
            let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
            if *reauth {
                let http = reqwest::Client::new();
                account::test_with_reauth(&config_path, &store, &http, &context, *port, *paste)
                    .await
            } else {
                account::test(&context).await
            }
        }
    }
}

/// The client secret is never accepted as a CLI flag — that would leak it into shell history
/// and `ps` output. Resolution order: env var (for CI/non-interactive use), then a hidden
/// interactive prompt.
fn resolve_client_secret() -> Result<String, CliError> {
    if let Ok(secret) = std::env::var("INTACCT_CLI_CLIENT_SECRET") {
        return Ok(secret);
    }
    if std::io::stdin().is_terminal() {
        return rpassword::prompt_password("Client secret: ").map_err(|read_error| {
            CliError::Usage(format!("failed to read client secret: {read_error}"))
        });
    }
    Err(CliError::Usage(
        "set INTACCT_CLI_CLIENT_SECRET or run interactively to be prompted".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    /// `Cli` intentionally has no `Debug` impl (it carries no state worth dumping), so
    /// `Result::unwrap_err` (which requires `T: Debug`) doesn't work here; extract the error by
    /// hand instead.
    fn expect_parse_error(args: &[&str]) -> clap::Error {
        match Cli::try_parse_from(args) {
            Ok(_) => panic!("expected {args:?} to fail parsing"),
            Err(clap_error) => clap_error,
        }
    }

    #[test]
    fn bad_flag_yields_a_usage_kind_error_and_exit_code_two() {
        let clap_error = expect_parse_error(&["intacct-cli", "--bogus-flag"]);
        assert_ne!(clap_error.kind(), ErrorKind::DisplayHelp);
        assert_ne!(clap_error.kind(), ErrorKind::DisplayVersion);
        assert_eq!(handle_clap_error(&clap_error), 2);
    }

    #[test]
    fn help_flag_yields_display_help_kind_and_exit_code_zero() {
        let clap_error = expect_parse_error(&["intacct-cli", "--help"]);
        assert_eq!(clap_error.kind(), ErrorKind::DisplayHelp);
        assert_eq!(handle_clap_error(&clap_error), 0);
    }

    #[test]
    fn version_flag_yields_display_version_kind_and_exit_code_zero() {
        let clap_error = expect_parse_error(&["intacct-cli", "--version"]);
        assert_eq!(clap_error.kind(), ErrorKind::DisplayVersion);
        assert_eq!(handle_clap_error(&clap_error), 0);
    }

    #[test]
    fn account_add_parses_client_credentials_flow_with_kebab_case_value() {
        let cli = Cli::try_parse_from([
            "intacct-cli",
            "account",
            "add",
            "prod",
            "--company-id",
            "creativeplanning",
            "--flow",
            "client-credentials",
            "--client-id",
            "CID",
            "--user-id",
            "svc_api",
        ])
        .expect("client-credentials add should parse");
        let Command::Account {
            action:
                AccountAction::Add {
                    alias,
                    company_id,
                    flow,
                    client_id,
                    user_id,
                    entity_id,
                    port,
                    paste,
                },
        } = cli.command
        else {
            panic!("wrong variant")
        };
        assert_eq!(alias, "prod");
        assert_eq!(company_id, "creativeplanning");
        assert!(matches!(flow, AuthFlow::ClientCredentials));
        assert_eq!(client_id, "CID");
        assert_eq!(user_id.as_deref(), Some("svc_api"));
        assert_eq!(entity_id, None);
        assert_eq!(port, 8899);
        assert!(!paste);
    }

    #[test]
    fn account_add_parses_auth_code_flow_with_port_and_paste() {
        let cli = Cli::try_parse_from([
            "intacct-cli",
            "account",
            "add",
            "prod",
            "--company-id",
            "creativeplanning",
            "--flow",
            "auth-code",
            "--client-id",
            "CID",
            "--port",
            "9000",
            "--paste",
        ])
        .expect("auth-code add should parse");
        let Command::Account {
            action:
                AccountAction::Add {
                    flow,
                    user_id,
                    port,
                    paste,
                    ..
                },
        } = cli.command
        else {
            panic!("wrong variant")
        };
        assert!(matches!(flow, AuthFlow::AuthCode));
        assert_eq!(user_id, None);
        assert_eq!(port, 9000);
        assert!(paste);
    }

    #[test]
    fn account_revoke_parses_alias_and_yes_flag() {
        let cli = Cli::try_parse_from(["intacct-cli", "account", "revoke", "prod", "--yes"])
            .expect("revoke should parse");
        let Command::Account {
            action: AccountAction::Revoke { alias, yes },
        } = cli.command
        else {
            panic!("wrong variant")
        };
        assert_eq!(alias, "prod");
        assert!(yes);
    }

    #[test]
    fn account_test_parses_reauth_port_and_paste() {
        let cli = Cli::try_parse_from([
            "intacct-cli",
            "account",
            "test",
            "--reauth",
            "--port",
            "9001",
            "--paste",
        ])
        .expect("test --reauth should parse");
        let Command::Account {
            action:
                AccountAction::Test {
                    reauth,
                    port,
                    paste,
                },
        } = cli.command
        else {
            panic!("wrong variant")
        };
        assert!(reauth);
        assert_eq!(port, 9001);
        assert!(paste);
    }

    #[test]
    fn account_add_requires_flow_flag() {
        expect_parse_error(&[
            "intacct-cli",
            "account",
            "add",
            "prod",
            "--company-id",
            "creativeplanning",
            "--client-id",
            "CID",
        ]);
    }

    #[test]
    fn account_subcommands_parse() {
        Cli::try_parse_from(["intacct-cli", "account", "list"]).expect("list parses");
        Cli::try_parse_from(["intacct-cli", "account", "set-default", "prod"])
            .expect("set-default parses");
        Cli::try_parse_from(["intacct-cli", "account", "remove", "prod"]).expect("remove parses");
        Cli::try_parse_from(["intacct-cli", "account", "test"]).expect("test parses");
    }

    #[test]
    fn global_flags_parse_before_and_after_subcommand() {
        Cli::try_parse_from([
            "intacct-cli",
            "--account",
            "prod",
            "--entity",
            "CentralUS-35",
            "--pretty",
            "account",
            "list",
        ])
        .expect("global flags before subcommand should parse");
    }
}
