use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    ycloud_console_cli::run().await
}
