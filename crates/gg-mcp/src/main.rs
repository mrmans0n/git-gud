//! gg-mcp: MCP server for git-gud (gg) stacked-diffs CLI tool.
//!
//! Exposes git-gud operations as MCP tools for AI assistants.

mod tools;

use rmcp::{transport::stdio, ServiceExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = tools::GgMcpServer::new();
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
