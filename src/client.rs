use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::auth::TokenProvider;
use crate::error::CliError;

const MAX_ATTEMPTS: u32 = 4;
const BASE_BACKOFF_MILLIS: u64 = 500;

pub struct IaClient {
    http: reqwest::Client,
    base: String,
    tokens: Arc<dyn TokenProvider>,
    entity: Option<String>,
}

#[derive(Debug)]
pub struct IaResponse {
    pub status: u16,
    pub body: Option<Value>,
    pub location: Option<String>,
}

/// For endpoints whose success body is a file (export, report download), not JSON.
#[derive(Debug)]
pub struct BinaryResponse {
    pub status: u16,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
}

impl IaClient {
    pub fn new(
        http: reqwest::Client,
        base: String,
        tokens: Arc<dyn TokenProvider>,
        entity: Option<String>,
    ) -> IaClient {
        IaClient {
            http,
            base: base.trim_end_matches('/').to_string(),
            tokens,
            entity,
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        headers: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<IaResponse, CliError> {
        let url = resolve_url(&self.base, path);
        let build_request = || {
            let mut request = self.http.request(method.clone(), &url).query(query);
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            if let Some(json_body) = body {
                request = request.json(json_body);
            }
            request
        };
        self.execute_with_retry(&url, &build_request).await
    }

    pub async fn request_multipart(
        &self,
        path: &str,
        build_form: &(dyn Fn() -> reqwest::multipart::Form + Send + Sync),
    ) -> Result<IaResponse, CliError> {
        let url = resolve_url(&self.base, path);
        let build_request = || self.http.post(&url).multipart(build_form());
        let response = self.send_with_retry(&url, &build_request).await?;
        parse_json_response(&url, response).await
    }

    /// For endpoints whose success body is a file (export, report download). Shares the
    /// same auth/retry loop as `request`; only the success-path body handling differs
    /// (raw bytes instead of parsed JSON). On non-2xx the body is still read as text and
    /// parsed as JSON, mapped through the same `api_error()` as `request`.
    pub async fn request_binary(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<BinaryResponse, CliError> {
        let url = resolve_url(&self.base, path);
        let build_request = || {
            let mut request = self.http.request(method.clone(), &url).query(query);
            if let Some(json_body) = body {
                request = request.json(json_body);
            }
            request
        };
        let response = self.send_with_retry(&url, &build_request).await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        if status.is_success() {
            let bytes = response.bytes().await.map_err(|read_error| {
                CliError::Network(format!("reading response from {url} failed: {read_error}"))
            })?;
            return Ok(BinaryResponse {
                status: status.as_u16(),
                bytes: bytes.to_vec(),
                content_type,
            });
        }

        let raw = response.text().await.map_err(|read_error| {
            CliError::Network(format!("reading response from {url} failed: {read_error}"))
        })?;
        let parsed: Option<Value> = if raw.trim().is_empty() {
            None
        } else {
            serde_json::from_str(&raw).ok()
        };
        Err(api_error(status.as_u16(), parsed, raw))
    }

    /// Owns the shared per-attempt control flow: 401 invalidate-and-retry-once,
    /// 429/5xx Retry-After/backoff. `build_request` produces a fresh, unauthenticated
    /// `RequestBuilder` for each attempt (method/url/query/body already applied); this
    /// helper adds bearer auth and the entity header before sending. Returns the final
    /// `reqwest::Response` (success or exhausted-retries error) for the caller to read the
    /// body from — as JSON (`request`/`request_multipart`) or raw bytes (`request_binary`).
    async fn execute_with_retry(
        &self,
        url: &str,
        build_request: &(dyn Fn() -> reqwest::RequestBuilder + Send + Sync),
    ) -> Result<IaResponse, CliError> {
        let response = self.send_with_retry(url, build_request).await?;
        parse_json_response(url, response).await
    }

    async fn send_with_retry(
        &self,
        url: &str,
        build_request: &(dyn Fn() -> reqwest::RequestBuilder + Send + Sync),
    ) -> Result<reqwest::Response, CliError> {
        let mut reauthorized = false;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.tokens.access_token().await?;
            let mut request = build_request().bearer_auth(&token);
            if let Some(entity) = &self.entity {
                request = request.header("X-IA-API-Param-Entity", entity);
            }

            let response = request.send().await.map_err(|send_error| {
                CliError::Network(format!("request to {url} failed: {send_error}"))
            })?;
            let status = response.status();

            if status.as_u16() == 401 && !reauthorized && attempt < MAX_ATTEMPTS {
                self.tokens.invalidate();
                reauthorized = true;
                continue;
            }
            if (status.as_u16() == 429 || status.is_server_error()) && attempt < MAX_ATTEMPTS {
                let delay = retry_after_seconds(&response)
                    .map(Duration::from_secs)
                    .unwrap_or_else(|| {
                        Duration::from_millis(BASE_BACKOFF_MILLIS * 2u64.pow(attempt - 1))
                    });
                tokio::time::sleep(delay).await;
                continue;
            }

            return Ok(response);
        }
    }
}

async fn parse_json_response(
    url: &str,
    response: reqwest::Response,
) -> Result<IaResponse, CliError> {
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let raw = response.text().await.map_err(|read_error| {
        CliError::Network(format!("reading response from {url} failed: {read_error}"))
    })?;
    let parsed: Option<Value> = if raw.trim().is_empty() {
        None
    } else {
        serde_json::from_str(&raw).ok()
    };

    if status.is_success() {
        return Ok(IaResponse {
            status: status.as_u16(),
            body: parsed,
            location,
        });
    }
    Err(api_error(status.as_u16(), parsed, raw))
}

fn resolve_url(base: &str, path: &str) -> String {
    if path.starts_with("https://") || path.starts_with("http://") {
        path.to_string()
    } else if path.starts_with('/') {
        format!("{base}{path}")
    } else {
        format!("{base}/{path}")
    }
}

fn retry_after_seconds(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get("retry-after")?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

fn api_error(status: u16, parsed: Option<Value>, raw: String) -> CliError {
    let Some(body) = parsed else {
        return CliError::Api {
            status,
            message: raw,
            details: vec![],
            support_id: None,
        };
    };
    let error = &body["ia::error"];
    if error.is_null() {
        return CliError::Api {
            status,
            message: raw,
            details: vec![],
            support_id: None,
        };
    }
    let code = error["code"].as_str().unwrap_or("");
    let text = error["message"].as_str().unwrap_or("request failed");
    let message = if code.is_empty() {
        text.to_string()
    } else {
        format!("{code}: {text}")
    };
    let details = error["details"].as_array().cloned().unwrap_or_default();
    let support_id = error["supportId"].as_str().map(str::to_string);
    CliError::Api {
        status,
        message,
        details,
        support_id,
    }
}
