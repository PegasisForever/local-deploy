use std::{fs, path::Path};

use anyhow::{anyhow, Context, Result};
use flate2::{write::GzEncoder, Compression};
use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    ErrorData as McpError, ServerHandler,
};
use serde::Deserialize;
use tar::Builder;
use walkdir::WalkDir;

use crate::client::{format_http_error, ManagementClient};

#[derive(Clone)]
pub struct LocalDeployMcp {
    client: ManagementClient,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UploadWebsiteRequest {
    #[schemars(description = "UUID returned by allocate_subpath")]
    uuid: String,
    #[schemars(description = "Path to the local folder to upload (absolute or relative)")]
    local_folder: String,
}

#[tool_router]
impl LocalDeployMcp {
    pub fn new(management_address: String) -> Self {
        Self {
            client: ManagementClient::new(management_address),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Allocate a new UUID subpath for a static website deployment")]
    async fn allocate_subpath(&self) -> Result<String, McpError> {
        self.client
            .allocate()
            .await
            .map_err(|error| McpError::internal_error(format_http_error(&error), None))
    }

    #[tool(description = "Upload a local folder as a static website to an allocated UUID subpath")]
    async fn upload_website(
        &self,
        Parameters(UploadWebsiteRequest { uuid, local_folder }): Parameters<UploadWebsiteRequest>,
    ) -> Result<String, McpError> {
        let folder = Path::new(&local_folder);
        if !folder.exists() {
            return Err(McpError::invalid_params(
                format!("local_folder does not exist: {local_folder}"),
                None,
            ));
        }
        if !folder.is_dir() {
            return Err(McpError::invalid_params(
                format!("local_folder is not a directory: {local_folder}"),
                None,
            ));
        }

        let archive = create_tar_gz(folder).map_err(|error| {
            McpError::internal_error(format!("failed to create tar.gz: {error:#}"), None)
        })?;

        self.client
            .upload(&uuid, archive)
            .await
            .map(|response| response.url)
            .map_err(|error| McpError::internal_error(format_http_error(&error), None))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for LocalDeployMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Tools for allocating UUID subpaths and uploading static websites to local-deploy.",
        )
    }
}

fn create_tar_gz(folder: &Path) -> Result<Vec<u8>> {
    let encoder = GzEncoder::new(Vec::new(), Compression::default());
    let mut builder = Builder::new(encoder);

    for entry in WalkDir::new(folder).into_iter().filter_map(Result::ok) {
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let relative = path
            .strip_prefix(folder)
            .context("failed to compute relative path for archive entry")?;

        if relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        }) {
            return Err(anyhow!(
                "invalid path in local folder: {}",
                relative.display()
            ));
        }

        let mut file = fs::File::open(path)
            .with_context(|| format!("failed to open file for archiving: {}", path.display()))?;
        builder
            .append_file(relative, &mut file)
            .with_context(|| format!("failed to append file to archive: {}", path.display()))?;
    }

    let encoder = builder
        .into_inner()
        .context("failed to finalize tar archive")?;
    encoder
        .finish()
        .context("failed to finalize gzip archive")
}
