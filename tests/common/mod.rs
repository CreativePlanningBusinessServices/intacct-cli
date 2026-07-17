#![allow(dead_code)] // helpers are shared across test binaries; not every binary uses every helper

use std::future::Future;
use std::pin::Pin;

use intacct_cli::auth::TokenProvider;
use intacct_cli::error::CliError;

pub struct StaticToken;

impl TokenProvider for StaticToken {
    fn access_token<'life>(
        &'life self,
    ) -> Pin<Box<dyn Future<Output = Result<String, CliError>> + Send + 'life>> {
        Box::pin(async { Ok("TEST_TOKEN".to_string()) })
    }
    fn invalidate(&self) {}
}
