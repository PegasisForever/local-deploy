use anyhow::{anyhow, Context, Result};
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path as AxumPath, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use flate2::read::GzDecoder;
use serde::Serialize;
use std::{
    fs,
    io::Read,
    path::{Component, Path as StdPath, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tar::Archive;
use tokio::process::Command;
use tracing::{error, info};
use uuid::Uuid;

const DATA_DIR: &str = "/data";
const NGINX_CONF_PATH: &str = "/etc/nginx/nginx.conf";
const NGINX_PORT: u16 = 11000;
const MANAGEMENT_PORT: u16 = 11001;

#[derive(Clone)]
struct AppState {
    public_address: String,
    data_dir: PathBuf,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct AllocateResponse {
    uuid: String,
}

#[derive(Serialize)]
struct UploadResponse {
    files: usize,
    url: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let public_address = normalize_address(
        std::env::var("LOCAL_DEPLOY_PUBLIC_ADDRESS")
            .context("LOCAL_DEPLOY_PUBLIC_ADDRESS is required")?,
    )?;

    let data_dir = PathBuf::from(DATA_DIR);
    fs::create_dir_all(&data_dir).context("failed to create /data directory")?;

    write_nginx_config(NGINX_CONF_PATH)?;
    start_nginx().await?;

    let state = Arc::new(AppState {
        public_address,
        data_dir,
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/allocate", post(allocate))
        .route(
            "/upload/{uuid}",
            put(upload).layer(DefaultBodyLimit::max(256 * 1024 * 1024)),
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{MANAGEMENT_PORT}"))
        .await
        .context("failed to bind management API")?;

    info!("management API listening on 0.0.0.0:{MANAGEMENT_PORT}");
    axum::serve(listener, app)
        .await
        .context("management API server failed")?;

    Ok(())
}

fn normalize_address(raw: String) -> Result<String> {
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return Err(anyhow!("LOCAL_DEPLOY_PUBLIC_ADDRESS must not be empty"));
    }
    if !trimmed.starts_with("http://") && !trimmed.starts_with("https://") {
        return Err(anyhow!(
            "LOCAL_DEPLOY_PUBLIC_ADDRESS must include http:// or https://"
        ));
    }
    Ok(trimmed)
}

fn write_nginx_config(path: &str) -> Result<()> {
    let config = r#"user www-data;
worker_processes auto;
pid /run/nginx.pid;
error_log /var/log/nginx/error.log warn;

events {
    worker_connections 1024;
}

http {
    include /etc/nginx/mime.types;
    default_type application/octet-stream;

    sendfile on;
    tcp_nopush on;
    tcp_nodelay on;
    keepalive_timeout 65;

    gzip on;
    gzip_vary on;
    gzip_proxied any;
    gzip_comp_level 6;
    gzip_types text/plain text/css application/json application/javascript text/xml application/xml application/xml+rss text/javascript image/svg+xml;

    open_file_cache max=1000 inactive=20s;
    open_file_cache_valid 30s;
    open_file_cache_min_uses 2;
    open_file_cache_errors on;

    server {
        listen 11000;
        server_name _;
        index index.html;

        location ~ "^/([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})$" {
            return 301 /$1/;
        }

        location ~ "^/([0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12})(/.*)?$" {
            root /data;
            try_files $uri $uri/ /$1/index.html;

            add_header Access-Control-Allow-Origin * always;
            add_header Access-Control-Allow-Methods "GET, HEAD, OPTIONS" always;
            add_header Access-Control-Allow-Headers * always;

            if ($request_method = OPTIONS) {
                return 204;
            }
        }

        location ~* "^/[0-9a-f-]{36}/index\.html$" {
            root /data;
            add_header Cache-Control "no-cache";
        }
    }
}
"#;

    if let Some(parent) = StdPath::new(path).parent() {
        fs::create_dir_all(parent).context("failed to create nginx config directory")?;
    }
    fs::write(path, config).context("failed to write nginx config")?;
    Ok(())
}

async fn start_nginx() -> Result<()> {
    let status = Command::new("nginx")
        .arg("-t")
        .arg("-c")
        .arg(NGINX_CONF_PATH)
        .status()
        .await
        .context("failed to test nginx config")?;

    if !status.success() {
        return Err(anyhow!("nginx config test failed"));
    }

    tokio::spawn(async {
        let result = Command::new("nginx")
            .arg("-c")
            .arg(NGINX_CONF_PATH)
            .arg("-g")
            .arg("daemon off;")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .await;

        if let Err(error) = result {
            error!("nginx process failed: {error}");
        }
    });

    info!("nginx started on port {NGINX_PORT}");
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn allocate(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let uuid = Uuid::new_v4();
    let uuid_str = uuid.to_string();
    let target = state.data_dir.join(&uuid_str);

    fs::create_dir_all(&target).map_err(|e| {
        error!("allocate failed for {uuid_str}: {e}");
        AppError::internal("failed to create allocation directory")
    })?;

    Ok((
        StatusCode::CREATED,
        Json(AllocateResponse {
            uuid: uuid_str,
        }),
    )
        .into_response())
}

async fn upload(
    State(state): State<Arc<AppState>>,
    AxumPath(uuid): AxumPath<String>,
    body: Bytes,
) -> Result<Response, AppError> {
    if Uuid::parse_str(&uuid).is_err() {
        return Err(AppError::not_found("UUID not allocated"));
    }

    let target = state.data_dir.join(&uuid);
    if !target.is_dir() {
        return Err(AppError::not_found("UUID not allocated"));
    }

    if !directory_is_empty(&target).map_err(|_| AppError::internal("failed to read allocation directory"))? {
        return Err(AppError::conflict("UUID already has uploaded content"));
    }

    let file_count = extract_tar_gz(&body, &target).map_err(|e| {
        error!("upload failed for {uuid}: {e:#}");
        if e.to_string().contains("path traversal") {
            AppError::bad_request(e.to_string())
        } else {
            AppError::internal("failed to extract upload archive")
        }
    })?;

    Ok((
        StatusCode::OK,
        Json(UploadResponse {
            files: file_count,
            url: format!("{}/{}", state.public_address, uuid),
        }),
    )
        .into_response())
}

fn directory_is_empty(path: &StdPath) -> Result<bool> {
    let mut entries = fs::read_dir(path).context("failed to read allocation directory")?;
    Ok(entries.next().transpose()?.is_none())
}

fn extract_tar_gz(data: &[u8], dest: &StdPath) -> Result<usize> {
    let decoder = GzDecoder::new(data);
    let mut archive = Archive::new(decoder);
    let mut file_count = 0usize;

    for entry in archive.entries().context("invalid tar.gz archive")? {
        let mut entry = entry.context("invalid tar entry")?;
        let entry_path = entry.path().context("invalid tar entry path")?;
        let safe_path = sanitize_tar_path(&entry_path)?;

        let out_path = dest.join(&safe_path);
        if !out_path.starts_with(dest) {
            return Err(anyhow!("path traversal detected"));
        }

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&out_path).context("failed to create directory from archive")?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).context("failed to create parent directory")?;
            }
            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .context("failed to read tar entry")?;
            fs::write(&out_path, contents).context("failed to write extracted file")?;
            file_count += 1;
        }
    }

    Ok(file_count)
}

fn sanitize_tar_path(path: &StdPath) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                return Err(anyhow!("path traversal detected"));
            }
            Component::ParentDir => {
                return Err(anyhow!("path traversal detected"));
            }
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
        }
    }
    Ok(normalized)
}

enum AppError {
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Internal(String),
}

impl AppError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            AppError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            AppError::Conflict(message) => (StatusCode::CONFLICT, message),
            AppError::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };

        (
            status,
            [(header::CONTENT_TYPE, "application/json")],
            Json(ErrorResponse { error: message }),
        )
            .into_response()
    }
}
