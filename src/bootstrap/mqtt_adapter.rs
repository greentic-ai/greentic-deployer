use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::bootstrap::flow_runner::{PromptAdapter, Question};
use crate::bootstrap::network::NetworkPolicy;
use crate::error::{DeployerError, Result};

type SubscriberMap = HashMap<String, Vec<std::sync::mpsc::Sender<Vec<u8>>>>;

/// Simple in-memory mock broker used for tests and scaffolding.
#[derive(Clone, Default)]
pub struct MockBroker {
    inner: Arc<Mutex<SubscriberMap>>,
}

impl MockBroker {
    pub fn subscribe(&self, topic: &str) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut guard = self.inner.lock().unwrap();
        guard.entry(topic.to_string()).or_default().push(tx);
        rx
    }

    pub fn publish(&self, topic: &str, payload: &[u8]) {
        let guard = self.inner.lock().unwrap();
        if let Some(subs) = guard.get(topic) {
            for sub in subs {
                let _ = sub.send(payload.to_vec());
            }
        }
    }
}

pub struct MqttPromptAdapter {
    broker: MockBroker,
    device_id: String,
    topic_prefix: String,
    timeout: Duration,
    network_policy: Option<(NetworkPolicy, String)>,
}

impl MqttPromptAdapter {
    pub fn new_mock(broker: MockBroker, device_id: String, topic_prefix: String) -> Result<Self> {
        Ok(Self {
            broker,
            device_id,
            topic_prefix,
            timeout: Duration::from_secs(5),
            network_policy: None,
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_network_policy(
        mut self,
        policy: NetworkPolicy,
        broker_host: String,
    ) -> Result<Self> {
        policy.enforce(&broker_host)?;
        self.network_policy = Some((policy, broker_host));
        Ok(self)
    }

    fn schema_topic(&self) -> String {
        format!("{}/{}/schema", self.topic_prefix, self.device_id)
    }

    fn answers_topic(&self) -> String {
        format!("{}/{}/answers", self.topic_prefix, self.device_id)
    }

    fn status_topic(&self) -> String {
        format!("{}/{}/status", self.topic_prefix, self.device_id)
    }
}

impl PromptAdapter for MqttPromptAdapter {
    fn ask(&mut self, questions: &[Question]) -> Result<Value> {
        if let Some((policy, host)) = &self.network_policy {
            policy.enforce(host)?;
        }
        // publish schema
        let payload = serde_json::to_vec(&json!({ "questions": questions }))
            .map_err(|err| DeployerError::Other(format!("failed to encode schema: {err}")))?;
        self.broker.publish(&self.schema_topic(), &payload);

        // subscribe to answers
        let rx = self.broker.subscribe(&self.answers_topic());
        let answers_payload = rx
            .recv_timeout(self.timeout)
            .map_err(|_| DeployerError::Other("timeout waiting for MQTT answers".into()))?;

        let answers: Value = serde_json::from_slice(&answers_payload)
            .map_err(|err| DeployerError::Other(format!("invalid MQTT answers payload: {err}")))?;

        let mut provided = serde_json::Map::new();
        for q in questions {
            let value = answers
                .get(&q.id)
                .cloned()
                .or_else(|| q.default.as_ref().map(|d| Value::String(d.clone())))
                .ok_or_else(|| {
                    DeployerError::Config(format!("missing answer for question '{}'", q.id))
                })?;
            provided.insert(q.id.clone(), value);
        }

        // publish status update for observers
        let status = json!({"status": "answers_received"});
        if let Ok(bytes) = serde_json::to_vec(&status) {
            self.broker.publish(&self.status_topic(), &bytes);
        }

        Ok(Value::Object(provided))
    }
}
