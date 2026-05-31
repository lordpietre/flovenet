mod networking;

use std::net::SocketAddr;
use std::sync::LazyLock;

use axum::{routing::get, Router};
use clap::Parser;
use cli::{Cli, Commands};
use prometheus::{register_int_counter, Encoder, IntCounter, TextEncoder};
use reputation_engine::{EventKind, ReputationEvent};
use resource_manager::{NodeDescriptor, NodeResources, NodeRole};
use scheduler::LocalScheduler;
use tower_http::cors::CorsLayer;
use tracing_subscriber::EnvFilter;
use vm_runtime::{wasmtime_runner::WasmtimeRunner, Manifest, Runner};

static HTTP_REQUESTS: LazyLock<IntCounter> = LazyLock::new(|| {
    register_int_counter!("flovenet_http_requests_total", "Total HTTP requests").unwrap()
});

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
}

async fn metrics_handler() -> String {
    HTTP_REQUESTS.inc();
    let metric_families = prometheus::gather();
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap_or_default()
}

fn build_router() -> Router {
    Router::new()
        .route("/metrics", get(metrics_handler))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
}

async fn run_metrics_server(port: u16) -> anyhow::Result<()> {
    let app = build_router();
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Metrics/API endpoint: http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn run_daemon(
    port: u16,
    api_port: u16,
    roles: Vec<NodeRole>,
    swarm_key_path: Option<&str>,
) -> anyhow::Result<()> {
    let swarm_key = networking::load_swarm_key(swarm_key_path);
    if swarm_key_path.is_some() {
        tracing::info!(
            "Swarm key {}loaded",
            if swarm_key.is_some() { "" } else { "not " }
        );
    }
    tracing::info!(
        "Starting flovenet daemon (libp2p port: {port}, api: {api_port}, roles: {roles:?})"
    );

    let resources = NodeResources::detect();
    let total_slots = roles
        .iter()
        .map(|r| NodeDescriptor::slots_for_role(r, &resources))
        .max()
        .unwrap_or(1);
    let local_descriptor = NodeDescriptor {
        peer_id: String::new(),
        roles,
        resources: resources.clone(),
        region: "auto".into(),
        api_url: Some(format!("http://127.0.0.1:{api_port}")),
        total_slots,
        available_slots: total_slots,
    };

    let gpu_manager = resources.gpu_vram_gb.map(|vram| {
        let model = resources.gpu_model.as_deref().unwrap_or("unknown");
        std::sync::Arc::new(std::sync::Mutex::new(
            resource_manager::gpu::GpuSlotManager::new(vram, model),
        ))
    });
    if let Some(ref mgr) = gpu_manager {
        let m = mgr.lock().unwrap();
        tracing::info!(
            "GPU slot manager initialized: {} slots, {:.1} GiB total VRAM",
            m.total_slots(),
            m.total_vram_gb()
        );
    }

    let mut network = networking::NodeNetwork::new(port, None, swarm_key)?;
    let peer_id = network.peer_id;
    let listen_addr = network.listen_addr.clone();
    tracing::info!("Peer ID: {peer_id}, listening on {listen_addr}");

    let runner = WasmtimeRunner::new();
    let mut scheduler = LocalScheduler::new();

    // Initial reputation: record our uptime
    {
        let rep = network.reputation.read().await;
        scheduler.merge_reputation(&rep);
    }

    let rep_arc = network.reputation.clone();
    let peer_id_for_handler = peer_id.to_string();
    let gpu_mgr_for_handler = gpu_manager.clone();

    network
        .set_job_handler(move |offer: market_protocol::JobOffer| {
            let manifest = Manifest {
                image_cid: offer.manifest_cid.clone(),
                entrypoint: "_start".into(),
                args: vec![],
                max_duration_secs: offer.max_duration_secs,
                slots_required: offer.slots_required,
                gpu_vram_gb: offer.gpu_vram_gb,
            };

            let requirement = scheduler::SlotRequirement {
                cpu_cores: offer.slots_required,
                ram_gb: 1.0,
                disk_gb: 2.0,
                gpu_vram_gb: offer.gpu_vram_gb,
            };

            let role = if offer.gpu_required {
                resource_manager::NodeRole::Ai
            } else {
                resource_manager::NodeRole::Compute
            };

            match scheduler.can_accept(&local_descriptor, &requirement, &role) {
                scheduler::MatchResult::Rejected { reason } => market_protocol::JobResponse {
                    job_id: offer.job_id,
                    accepted: false,
                    reason: Some(reason),
                    result_cid: None,
                },
                scheduler::MatchResult::Accepted { .. } => {
                    let mut gpu_slots_allocated: Option<Vec<u32>> = None;
                    if let Some(ref mgr) = gpu_mgr_for_handler {
                        if let Some(vram_needed) = offer.gpu_vram_gb {
                            match mgr.lock().unwrap().allocate(vram_needed) {
                                Ok(slots) => {
                                    gpu_slots_allocated = Some(slots);
                                }
                                Err(reason) => {
                                    return market_protocol::JobResponse {
                                        job_id: offer.job_id,
                                        accepted: false,
                                        reason: Some(reason),
                                        result_cid: None,
                                    };
                                }
                            }
                        }
                    }

                    let result = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(runner.run(manifest))
                    });
                    let success = result.is_ok();

                    if let Some(ref mgr) = gpu_mgr_for_handler {
                        if let Some(slot_ids) = gpu_slots_allocated {
                            mgr.lock().unwrap().release(&slot_ids);
                        }
                    }

                    // Record job outcome in reputation
                    if let Ok(mut rep) = rep_arc.try_write() {
                        let mut score = rep
                            .get_score(&peer_id_for_handler)
                            .cloned()
                            .unwrap_or_else(|| {
                                reputation_engine::ReputationScore::new(&peer_id_for_handler)
                            });
                        score.record_job_outcome(success);
                        rep.apply_events(&[ReputationEvent {
                            peer_id: peer_id_for_handler.clone(),
                            timestamp: chrono::Utc::now(),
                            kind: if success {
                                EventKind::JobSuccess
                            } else {
                                EventKind::JobFailure
                            },
                        }]);
                        rep.recompute_all();
                    }

                    match result {
                        Ok(run_result) => market_protocol::JobResponse {
                            job_id: offer.job_id,
                            accepted: true,
                            reason: None,
                            result_cid: Some(format!("exit:{}", run_result.exit_code)),
                        },
                        Err(e) => market_protocol::JobResponse {
                            job_id: offer.job_id,
                            accepted: false,
                            reason: Some(e.to_string()),
                            result_cid: None,
                        },
                    }
                }
            }
        })
        .await;

    // Metrics server in background
    let metrics_handle = tokio::spawn(async move { run_metrics_server(api_port).await });

    // Run network event loop
    tokio::select! {
        result = network.run() => {
            tracing::error!("Network loop ended: {:?}", result);
        }
        result = metrics_handle => {
            tracing::error!("Metrics server ended: {:?}", result);
        }
    }

    Ok(())
}

