//! Ephemera desktop client entry point.
//!
//! Starts the embedded node, serves the frontend on localhost,
//! and opens the default browser to the application.

use ephemera_client::state::AppState;
use ephemera_config::NodeConfig;
use ephemera_node::debug_log::{DebugLogHandle, DebugLogLayer};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The default port for the local HTTP server.
const DEFAULT_PORT: u16 = 3500;

#[tokio::main]
async fn main() {
    // Create the shared debug log handle BEFORE initializing tracing so
    // every log line from startup onwards is captured for the debug console.
    let debug_log = DebugLogHandle::new();

    // Initialize tracing with BOTH the fmt layer (console output) and the
    // DebugLogLayer (in-app ring buffer).
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(DebugLogLayer::new(debug_log.clone()))
        .init();

    tracing::info!("Ephemera v{}", env!("CARGO_PKG_VERSION"));

    // Determine data directory: prefer EPHEMERA_DATA_DIR env var, then
    // platform default, then fallback to .ephemera in cwd.
    let data_dir = std::env::var("EPHEMERA_DATA_DIR")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(NodeConfig::default_data_dir)
        .unwrap_or_else(|| {
            let fallback = std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(".ephemera");
            tracing::warn!(
                path = %fallback.display(),
                "could not determine platform data directory, using fallback"
            );
            fallback
        });

    tracing::info!(data_dir = %data_dir.display(), "using data directory");

    // Initialize application state (boots the embedded node)
    let state = match AppState::initialize(data_dir, debug_log).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!(error = %e, "failed to initialize ephemera node");
            eprintln!("Error: failed to start Ephemera node: {e}");
            eprintln!("Check that the data directory is writable and try again.");
            std::process::exit(1);
        }
    };

    // Find an available port starting from DEFAULT_PORT
    let addr = find_available_port(DEFAULT_PORT).await;
    let url = format!("http://localhost:{}", addr.port());

    tracing::info!(%url, "starting Ephemera client");
    println!();
    println!("  ╔══════════════════════════════════════╗");
    println!("  ║          Ephemera is running          ║");
    println!("  ║                                       ║");
    println!("  ║  Open: {:<29} ║", url);
    println!("  ║                                       ║");
    println!("  ║  Press Ctrl+C to stop                 ║");
    println!("  ╚══════════════════════════════════════╝");
    println!();

    // Open the browser (skip in headless/Docker environments)
    if std::env::var("EPHEMERA_NO_BROWSER").is_err() {
        if let Err(e) = open::that(&url) {
            tracing::warn!(error = %e, "could not open browser automatically");
            println!("Could not open browser. Please navigate to: {url}");
        }
    }

    // Start the HTTP server (blocks until shutdown)
    if let Err(e) = ephemera_client::server::start_server(state, addr).await {
        tracing::error!(error = %e, "server error");
        eprintln!("Server error: {e}");
        std::process::exit(1);
    }
}

/// Try to find an available port starting from `preferred`.
/// Falls back to letting the OS assign one if the preferred range is taken.
///
/// Respects `EPHEMERA_HTTP_ADDR` env var to override the bind IP address
/// (default `127.0.0.1`). Set to `0.0.0.0` for Docker containers.
async fn find_available_port(preferred: u16) -> SocketAddr {
    let bind_ip: std::net::IpAddr = std::env::var("EPHEMERA_HTTP_ADDR")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));

    for port in preferred..preferred + 100 {
        let addr = SocketAddr::new(bind_ip, port);
        if tokio::net::TcpListener::bind(addr).await.is_ok() {
            return addr;
        }
    }

    // Let the OS pick
    tracing::warn!(
        "could not bind to ports {preferred}-{}, letting OS choose",
        preferred + 99
    );
    let addr = SocketAddr::new(bind_ip, 0);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind to any port");
    listener.local_addr().expect("failed to get local address")
}
