use std::io::IsTerminal;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::commands::{
    self, account, composite, config_cmd, describe, export, job, object, query, raw, report, skill,
    update, view,
};
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
    /// Get or set configuration values
    Config {
        #[command(subcommand)]
        action: ConfigAction,
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
        #[command(flatten)]
        query: QueryFlags,
        /// Page through the full result set, merging pages into one envelope
        #[arg(long)]
        all: bool,
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
    /// Submit a composite batch request (2-10 sub-requests), each with its own method/path/body
    #[command(
        after_help = "Sub-request format: {\"method\": \"POST\", \"path\": \"/objects/...\", \"body\": {...}, \"resultReference\": \"vendor\", \"headers\": {...}}\n\nExample: intacct-cli composite --data '[{\"method\":\"GET\",\"path\":\"/objects/accounts-payable/vendor/1\"},{\"method\":\"GET\",\"path\":\"/objects/accounts-payable/vendor/2\"}]'"
    )]
    Composite {
        /// JSON array of 2-10 sub-requests (inline JSON, @file, or - for stdin)
        #[arg(long)]
        data: String,
    },
    /// Get the session ID for the current Bearer token (XML-API escape hatch)
    SessionId,
    /// Execute a system or user view, or list system views for an object
    #[command(
        after_help = "Examples:\n  intacct-cli view run expenses/employee-expense::systemfw1 --view-type system --size 10\n  intacct-cli view list accounts-payable/vendor"
    )]
    View {
        #[command(subcommand)]
        action: ViewAction,
    },
    /// Export query results to a file (pdf, csv, word, xml, or xlsx)
    #[command(
        after_help = "Examples:\n  intacct-cli export accounts-payable/vendor --file-type csv --output vendors.csv --fields key,id,name\n  intacct-cli export --file-type xlsx --output out.xlsx --body '{\"object\":\"accounts-payable/vendor\",\"fields\":[\"key\"]}'"
    )]
    Export {
        /// Object path, e.g. accounts-payable/vendor (omit when --body is given)
        object: Option<String>,
        #[arg(value_enum, long = "file-type")]
        file_type: FileTypeArg,
        /// File path to write the exported binary to; refuses to overwrite an existing file
        #[arg(long)]
        output: std::path::PathBuf,
        #[command(flatten)]
        query: QueryFlags,
        /// Raw query body (JSON, @file, or - for stdin); mutually exclusive with the structured flags
        #[arg(long)]
        body: Option<String>,
    },
    /// Run, check, download, or cancel a stored/memorized report
    Report {
        #[command(subcommand)]
        action: ReportAction,
    },
    /// Install or refresh the bundled agent skill (SKILL.md) into your Claude skills dir
    #[command(
        after_help = "Examples:\n  intacct-cli skill install\n  intacct-cli skill install --dir ~/.config/claude/skills/intacct-cli"
    )]
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Self-update from GitHub releases (requires GITHUB_TOKEN for this private repo)
    #[command(after_help = "Examples:\n  intacct-cli update --check\n  intacct-cli update")]
    Update {
        /// Report the current/latest versions without installing
        #[arg(long)]
        check: bool,
        /// Skip re-running `skill install` after a successful update
        #[arg(long = "no-skill")]
        no_skill: bool,
    },
}

#[derive(Subcommand)]
pub enum SkillAction {
    /// Write the embedded SKILL.md to your Claude skills dir (skips a symlinked/repo-tracked copy)
    Install {
        /// Target dir (default: $CLAUDE_CONFIG_DIR or ~/.claude, under skills/intacct-cli)
        #[arg(long)]
        dir: Option<std::path::PathBuf>,
    },
}

/// Structured query flags shared by `query` and `export` (the exported file's contents are
/// governed by the same query grammar as `/services/core/query`).
#[derive(clap::Args)]
pub struct QueryFlags {
    /// Comma-separated field list (aggregates like sum:amount allowed); required unless --body is given
    #[arg(long)]
    pub fields: Option<String>,
    /// A filter object, e.g. {"$eq":{"status":"active"}}; may be repeated
    #[arg(long = "filter")]
    pub filters: Vec<String>,
    #[arg(long = "filter-expression")]
    pub filter_expression: Option<String>,
    /// An orderBy entry, e.g. {"id":"asc"}; may be repeated
    #[arg(long = "order-by")]
    pub order_by: Vec<String>,
    #[arg(long)]
    pub start: Option<u64>,
    #[arg(long)]
    pub size: Option<u64>,
    #[arg(long = "as-of-date")]
    pub as_of_date: Option<String>,
    #[arg(long = "case-sensitive")]
    pub case_sensitive: bool,
    #[arg(long = "include-private")]
    pub include_private: bool,
    #[arg(long = "include-hierarchy-fields")]
    pub include_hierarchy_fields: bool,
}

