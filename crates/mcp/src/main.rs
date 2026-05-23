use anyhow::Context;
use local_deploy_mcp::{client, tools::LocalDeployMcp};
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let management_address = client::normalize_management_address(
        std::env::var("LOCAL_DEPLOY_MANAGEMENT_ADDRESS")
            .context("LOCAL_DEPLOY_MANAGEMENT_ADDRESS is required")?,
    )?;

    let server = LocalDeployMcp::new(management_address);
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