async fn run_api_gateway(port: u16) -> anyhow::Result<()> {
    tracing::info!("Starting flovenet API gateway on port {port}");

    let mut network = networking::NodeNetwork::new(0, None, None)?;
    let peer_id = network.peer_id;
    tracing::info!("Gateway Peer ID: {peer_id}");

    tokio::spawn(async move {
        let _ = network.run().await;
    });

    let auth = graphql_api::auth::AuthManager::new("flovenet-dev-secret");
    let (event_tx, _) = tokio::sync::broadcast::channel(256);
    let state = graphql_api::AppState {
        auth,
        event_tx,
        store: Default::default(),
    };
    graphql_api::run_gateway(
        graphql_api::GatewayConfig {
            port,
            peer_id: peer_id.to_string(),
        },
        state,
    )
    .await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon {
            port,
            api_port,
            roles,
            swarm_key,
        } => {
            let parsed_roles: Vec<NodeRole> = if roles.is_empty() {
                vec![NodeRole::Compute]
            } else {
                roles
                    .split(',')
                    .map(|r| r.trim().parse::<NodeRole>().unwrap_or(NodeRole::Compute))
                    .collect()
            };
            run_daemon(port, api_port, parsed_roles, swarm_key.as_deref()).await
        }
        Commands::ApiGateway { port } => run_api_gateway(port).await,
        Commands::Share { role } => {
            let role_parsed = role
                .as_deref()
                .unwrap_or("compute")
                .parse::<NodeRole>()
                .unwrap_or(NodeRole::Compute);
            let resources = NodeResources::detect();
            let slots = NodeDescriptor::slots_for_role(&role_parsed, &resources);
            tracing::info!("Sharing resources for role {role_parsed:?}");
            tracing::info!("  CPU cores: {}", resources.cpu_cores);
            tracing::info!("  RAM: {:.1} GB", resources.ram_available_gb);
            tracing::info!("  Disk: {:.1} GB", resources.disk_available_gb);
            tracing::info!("  Available slots: {slots}");
            if let Some(vram) = resources.gpu_vram_gb {
                let model = resources.gpu_model.as_deref().unwrap_or("unknown");
                tracing::info!("  GPU: {model} ({vram:.0} GiB VRAM)");
                let gpu_slots = resource_manager::gpu::GpuSlot::create_slots(vram, model);
                tracing::info!(
                    "  GPU slots: {} ({} GiB each)",
                    gpu_slots.len(),
                    gpu_slots.first().map(|s| s.vram_gb).unwrap_or(0.0)
                );
            }
            println!("{}", serde_json::to_string_pretty(&resources)?);
            Ok(())
        }
        Commands::Run { manifest, image } => {
            let manifest = Manifest {
                image_cid: image.unwrap_or_else(|| manifest.clone()),
                entrypoint: manifest,
                args: vec![],
                max_duration_secs: 60,
                slots_required: 1,
                gpu_vram_gb: None,
            };
            tracing::info!("Running WASM locally: {manifest:?}");
            let runner = WasmtimeRunner::new();
            let result = runner.run(manifest).await?;
            tracing::info!(
                "Exit code: {}, duration: {:.2}s",
                result.exit_code,
                result.metrics.duration_secs
            );
            println!("{}", serde_json::to_string_pretty(&result)?);
            Ok(())
        }
        Commands::Status => {
            let resources = NodeResources::detect();
            println!("Flovenet Node Status");
            println!("====================");
            println!("CPU cores: {}", resources.cpu_cores);
            println!(
                "RAM: {:.1}/{:.1} GB",
                resources.ram_available_gb, resources.ram_total_gb
            );
            println!(
                "Disk: {:.1}/{:.1} GB",
                resources.disk_available_gb, resources.disk_total_gb
            );
            println!("Uptime: {}s", resources.uptime_secs);
            if let Some(vram) = resources.gpu_vram_gb {
                let model = resources.gpu_model.as_deref().unwrap_or("unknown");
                println!("GPU: {model} ({vram:.0} GiB VRAM)");
            } else {
                println!("GPU: not detected");
            }
            Ok(())
        }
    }
}
