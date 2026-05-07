use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    kria_core::test_runner::run_from_cli().await
}
