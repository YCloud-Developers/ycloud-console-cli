use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    ycloud_dashboard_cli::run().await
}
