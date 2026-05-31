use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "flovenet", about = "Flovenet - Red social descentralizada")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Inicia el proceso daemon del nodo
    Daemon {
        /// Puerto libp2p
        #[arg(long, default_value = "0")]
        port: u16,
        /// Puerto HTTP para metrics/API
        #[arg(long, default_value_t = 9090)]
        api_port: u16,
        /// Roles del nodo (compute, storage, validation, ai, social)
        #[arg(long, default_value_t = String::new())]
        roles: String,
        /// Swarm key file (PSK) for private sub-network
        #[arg(long)]
        swarm_key: Option<String>,
    },
    /// Inicia el gateway GraphQL
    ApiGateway {
        /// Puerto HTTP
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
    /// Comparte recursos locales
    Share {
        #[arg(long)]
        role: Option<String>,
    },
    /// Ejecuta un WASM localmente
    Run {
        /// Nombre de la función entrypoint (_start, run, etc.)
        #[arg(long, default_value = "_start")]
        manifest: String,
        /// CID o path del WASM image
        #[arg(long)]
        image: Option<String>,
    },
    /// Muestra estado del nodo
    Status,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_cli_daemon_defaults() {
        let cli = Cli::parse_from(["flovenet", "daemon"]);
        match cli.command {
            Commands::Daemon {
                port,
                api_port,
                roles,
                swarm_key,
            } => {
                assert_eq!(port, 0);
                assert_eq!(api_port, 9090);
                assert!(roles.is_empty());
                assert!(swarm_key.is_none());
            }
            _ => panic!("expected Daemon command"),
        }
    }

    #[test]
    fn test_cli_daemon_with_args() {
        let cli = Cli::parse_from([
            "flovenet",
            "daemon",
            "--port",
            "9091",
            "--api-port",
            "9092",
            "--roles",
            "compute,storage",
            "--swarm-key",
            "key.bin",
        ]);
        match cli.command {
            Commands::Daemon {
                port,
                api_port,
                roles,
                swarm_key,
            } => {
                assert_eq!(port, 9091);
                assert_eq!(api_port, 9092);
                assert_eq!(roles, "compute,storage");
                assert_eq!(swarm_key, Some("key.bin".into()));
            }
            _ => panic!("expected Daemon command"),
        }
    }

    #[test]
    fn test_cli_api_gateway() {
        let cli = Cli::parse_from(["flovenet", "api-gateway", "--port", "8080"]);
        match cli.command {
            Commands::ApiGateway { port } => assert_eq!(port, 8080),
            _ => panic!("expected ApiGateway command"),
        }
    }

    #[test]
    fn test_cli_api_gateway_default() {
        let cli = Cli::parse_from(["flovenet", "api-gateway"]);
        match cli.command {
            Commands::ApiGateway { port } => assert_eq!(port, 8080),
            _ => panic!("expected ApiGateway command"),
        }
    }

    #[test]
    fn test_cli_share_default() {
        let cli = Cli::parse_from(["flovenet", "share"]);
        match cli.command {
            Commands::Share { role } => assert!(role.is_none()),
            _ => panic!("expected Share command"),
        }
    }

    #[test]
    fn test_cli_share_with_role() {
        let cli = Cli::parse_from(["flovenet", "share", "--role", "gpu"]);
        match cli.command {
            Commands::Share { role } => assert_eq!(role, Some("gpu".into())),
            _ => panic!("expected Share command"),
        }
    }

    #[test]
    fn test_cli_run_defaults() {
        let cli = Cli::parse_from(["flovenet", "run"]);
        match cli.command {
            Commands::Run { manifest, image } => {
                assert_eq!(manifest, "_start");
                assert!(image.is_none());
            }
            _ => panic!("expected Run command"),
        }
    }

    #[test]
    fn test_cli_run_with_image() {
        let cli = Cli::parse_from([
            "flovenet",
            "run",
            "--manifest",
            "run",
            "--image",
            "feed_ranker.wasm",
        ]);
        match cli.command {
            Commands::Run { manifest, image } => {
                assert_eq!(manifest, "run");
                assert_eq!(image, Some("feed_ranker.wasm".into()));
            }
            _ => panic!("expected Run command"),
        }
    }

    #[test]
    fn test_cli_status() {
        let cli = Cli::parse_from(["flovenet", "status"]);
        assert!(matches!(cli.command, Commands::Status));
    }
}
