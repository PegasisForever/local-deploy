use std::{env, path::PathBuf, process::Stdio};

use anyhow::{Context, Result};
use local_deploy_mcp::client;
use rmcp::{
    model::CallToolRequestParams,
    service::ServiceExt,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use tokio::process::Command;

#[tokio::main]
async fn main() -> Result<()> {
    let dist_dir = env::args()
        .nth(1)
        .context("usage: local-deploy-mcp-e2e <dist_dir>")?;

    let management_address = client::normalize_management_address(
        env::var("LOCAL_DEPLOY_MANAGEMENT_ADDRESS")
            .context("LOCAL_DEPLOY_MANAGEMENT_ADDRESS is required")?,
    )?;

    env::set_var("LOCAL_DEPLOY_MANAGEMENT_ADDRESS", &management_address);

    let mcp_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/release/local-deploy-mcp");
    let service = ()
        .serve(TokioChildProcess::new(
            Command::new(&mcp_bin).configure(|cmd| {
                cmd.stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit())
                    .env(
                        "LOCAL_DEPLOY_MANAGEMENT_ADDRESS",
                        &management_address,
                    );
            }),
        )?)
        .await
        .context("failed to start local-deploy-mcp")?;

    let allocate = service
        .call_tool(CallToolRequestParams::new("allocate_subpath"))
        .await
        .context("allocate_subpath failed")?;
    let uuid = allocate
        .content
        .first()
        .and_then(|content| content.raw.as_text())
        .map(|text| text.text.clone())
        .context("allocate_subpath returned no text")?;

    let upload = service
        .call_tool(
            CallToolRequestParams::new("upload_website").with_arguments(
                serde_json::json!({
                    "uuid": uuid,
                    "local_folder": dist_dir,
                })
                .as_object()
                .cloned()
                .context("failed to build upload_website arguments")?,
            ),
        )
        .await
        .context("upload_website failed")?;
    let url = upload
        .content
        .first()
        .and_then(|content| content.raw.as_text())
        .map(|text| text.text.clone())
        .context("upload_website returned no text")?;

    println!("{uuid}");
    println!("{url}");
    Ok(())
}