#[derive(Subcommand)]
pub enum ViewAction {
    /// Execute a system or user view by key
    #[command(
        after_help = "Example: intacct-cli view run expenses/employee-expense::systemfw1 --view-type system --size 10"
    )]
    Run {
        key: String,
        #[arg(value_enum, long = "view-type")]
        view_type: ViewTypeArg,
        #[arg(long)]
        start: Option<u64>,
        #[arg(long)]
        size: Option<u64>,
    },
    /// List system views for an object
    #[command(after_help = "Example: intacct-cli view list accounts-payable/vendor")]
    List {
        /// Object path, e.g. accounts-payable/vendor
        object: String,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum ViewTypeArg {
    System,
    User,
}

impl ViewTypeArg {
    fn as_str(self) -> &'static str {
        match self {
            ViewTypeArg::System => "system",
            ViewTypeArg::User => "user",
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum FileTypeArg {
    Pdf,
    Csv,
    Word,
    Xml,
    Xlsx,
}

impl FileTypeArg {
    fn as_str(self) -> &'static str {
        match self {
            FileTypeArg::Pdf => "pdf",
            FileTypeArg::Csv => "csv",
            FileTypeArg::Word => "word",
            FileTypeArg::Xml => "xml",
            FileTypeArg::Xlsx => "xlsx",
        }
    }
}

#[derive(Subcommand)]
pub enum ReportAction {
    /// Submit a stored/memorized report to run
    #[command(after_help = "Example: intacct-cli report run 1 --output-type pdf")]
    Run {
        report_id: String,
        #[arg(value_enum, long = "output-type")]
        output_type: OutputTypeArg,
        #[arg(value_enum, long = "output-location", default_value = "intacct")]
        output_location: OutputLocationArg,
    },
    /// Check a report run's status
    #[command(after_help = "Example: intacct-cli report status 1 --output-type pdf")]
    Status {
        report_id: String,
        #[arg(value_enum, long = "output-type")]
        output_type: OutputTypeArg,
        #[arg(value_enum, long = "output-location", default_value = "intacct")]
        output_location: OutputLocationArg,
    },
    /// Download a completed report's output to a file
    #[command(
        after_help = "Example: intacct-cli report download 1 --output-type pdf --output report.pdf"
    )]
    Download {
        report_id: String,
        #[arg(value_enum, long = "output-type")]
        output_type: OutputTypeArg,
        /// File path to write the downloaded binary to; refuses to overwrite an existing file
        #[arg(long)]
        output: std::path::PathBuf,
    },
    /// Cancel a queued report run
    #[command(after_help = "Example: intacct-cli report cancel 1 --output-type pdf")]
    Cancel {
        report_id: String,
        #[arg(value_enum, long = "output-type")]
        output_type: OutputTypeArg,
        #[arg(value_enum, long = "output-location", default_value = "intacct")]
        output_location: OutputLocationArg,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OutputTypeArg {
    Html,
    Pdf,
    Csv,
    Excel,
    Text,
    Fec,
    Zip,
}

impl OutputTypeArg {
    fn as_str(self) -> &'static str {
        match self {
            OutputTypeArg::Html => "html",
            OutputTypeArg::Pdf => "pdf",
            OutputTypeArg::Csv => "csv",
            OutputTypeArg::Excel => "excel",
            OutputTypeArg::Text => "text",
            OutputTypeArg::Fec => "fec",
            OutputTypeArg::Zip => "zip",
        }
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
pub enum OutputLocationArg {
    Intacct,
    Cloud,
}

impl OutputLocationArg {
    fn as_str(self) -> &'static str {
        match self {
            OutputLocationArg::Intacct => "intacct",
            OutputLocationArg::Cloud => "cloud",
        }
    }
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
pub enum ConfigAction {
    /// Get one or all configuration values
    #[command(
        after_help = "Examples:\n  intacct-cli config get\n  intacct-cli config get default_account"
    )]
    Get {
        /// Configuration key to retrieve; omit to get all values
        key: Option<String>,
    },
    /// Set a configuration value
    #[command(
        after_help = "Examples:\n  intacct-cli config set default_account prod\n  intacct-cli config set cache_ttl_hours 48"
    )]
    Set { key: String, value: String },
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
        /// Loopback port for the auth-code browser redirect (443 keeps the registered
        /// redirect URI portless, the only form Sage accepts)
        #[arg(long, default_value_t = 443)]
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
        #[arg(long, default_value_t = 443)]
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
        Command::Config { action } => dispatch_config(action),
        Command::Object { action } => dispatch_object(cli, action).await,
        Command::Query {
            object,
            query,
            all,
            body,
        } => dispatch_query(cli, object, query, *all, body).await,
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
        Command::Composite { data } => {
            let sub_requests = commands::read_data_arg(data)?;
            let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
            composite::run(&context.client, sub_requests).await
        }
        Command::SessionId => {
            let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
            composite::session_id(&context.client).await
        }
        Command::View { action } => dispatch_view(cli, action).await,
        Command::Export {
            object,
            file_type,
            output,
            query,
            body,
        } => dispatch_export(cli, object, *file_type, output, query, body).await,
        Command::Report { action } => dispatch_report(cli, action).await,
        Command::Skill { action } => match action {
            SkillAction::Install { dir } => skill::install(dir.as_deref()),
        },
        Command::Update { check, no_skill } => update::run(*check, *no_skill).await,
    }
}

