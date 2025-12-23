use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bootstrap::output::BootstrapOutput;
use crate::error::{DeployerError, Result};

#[derive(Debug, Deserialize)]
struct BootstrapFlow {
    steps: Vec<BootstrapStep>,
}

#[derive(Debug, Deserialize)]
struct BootstrapStep {
    kind: String,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    questions: Option<Vec<Question>>,
}

#[derive(Debug, Deserialize, Clone, Serialize)]
pub struct Question {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub default: Option<String>,
}

pub trait PromptAdapter {
    fn ask(&mut self, questions: &[Question]) -> Result<Value>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlowExecutionResult {
    pub output: BootstrapOutput,
    pub status_history: Vec<String>,
}

/// Executes a minimal bootstrap flow definition.
///
/// Allowed steps:
/// - kind="installer_call": captures `result` as the flow output (last one wins)
/// - kind="prompt": prompts via adapter (answers are not yet fed back to installer)
///
/// Everything else is denied in bootstrap mode.
pub fn run_bootstrap_flow(
    bytes: &[u8],
    prompt_adapter: &mut dyn PromptAdapter,
) -> Result<FlowExecutionResult> {
    let flow: BootstrapFlow = serde_json::from_slice(bytes)
        .map_err(|err| DeployerError::Other(format!("invalid ygtc format: {err}")))?;

    let mut statuses: Vec<String> = Vec::new();
    statuses.push("waiting_for_answers".into());

    let mut output: Option<BootstrapOutput> = None;
    for step in flow.steps {
        match step.kind.as_str() {
            "installer_call" => {
                statuses.push("deploying".into());
                let raw = step
                    .result
                    .ok_or_else(|| DeployerError::Other("installer_call missing result".into()))?;
                let parsed: BootstrapOutput = serde_json::from_value(raw).map_err(|err| {
                    DeployerError::Other(format!("invalid bootstrap output: {err}"))
                })?;
                output = Some(parsed);
            }
            "prompt" => {
                statuses.push("validating".into());
                let questions = step.questions.unwrap_or_default();
                prompt_adapter.ask(&questions)?;
                statuses.push("applying_config".into());
            }
            other => {
                return Err(DeployerError::Other(format!(
                    "not allowed in bootstrap mode: {other}"
                )));
            }
        }
    }

    let final_output = output.ok_or_else(|| {
        DeployerError::Other("bootstrap flow produced no installer_call output".into())
    })?;
    statuses.push(if final_output.ready {
        "completed".into()
    } else {
        "failed".into()
    });

    Ok(FlowExecutionResult {
        output: final_output,
        status_history: statuses,
    })
}
