use greentic_telemetry::{TelemetryConfig, init_telemetry_auto};

use crate::config::DeployerConfig;
use crate::error::{DeployerError, Result};

pub fn init(config: &DeployerConfig) -> Result<()> {
    let telemetry_cfg = config.telemetry_config();
    if !telemetry_cfg.enabled {
        return Ok(());
    }

    let cfg = TelemetryConfig {
        service_name: format!("greentic-deployer-{}", config.provider.as_str()),
    };

    init_telemetry_auto(cfg).map_err(|err| DeployerError::Telemetry(err.to_string()))
}
