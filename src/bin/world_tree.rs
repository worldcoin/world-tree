use std::path::PathBuf;

use clap::Parser;
use telemetry_batteries::metrics::statsd::StatsdBattery;
use telemetry_batteries::tracing::datadog::DatadogBattery;
use telemetry_batteries::tracing::TracingShutdownHandle;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use world_tree::init_world_tree;
use world_tree::tasks::monitor_tasks;
use world_tree::tree::config::ServiceConfig;
use world_tree::tree::error::WorldTreeResult;
use world_tree::tree::service::InclusionProofService;

/// This service syncs the state of the World Tree and spawns a server that can deliver inclusion proofs for a given identity.
#[derive(Parser, Debug)]
#[clap(name = "Tree Availability Service")]
#[clap(version)]
struct Opts {
    /// Path to the configuration file
    #[clap(short, long)]
    config: Option<PathBuf>,

    /// Set to disable colors in the logs
    #[clap(long)]
    no_ansi: bool,
}

#[tokio::main]
pub async fn main() -> WorldTreeResult<()> {
    dotenv::dotenv().ok();

    let opts = Opts::parse();

    let config = ServiceConfig::load(opts.config.as_deref())?;

    let _tracing_shutdown_handle = if let Some(telemetry) = &config.telemetry {
        let tracing_shutdown_handle = DatadogBattery::init(
            telemetry.traces_endpoint.as_deref(),
            &telemetry.service_name,
            None,
            true,
        );

        if let Some(metrics_config) = &telemetry.metrics {
            StatsdBattery::init(
                &metrics_config.host,
                metrics_config.port,
                metrics_config.queue_size,
                metrics_config.buffer_size,
                Some(&metrics_config.prefix),
            )?;
        }

        tracing_shutdown_handle
    } else {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(!opts.no_ansi)
                    .pretty()
                    .compact(),
            )
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .init();

        TracingShutdownHandle
    };

    tracing::info!(?config, "Starting World Tree service");

    let world_tree = init_world_tree(&config).await?;

    let service = InclusionProofService::new(world_tree);
    let (_, handles) = service.serve(config.socket_address).await?;
    monitor_tasks(handles, config.shutdown_delay).await?;

    Ok(())
}
