mod client;
mod key;
mod parse;
mod proxy;
mod server;
mod stdio;
mod udp;

use anyhow::{Context, Result};
use clap::Parser;
use iroh::EndpointId;

#[derive(Parser)]
#[command(about = "Peer-to-peer TCP and UDP port forwarding over iroh")]
enum Cli {
    /// Expose local ports to remote peers
    Out {
        /// Ports to expose (e.g. 8080 53/udp)
        #[arg(required = true)]
        ports: Vec<String>,
    },
    /// Connect to a remote peer
    In {
        /// Remote peer's endpoint ID (base32)
        pubkey: String,
        /// Mappings (e.g. 4000:8080 5300:53/udp -:22)
        #[arg(required = true, allow_hyphen_values = true)]
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

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn cli_accepts_stdio_mapping_without_double_dash() {
        let cli = Cli::try_parse_from(["punch", "in", "peer", "-:22"]).unwrap();
        match cli {
            Cli::In { mappings, .. } => assert_eq!(mappings, vec!["-:22"]),
            _ => panic!("expected in subcommand"),
        }
    }

    #[test]
    fn cli_accepts_mixed_mappings_with_stdio() {
        let cli = Cli::try_parse_from(["punch", "in", "peer", "-:22", "3000:8080", "5300:53/udp"])
            .unwrap();
        match cli {
            Cli::In { mappings, .. } => {
                assert_eq!(mappings, vec!["-:22", "3000:8080", "5300:53/udp"])
            }
            _ => panic!("expected in subcommand"),
        }
    }

    #[test]
    fn cli_still_accepts_stdio_mapping_after_double_dash() {
        let cli = Cli::try_parse_from(["punch", "in", "peer", "--", "-:22"]).unwrap();
        match cli {
            Cli::In { mappings, .. } => assert_eq!(mappings, vec!["-:22"]),
            _ => panic!("expected in subcommand"),
        }
    }
}
