mod config;
mod handler;
mod metrics;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().json().init();
    tracing::info!("Aegis-SVM starting");
}
