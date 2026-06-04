use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    animus_notifier_http::run().await
}
