mod client;
mod key;
mod parse;
mod proxy;
mod server;

use anyhow::{Context, Result};
use clap::Parser;
use iroh::EndpointId;

#[derive(Parser)]
#[command(about = "Peer-to-peer TCP and UDP port forwarding over iroh")]
enum Cli {
    /// Expose local ports to remote peers
    Out {
        /// Ports to expose (e.g. 8080 22)
        #[arg(required = true)]
        ports: Vec<String>,
    },
    /// Connect to a remote peer
    In {
        /// Remote peer's endpoint ID (base32)
        pubkey: String,
        /// Mappings (e.g. 4000:8080)
        #[arg(required = true)]
        mappings: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse() {
        Cli::Out { ports } => {
            let ports = parse::parse_ports(&ports)?;
            let secret_key = key::load_or_generate()?;
            server::run(ports, secret_key).await
        }
        Cli::In { pubkey, mappings } => {
            let endpoint_id: EndpointId = pubkey.parse().context("invalid endpoint ID")?;
            let mappings = parse::parse_mappings(&mappings)?;
            let secret_key = key::load_or_generate()?;
            client::run(endpoint_id, mappings, secret_key).await
        }
    }
}
