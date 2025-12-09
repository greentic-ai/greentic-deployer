use std::io;

use serde_json;
use thiserror::Error;

use greentic_distributor_client::error::DistributorError;

#[derive(Debug, Error)]
pub enum DeployerError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("pack parsing error: {0}")]
    Pack(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("manifest decode error: {0}")]
    ManifestDecode(#[from] greentic_types::cbor::CborError),

    #[error("distributor error: {0}")]
    Distributor(#[from] DistributorError),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("telemetry initialization error: {0}")]
    Telemetry(String),

    #[error("secret backend error: {0}")]
    Secret(String),

    #[error(
        "IaC tool '{tool}' missing on PATH (binary '{binary}'). Install it or choose --iac-tool / GREENTIC_IAC_TOOL."
    )]
    IaCToolMissing { tool: String, binary: &'static str },

    #[error("IaC tool '{tool}' command '{step}' failed (exit {status:?}): {stderr}")]
    IaCTool {
        tool: String,
        step: String,
        status: Option<i32>,
        stderr: String,
    },

    #[error("deployment packs not wired yet for provider={provider}, strategy={strategy}")]
    DeploymentPackUnsupported { provider: String, strategy: String },

    #[error("unexpected error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DeployerError>;
