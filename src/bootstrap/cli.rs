use std::io::{BufRead, Write};

use crate::bootstrap::flow_runner::{PromptAdapter, Question};
use crate::error::{DeployerError, Result};

pub struct CliPromptAdapter<R: BufRead, W: Write> {
    input: R,
    output: W,
}

impl<R: BufRead, W: Write> CliPromptAdapter<R, W> {
    pub fn new(input: R, output: W) -> Self {
        Self { input, output }
    }
}

impl<R: BufRead, W: Write> PromptAdapter for CliPromptAdapter<R, W> {
    fn ask(&mut self, questions: &[Question]) -> Result<serde_json::Value> {
        let mut answers = serde_json::Map::new();
        for q in questions {
            let prompt = format!(
                "{}{}: ",
                q.prompt,
                q.default
                    .as_ref()
                    .map(|d| format!(" [default: {d}]"))
                    .unwrap_or_default()
            );
            write!(self.output, "{prompt}")?;
            self.output.flush()?;
            let mut line = String::new();
            self.input.read_line(&mut line)?;
            let answer = line.trim().to_string();
            if answer.is_empty() {
                if let Some(default) = &q.default {
                    answers.insert(q.id.clone(), serde_json::Value::String(default.clone()));
                    continue;
                } else {
                    return Err(DeployerError::Other(format!(
                        "no input provided for {}",
                        q.id
                    )));
                }
            }
            answers.insert(q.id.clone(), serde_json::Value::String(answer));
        }
        Ok(serde_json::Value::Object(answers))
    }
}

/// Adapter used when interaction disallows prompts; errors immediately if asked.
pub struct DenyPromptAdapter;

impl PromptAdapter for DenyPromptAdapter {
    fn ask(&mut self, _questions: &[Question]) -> Result<serde_json::Value> {
        Err(DeployerError::Other(
            "interactive prompts are disabled by policy".into(),
        ))
    }
}

/// Adapter for non-interactive JSON answers.
pub struct JsonPromptAdapter {
    answers: serde_json::Map<String, serde_json::Value>,
}

impl JsonPromptAdapter {
    pub fn new(answers: serde_json::Value) -> Result<Self> {
        let map = answers
            .as_object()
            .cloned()
            .ok_or_else(|| DeployerError::Config("answers JSON must be an object".into()))?;
        Ok(Self { answers: map })
    }
}

impl PromptAdapter for JsonPromptAdapter {
    fn ask(&mut self, questions: &[Question]) -> Result<serde_json::Value> {
        let mut provided = serde_json::Map::new();
        for q in questions {
            let value = self
                .answers
                .get(&q.id)
                .cloned()
                .or_else(|| {
                    q.default
                        .as_ref()
                        .map(|d| serde_json::Value::String(d.clone()))
                })
                .ok_or_else(|| {
                    DeployerError::Config(format!("missing answer for question '{}'", q.id))
                })?;
            provided.insert(q.id.clone(), value);
        }
        Ok(serde_json::Value::Object(provided))
    }
}
