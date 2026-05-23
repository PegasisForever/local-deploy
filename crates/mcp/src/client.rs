use anyhow::{anyhow, Context, Result};
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde::Deserialize;

#[derive(Clone)]
pub struct ManagementClient {
    base_url: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct AllocateResponse {
    uuid: String,
}

#[derive(Deserialize)]
pub struct UploadResponse {
    pub url: String,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

impl ManagementClient {
    pub fn new(base_url: String) -> Self {
        Self {
            base_url,
            http: reqwest::Client::new(),
        }
    }

    pub async fn allocate(&self) -> Result<String> {
        let response = self
            .http
            .post(format!("{}/allocate", self.base_url))
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .context("failed to call POST /allocate")?;

        parse_response::<AllocateResponse>(response)
            .await
            .map(|parsed| parsed.uuid)
    }

    pub async fn upload(&self, uuid: &str, body: Vec<u8>) -> Result<UploadResponse> {
        let response = self
            .http
            .put(format!("{}/upload/{}", self.base_url, uuid))
            .header("Content-Type", "application/gzip")
            .body(body)
            .send()
            .await
            .context("failed to call PUT /upload/{uuid}")?;

        parse_response(response).await
    }
}

pub fn normalize_management_address(raw: String) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Err(anyhow!("LOCAL_DEPLOY_MANAGEMENT_ADDRESS must not be empty"));
    }
    Ok(trimmed)
}

async fn parse_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read management API response body")?;

    if status.is_success() {
        return serde_json::from_str(&body).with_context(|| {
            format!("failed to parse successful response (HTTP {status}): {body}")
        });
    }

    let message = serde_json::from_str::<ErrorResponse>(&body)
        .map(|parsed| parsed.error)
        .unwrap_or(body);

    Err(anyhow!("HTTP {}: {}", status.as_u16(), message))
        .with_context(|| format_http_context(status))
}

fn format_http_context(status: StatusCode) -> String {
    format!("management API returned HTTP {}", status.as_u16())
}

pub fn format_http_error(error: &anyhow::Error) -> String {
    error.to_string()
}
