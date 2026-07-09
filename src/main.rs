use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    yc_cli::run().await
}
