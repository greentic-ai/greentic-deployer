use std::env;

use greentic_telemetry::{OtlpConfig, init_otlp, layer_from_task_local};
use tracing_subscriber::{Registry, layer::Layer};

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};

pub fn init(config: &DeployerConfig) -> Result<()> {
    let endpoint = env::var("GREENTIC_OTLP_ENDPOINT")
        .or_else(|_| env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
        .ok();

    let layers: Vec<Box<dyn Layer<Registry> + Send + Sync>> =
        vec![Box::new(layer_from_task_local())];

    let cfg = OtlpConfig {
        service_name: format!("greentic-deployer-{}", config.provider.as_str()),
        endpoint,
        sampling_rate: None,
    };

    init_otlp(cfg, layers).map_err(|err| DeployerError::Telemetry(err.to_string()))
}
