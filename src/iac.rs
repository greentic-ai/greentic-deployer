use std::fmt;
use std::path::Path;
use std::process::Command;

use clap::ValueEnum;
use tracing::{debug, warn};

use crate::error::DeployerError;

/// Supported IaC tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IaCTool {
    Terraform,
    OpenTofu,
}

impl fmt::Display for IaCTool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            IaCTool::Terraform => "terraform",
            IaCTool::OpenTofu => "tofu",
        })
    }
}

impl IaCTool {
    pub fn binary_name(&self) -> &'static str {
        match self {
            IaCTool::Terraform => "terraform",
            IaCTool::OpenTofu => "tofu",
        }
    }

    pub fn from_env(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "terraform" | "tf" => Some(IaCTool::Terraform),
            "opentofu" | "tofu" => Some(IaCTool::OpenTofu),
            _ => None,
        }
    }
}

/// CLI argument helper for IaC tool selection.
#[derive(ValueEnum, Clone, Copy, Debug)]
pub enum IacToolArg {
    #[value(alias = "tf")]
    Terraform,
    #[value(alias = "tofu")]
    OpenTofu,
}

impl From<IacToolArg> for IaCTool {
    fn from(value: IacToolArg) -> Self {
        match value {
            IacToolArg::Terraform => IaCTool::Terraform,
            IacToolArg::OpenTofu => IaCTool::OpenTofu,
        }
    }
}

/// Resolve which IaC tool to use based on CLI flag, env, or PATH.
pub fn resolve_iac_tool(
    cli_arg: Option<IacToolArg>,
    env_override: Option<String>,
) -> Result<IaCTool, DeployerError> {
    if let Some(arg) = cli_arg {
        return Ok(arg.into());
    }
    if let Some(env) = env_override {
        if let Some(tool) = IaCTool::from_env(&env) {
            return Ok(tool);
        }
        return Err(DeployerError::Config(format!(
            "Invalid IaC tool '{env}' set via GREENTIC_IAC_TOOL"
        )));
    }

    if which::which("tofu").is_ok() {
        return Ok(IaCTool::OpenTofu);
    }
    if which::which("terraform").is_ok() {
        return Ok(IaCTool::Terraform);
    }

    warn!(
        "No terraform/tofu binary found on PATH; defaulting to terraform. IaC commands will fail later if the binary is missing."
    );
    Ok(IaCTool::Terraform)
}

/// Runner responsible for executing IaC commands.
pub trait IaCCommandRunner: Send + Sync {
    fn run(&self, tool: IaCTool, dir: &Path, args: &[&str]) -> Result<(), DeployerError>;
}

pub struct DefaultIaCCommandRunner;

impl IaCCommandRunner for DefaultIaCCommandRunner {
    fn run(&self, tool: IaCTool, dir: &Path, args: &[&str]) -> Result<(), DeployerError> {
        let binary = tool.binary_name();
        let mut command = Command::new(binary);
        command
            .args(args)
            .current_dir(dir)
            .stdin(std::process::Stdio::null());
        let output = command.output();
        match output {
            Ok(output) => {
                if output.status.success() {
                    Ok(())
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                    let step = args.get(0).copied().unwrap_or("command").to_string();
                    debug!(tool = ?tool, step = %step, stderr = %stderr);
                    Err(DeployerError::IaCTool {
                        tool: tool.to_string(),
                        step,
                        status: output.status.code(),
                        stderr,
                    })
                }
            }
            Err(err) => {
                if err.kind() == std::io::ErrorKind::NotFound {
                    Err(DeployerError::IaCToolMissing {
                        tool: tool.to_string(),
                        binary,
                    })
                } else {
                    Err(DeployerError::Io(err))
                }
            }
        }
    }
}

pub fn run_iac_plan_apply(
    runner: &dyn IaCCommandRunner,
    tool: IaCTool,
    dir: &Path,
) -> Result<(), DeployerError> {
    let commands = [
        &["init", "-input=false"][..],
        &["plan", "-input=false", "-out=plan.tfplan"][..],
        &["apply", "-input=false", "-auto-approve", "plan.tfplan"][..],
    ];
    for command in commands {
        runner.run(tool, dir, command)?;
    }
    Ok(())
}

pub fn run_iac_destroy(
    runner: &dyn IaCCommandRunner,
    tool: IaCTool,
    dir: &Path,
) -> Result<(), DeployerError> {
    let commands = [
        &["init", "-input=false"][..],
        &["destroy", "-input=false", "-auto-approve"][..],
    ];
    for command in commands {
        runner.run(tool, dir, command)?;
    }
    Ok(())
}

pub fn dry_run_commands(destroy: bool) -> Vec<Vec<&'static str>> {
    if destroy {
        vec![
            vec!["init", "-input=false"],
            vec!["destroy", "-input=false", "-auto-approve"],
        ]
    } else {
        vec![
            vec!["init", "-input=false"],
            vec!["plan", "-input=false", "-out=plan.tfplan"],
            vec!["apply", "-input=false", "-auto-approve", "plan.tfplan"],
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct MockRunner {
        calls: Arc<Mutex<Vec<(IaCTool, Vec<String>)>>>,
    }

    impl MockRunner {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn calls(&self) -> Vec<(IaCTool, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl IaCCommandRunner for MockRunner {
        fn run(&self, tool: IaCTool, _dir: &Path, args: &[&str]) -> Result<(), DeployerError> {
            let mut guard = self.calls.lock().unwrap();
            guard.push((tool, args.iter().map(|arg| arg.to_string()).collect()));
            Ok(())
        }
    }

    #[test]
    fn apply_sequence_invokes_commands() {
        let runner = MockRunner::new();
        run_iac_plan_apply(&runner, IaCTool::Terraform, Path::new("dummy")).unwrap();
        let expected = vec![
            vec!["init", "-input=false"],
            vec!["plan", "-input=false", "-out=plan.tfplan"],
            vec!["apply", "-input=false", "-auto-approve", "plan.tfplan"],
        ];
        assert_eq!(
            runner.calls(),
            expected
                .into_iter()
                .map(|args| (
                    IaCTool::Terraform,
                    args.into_iter().map(String::from).collect()
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn destroy_sequence_invokes_commands() {
        let runner = MockRunner::new();
        run_iac_destroy(&runner, IaCTool::OpenTofu, Path::new("dummy")).unwrap();
        let expected = vec![
            vec!["init", "-input=false"],
            vec!["destroy", "-input=false", "-auto-approve"],
        ];
        assert_eq!(
            runner.calls(),
            expected
                .into_iter()
                .map(|args| (
                    IaCTool::OpenTofu,
                    args.into_iter().map(String::from).collect()
                ))
                .collect::<Vec<_>>()
        );
    }
}
