#[tokio::main]
async fn main() {
    std::process::exit(intacct_cli::cli::cli_main().await);
}
