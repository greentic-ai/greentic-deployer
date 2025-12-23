#![forbid(unsafe_code)]

#[allow(unused_imports)]
use greentic_interfaces_host as _greentic_interfaces_host;

pub mod apply;
pub mod bootstrap;
pub mod config;
pub mod deployment;
pub mod error;
pub mod iac;
pub mod pack_introspect;
pub mod path_safety;
pub mod plan;
pub mod platform;
pub mod providers;
pub mod secrets;
pub mod telemetry;

pub use config::{Action, CliArgs, Command, DeployerConfig, OutputFormat, Provider};
pub use error::DeployerError;
pub use plan::{
    ChannelContext, ComponentRole, DeploymentProfile, InferenceNotes, InfraPlan, MessagingContext,
    PlanContext, PlannedComponent, Target, TelemetryContext,
};
pub use providers::{GeneratedFile, ProviderArtifacts, ProviderBackend};
