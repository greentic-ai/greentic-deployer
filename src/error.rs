use std::io;

use serde_cbor;
use serde_json;
use thiserror::Error;
use zip;

#[derive(Debug, Error)]
pub enum DeployerError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("pack parsing error: {0}")]
    Pack(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("CBOR serialization error: {0}")]
    Cbor(#[from] serde_cbor::Error),

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
