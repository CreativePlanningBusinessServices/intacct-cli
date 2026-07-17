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

        let mut reauthorized = false;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.tokens.access_token().await?;
            let mut request = self
                .http
                .request(method.clone(), &url)
                .bearer_auth(&token)
                .query(query);
            if let Some(entity) = &self.entity {
                request = request.header("X-IA-API-Param-Entity", entity);
            }
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            if let Some(json_body) = body {
                request = request.json(json_body);
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
            return Err(api_error(status.as_u16(), parsed, raw));
        }
    }

    pub async fn request_multipart(
        &self,
        path: &str,
        build_form: &(dyn Fn() -> reqwest::multipart::Form + Send + Sync),
    ) -> Result<IaResponse, CliError> {
        let url = resolve_url(&self.base, path);

        let mut reauthorized = false;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let token = self.tokens.access_token().await?;
            let mut request = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .multipart(build_form());
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
            return Err(api_error(status.as_u16(), parsed, raw));
        }
    }
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
