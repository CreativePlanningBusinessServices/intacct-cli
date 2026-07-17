use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use rand::Rng;
use sha2::{Digest, Sha256};

use crate::account;
use crate::auth::loopback;
use crate::auth::{TokenProvider, TokenResponse};
use crate::error::CliError;
use crate::secrets::{AccountSecrets, CachedToken, SecretStore};

pub struct LoginOptions {
    pub port: u16,
    pub paste: bool,
}

impl Default for LoginOptions {
    fn default() -> Self {
        LoginOptions {
            port: 8899,
            paste: false,
        }
    }
}

pub fn pkce_pair() -> (String, String) {
    let verifier = random_alphanumeric(64);
    let challenge = challenge_for_verifier(&verifier);
    (verifier, challenge)
}

fn random_alphanumeric(length: usize) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..length)
        .map(|_| CHARSET[rng.random_range(0..CHARSET.len())] as char)
        .collect()
}

fn random_hex(length: usize) -> String {
    const HEX_CHARSET: &[u8] = b"0123456789abcdef";
    let mut rng = rand::rng();
    (0..length)
        .map(|_| HEX_CHARSET[rng.random_range(0..HEX_CHARSET.len())] as char)
        .collect()
}

/// Split out from `pkce_pair` so the hashing step is testable against a fixed verifier
/// (RFC 7636 test vectors, or Intacct's own sample, both pin an exact verifier/challenge pair).
fn challenge_for_verifier(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn build_authorize_url(
    base_authorize_url: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
    challenge: &str,
) -> String {
    let mut url = url::Url::parse(base_authorize_url).expect("valid authorize base url");
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", "offline_access")
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    url.to_string()
}

fn parse_callback_query(query: &str, expected_state: &str) -> Result<String, CliError> {
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    let lookup = |name: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    };
    if let Some(oauth_error) = lookup("error") {
        return Err(CliError::Auth(format!(
            "authorization denied: {oauth_error}"
        )));
    }
    if lookup("state").as_deref() != Some(expected_state) {
        return Err(CliError::Auth("state mismatch — retry the login".into()));
    }
    lookup("code").ok_or_else(|| CliError::Auth("no authorization code in callback".into()))
}

async fn post_form(
    http: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
) -> Result<(reqwest::StatusCode, String), CliError> {
    let response = http
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|send_error| CliError::Network(format!("token request failed: {send_error}")))?;
    let status = response.status();
    let body = response.text().await.map_err(|read_error| {
        CliError::Network(format!("reading token response failed: {read_error}"))
    })?;
    Ok((status, body))
}

