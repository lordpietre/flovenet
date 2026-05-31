mod scenarios;

use std::fs;
use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use test_harness::{Scenario, ScenarioResult, TestOrchestrator, TestReport};

#[derive(Parser)]
#[command(name = "flovenet-test", about = "Flovenet inter-node test harness")]
struct Cli {
    /// Test scenarios to run (comma-separated, or "all")
    #[arg(short, long, default_value = "all")]
    scenarios: String,

    /// Output file for JSON report
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Number of P2P nodes to create (for mesh test)
    #[arg(long, default_value = "3")]
    mesh_nodes: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let requested: Vec<&str> = if cli.scenarios == "all" {
        vec!["p2p_mesh", "gossip_propagation", "sequential_messages"]
    } else {
        cli.scenarios.split(',').map(|s| s.trim()).collect()
    };

    let orch = TestOrchestrator::new();
    let mut results = Vec::new();

    for name in &requested {
        let result: ScenarioResult = match *name {
            "p2p_mesh" => {
                let s = scenarios::P2pMeshScenario {
                    node_count: cli.mesh_nodes,
                };
                s.run(&orch).await
            }
            "gossip_propagation" => {
                let s = scenarios::GossipPropagationScenario {
                    topic: "test/harness".into(),
                    message: "hello from flovenet-test".into(),
                };
                s.run(&orch).await
            }
            "sequential_messages" => {
                let s = scenarios::SequentialMessagesScenario;
                s.run(&orch).await
            }
            other => {
                tracing::warn!("Unknown scenario: {other}");
                continue;
            }
        };

        let status = if result.passed { "✅" } else { "❌" };
        println!("{status} {} ({:?})", result.name, result.duration_ms);
        for check in &result.checks {
            let ck = if check.passed { "  ✅" } else { "  ❌" };
            println!("  {ck} {}", check.name);
        }
        results.push(result);
    }

    let total_duration: u64 = results.iter().map(|r| r.duration_ms).sum();
    let report = TestReport::from_scenarios(results, total_duration);
    println!("\n{}", report.summary());

    if let Some(path) = cli.output {
        let json = report.to_json();
        fs::write(&path, &json)?;
        println!("Report saved to {}", path.display());
    }

    if report.failed_scenarios > 0 {
        std::process::exit(1);
    }
    Ok(())
}