async fn dispatch_view(cli: &Cli, action: &ViewAction) -> Result<serde_json::Value, CliError> {
    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    match action {
        ViewAction::Run {
            key,
            view_type,
            start,
            size,
        } => view::run(&context.client, key, view_type.as_str(), *start, *size).await,
        ViewAction::List { object } => view::list_system_views(&context.client, object).await,
    }
}

/// Structured flags and `--body` are mutually exclusive, mirroring `query`'s rule.
async fn dispatch_export(
    cli: &Cli,
    object: &Option<String>,
    file_type: FileTypeArg,
    output: &std::path::Path,
    query: &QueryFlags,
    body: &Option<String>,
) -> Result<serde_json::Value, CliError> {
    let query_body = build_query_or_body(object, query, body)?;
    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    export::run(&context.client, file_type.as_str(), query_body, output).await
}

async fn dispatch_report(cli: &Cli, action: &ReportAction) -> Result<serde_json::Value, CliError> {
    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    match action {
        ReportAction::Run {
            report_id,
            output_type,
            output_location,
        } => {
            report::run(
                &context.client,
                report_id,
                output_type.as_str(),
                output_location.as_str(),
            )
            .await
        }
        ReportAction::Status {
            report_id,
            output_type,
            output_location,
        } => {
            report::status(
                &context.client,
                report_id,
                output_type.as_str(),
                output_location.as_str(),
            )
            .await
        }
        ReportAction::Download {
            report_id,
            output_type,
            output,
        } => report::download(&context.client, report_id, output_type.as_str(), output).await,
        ReportAction::Cancel {
            report_id,
            output_type,
            output_location,
        } => {
            report::cancel(
                &context.client,
                report_id,
                output_type.as_str(),
                output_location.as_str(),
            )
            .await
        }
    }
}

async fn dispatch_query(
    cli: &Cli,
    object: &Option<String>,
    query: &QueryFlags,
    all: bool,
    body: &Option<String>,
) -> Result<serde_json::Value, CliError> {
    let request_body = build_query_or_body(object, query, body)?;
    let context = context_for(cli.account.as_deref(), cli.entity.as_deref())?;
    query::run(&context.client, request_body, all).await
}

/// Shared by `query` and `export`: structured flags and `--body` are mutually exclusive,
/// either give an object path with the structured flags, or a raw body.
fn build_query_or_body(
    object: &Option<String>,
    query: &QueryFlags,
    body: &Option<String>,
) -> Result<serde_json::Value, CliError> {
    let structured_flags_set = object.is_some()
        || query.fields.is_some()
        || !query.filters.is_empty()
        || query.filter_expression.is_some()
        || !query.order_by.is_empty()
        || query.start.is_some()
        || query.size.is_some()
        || query.as_of_date.is_some()
        || query.case_sensitive
        || query.include_private
        || query.include_hierarchy_fields;

    match body {
        Some(raw_body) => {
            if structured_flags_set {
                return Err(CliError::Usage(
                    "pass either --body or the structured flags, not both".into(),
                ));
            }
            commands::read_data_arg(raw_body)
        }
        None => {
            let object = object.clone().ok_or_else(|| {
                CliError::Usage("OBJECT is required unless --body is given".into())
            })?;
            query::build_query_body(&query::QueryArgs {
                object,
                fields: query.fields.clone(),
                filters: query.filters.clone(),
                filter_expression: query.filter_expression.clone(),
                order_by: query.order_by.clone(),
                start: query.start,
                size: query.size,
                as_of_date: query.as_of_date.clone(),
                case_sensitive: query.case_sensitive,
                include_private: query.include_private,
                include_hierarchy_fields: query.include_hierarchy_fields,
            })
        }
    }
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
    let cache_ttl = Duration::from_secs(config.cache_ttl_hours.unwrap_or(24).saturating_mul(3600));
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

fn dispatch_config(action: &ConfigAction) -> Result<serde_json::Value, CliError> {
    let config_path = crate::config::default_config_path();
    match action {
        ConfigAction::Get { key } => config_cmd::get(&config_path, key.as_deref()),
        ConfigAction::Set { key, value } => config_cmd::set(&config_path, key, value),
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
        assert_eq!(port, 443);
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
