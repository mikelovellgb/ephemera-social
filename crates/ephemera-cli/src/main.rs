//! Ephemera CLI: a thin command-line interface for PoC testing.
//!
//! Provides basic subcommands for creating an identity, posting content,
//! reading the feed, and checking node status. Embeds a full
//! [`ephemera_node::EphemeraNode`] in-process.

use clap::{Parser, Subcommand};
use ephemera_config::NodeConfig;
use ephemera_node::api::build_router_with_network;
use ephemera_node::rpc::JsonRpcRequest;
use ephemera_node::EphemeraNode;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Ephemera: decentralized, anonymous, ephemeral social media.
#[derive(Parser)]
#[command(name = "ephemera", version, about)]
struct Cli {
    /// Path to the data directory.
    #[arg(long, env = "EPHEMERA_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Override the listen address (e.g. "0.0.0.0:9100").
    #[arg(long)]
    listen: Option<String>,

    /// Connect to a peer on startup (e.g. "192.168.1.10:9100").
    #[arg(long)]
    connect: Option<String>,

    /// Bootstrap nodes to connect to on startup (repeatable).
    #[arg(long)]
    bootstrap: Vec<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new identity (first-time setup).
    Init {
        /// Passphrase to protect the keystore.
        #[arg(long)]
        passphrase: String,
    },

    /// Create a text post.
    Post {
        /// The post body text.
        body: String,

        /// TTL in seconds (default: 86400 = 24 hours).
        #[arg(long, default_value = "86400")]
        ttl: u64,
    },

    /// List the feed.
    Feed {
        /// Maximum number of posts to fetch.
        #[arg(long, default_value = "20")]
        limit: u64,
    },

    /// Show node status.
    Status,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing (logs to stderr).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();

    // Resolve data directory.
    let data_dir = cli
        .data_dir
        .or_else(NodeConfig::default_data_dir)
        .unwrap_or_else(|| PathBuf::from("./ephemera-data"));

    // Load or create configuration.
    let mut config = NodeConfig::load_or_create(&data_dir)?;

    // Apply CLI overrides to config.
    if let Some(ref listen) = cli.listen {
        config.listen_addr = Some(listen.parse().map_err(|e| {
            format!("invalid --listen address '{}': {}", listen, e)
        })?);
    }
    if let Some(ref connect_addr) = cli.connect {
        config.bootstrap_nodes.push(connect_addr.clone());
    }
    for addr in &cli.bootstrap {
        if !config.bootstrap_nodes.contains(addr) {
            config.bootstrap_nodes.push(addr.clone());
        }
    }

    // Create and start the embedded node.
    let mut node = EphemeraNode::new(config)?;
    node.start().await?;

    // Build the JSON-RPC router for API calls (with network support).
    let services = Arc::new(
        {
            let cfg = node.config().clone();
            let event_bus = node.event_bus().clone();
            ephemera_node::services::ServiceContainer::new(&cfg, event_bus)?
        },
    );
    let network = node.network().cloned();
    let router = build_router_with_network(services, network, None);

    // Dispatch the CLI command as a JSON-RPC call.
    let response = match cli.command {
        Commands::Init { passphrase } => {
            let req = make_request(
                "identity.create",
                serde_json::json!({ "passphrase": passphrase }),
            );
            router.dispatch(req).await
        }
        Commands::Post { body, ttl } => {
            let req = make_request(
                "posts.create",
                serde_json::json!({
                    "body": body,
                    "ttl_seconds": ttl,
                }),
            );
            router.dispatch(req).await
        }
        Commands::Feed { limit } => {
            let req = make_request("feed.connections", serde_json::json!({ "limit": limit }));
            router.dispatch(req).await
        }
        Commands::Status => {
            let req = make_request("meta.status", serde_json::json!({}));
            router.dispatch(req).await
        }
    };

    // Print the result.
    if let Some(error) = &response.error {
        eprintln!("Error ({}): {}", error.code, error.message);
        std::process::exit(1);
    }

    if let Some(result) = &response.result {
        println!("{}", serde_json::to_string_pretty(result)?);
    }

    // Shut down the node gracefully.
    node.shutdown().await?;
    Ok(())
}

/// Helper to create a JSON-RPC request.
fn make_request(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: method.to_string(),
        params,
        id: Value::Number(1.into()),
    }
}