async fn exchange_code(
    http: &reqwest::Client,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<TokenResponse, CliError> {
    let (status, body) = post_form(
        http,
        token_url,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    if !status.is_success() {
        return Err(CliError::Auth(format!(
            "token endpoint returned {status}: {body}"
        )));
    }
    serde_json::from_str(&body)
        .map_err(|parse_error| CliError::Auth(format!("bad token response: {parse_error}")))
}

/// Interactive login: opens the browser at the authorize URL, then either reads the pasted
/// redirect URL (`paste`) or runs a one-shot HTTPS loopback listener to catch the redirect
/// itself. Intacct rejects plain `http://` redirect URIs, so the listener terminates TLS with
/// a throwaway self-signed cert for `localhost`.
pub async fn run_login_flow(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    options: LoginOptions,
) -> Result<TokenResponse, CliError> {
    let (verifier, challenge) = pkce_pair();
    let state = random_hex(32);
    let redirect_uri = format!("https://localhost:{}/callback", options.port);
    let url = build_authorize_url(
        &account::authorize_url(),
        client_id,
        &redirect_uri,
        &state,
        &challenge,
    );

    eprintln!("Open this URL to log in (or it will open automatically):\n{url}");
    let _ = webbrowser::open(&url);

    let code = if options.paste {
        loopback::read_pasted_redirect(|query| parse_callback_query(query, &state))?
    } else {
        loopback::listen_for_redirect(options.port, |query| parse_callback_query(query, &state))
            .await?
    };

    let token = exchange_code(
        http,
        &account::token_url(),
        client_id,
        client_secret,
        &code,
        &redirect_uri,
        &verifier,
    )
    .await?;

    if token.refresh_token.is_none() {
        return Err(CliError::Auth(
            "no refresh token returned — the authorize request must include scope=offline_access"
                .into(),
        ));
    }
    Ok(token)
}

pub struct AuthCodeProvider {
    http: reqwest::Client,
    alias: String,
    token_url: String,
    store: Arc<dyn SecretStore>,
}

impl AuthCodeProvider {
    pub fn new(
        http: reqwest::Client,
        alias: String,
        token_url: String,
        store: Arc<dyn SecretStore>,
    ) -> Self {
        AuthCodeProvider {
            http,
            alias,
            token_url,
            store,
        }
    }

    async fn refresh_access_token(&self) -> Result<String, CliError> {
        let Some(AccountSecrets::AuthCode {
            client_id,
            client_secret,
            refresh_token: Some(current_refresh),
        }) = self.store.get(&self.alias)?
        else {
            return Err(CliError::Auth(format!(
                "no refresh token stored for '{}'; run `intacct-cli account add` again",
                self.alias
            )));
        };

        let (status, body) = post_form(
            &self.http,
            &self.token_url,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &current_refresh),
                ("client_id", &client_id),
                ("client_secret", &client_secret),
            ],
        )
        .await?;
        if !status.is_success() {
            return Err(CliError::Auth(format!(
                "refresh failed ({status}): {body} — run `intacct-cli account add` to re-authenticate"
            )));
        }
        let token: TokenResponse = serde_json::from_str(&body)
            .map_err(|parse_error| CliError::Auth(format!("bad token response: {parse_error}")))?;

        // Persist the (possibly rotated) refresh token BEFORE caching/returning the access
        // token: if the store write fails, we must not hand back a working access token for a
        // rotation we couldn't record — the next refresh would otherwise use a refresh token
        // Intacct may have already invalidated server-side.
        let rotated_refresh = token.refresh_token.clone().unwrap_or(current_refresh);
        self.store.set(
            &self.alias,
            &AccountSecrets::AuthCode {
                client_id: client_id.clone(),
                client_secret: client_secret.clone(),
                refresh_token: Some(rotated_refresh),
            },
        )?;

        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
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

impl TokenProvider for AuthCodeProvider {
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
            self.refresh_access_token().await
        })
    }

    fn invalidate(&self) {
        let _ = self.store.delete_token(&self.alias);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_reference_vector() {
        let challenge = challenge_for_verifier("zWiRBuiUcmnIbPBCoNqA5cWDFuaEZwZ7jLJJGJ4P3NQ");
        assert_eq!(challenge, "manPXpjp75EFeofO-YWtY3gIPp0S6_CJ-ciRJyYIonw");
    }

    #[test]
    fn generated_pkce_pair_is_well_formed() {
        let (verifier, challenge) = pkce_pair();
        assert_eq!(verifier.len(), 64);
        assert!(verifier.chars().all(|ch| ch.is_ascii_alphanumeric()));
        assert_eq!(challenge, challenge_for_verifier(&verifier));
    }

    #[test]
    fn authorize_url_contains_all_oauth_params() {
        let url_string = build_authorize_url(
            "https://api.intacct.com/ia/api/v1/oauth2/authorize",
            "cid.app.sage.com",
            "https://localhost:8899/callback",
            "deadbeefdeadbeefdeadbeefdeadbeef",
            "CHALLENGE",
        );
        let parsed = url::Url::parse(&url_string).expect("valid url");
        let params: std::collections::HashMap<String, String> =
            parsed.query_pairs().into_owned().collect();
        assert_eq!(
            params.len(),
            7,
            "expected exactly 7 query params: {params:?}"
        );
        assert_eq!(
            params.get("response_type").map(String::as_str),
            Some("code")
        );
        assert_eq!(
            params.get("client_id").map(String::as_str),
            Some("cid.app.sage.com")
        );
        assert_eq!(
            params.get("redirect_uri").map(String::as_str),
            Some("https://localhost:8899/callback")
        );
        assert_eq!(
            params.get("state").map(String::as_str),
            Some("deadbeefdeadbeefdeadbeefdeadbeef")
        );
        assert_eq!(
            params.get("scope").map(String::as_str),
            Some("offline_access")
        );
        assert_eq!(
            params.get("code_challenge").map(String::as_str),
            Some("CHALLENGE")
        );
        assert_eq!(
            params.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
    }

    #[test]
    fn callback_query_parsing_extracts_code_and_validates_state() {
        let parsed = parse_callback_query("code=abc123&state=EXPECTED", "EXPECTED").unwrap();
        assert_eq!(parsed, "abc123");
        assert!(parse_callback_query("code=abc&state=WRONG", "EXPECTED").is_err());
        assert!(parse_callback_query("error=access_denied&state=EXPECTED", "EXPECTED").is_err());
    }
}
