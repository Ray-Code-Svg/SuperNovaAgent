use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model_runtime::ModelOperation;
use crate::now_ms;
use crate::observation::TaskObservation;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskAgentDecisionKind {
    DecideNextAction,
    VerifyArtifact,
    RunCapability,
    RequestPreview,
    Clarify,
    Complete,
    Interrupted,
    Fail,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct NextActionDecision {
    pub decision_id: String,
    pub kind: TaskAgentDecisionKind,
    pub reason: String,
    pub capability_id: String,
    pub artifact_path: Option<String>,
    pub input_refs: Vec<String>,
    pub output_spec: Value,
    pub model_operation: Option<ModelOperation>,
    pub retry_limit: u32,
}

pub fn decision(
    kind: TaskAgentDecisionKind,
    capability_id: &str,
    reason: &str,
) -> NextActionDecision {
    NextActionDecision {
        decision_id: format!("decision_{}", now_ms()),
        kind,
        reason: reason.to_string(),
        capability_id: capability_id.to_string(),
        artifact_path: None,
        input_refs: Vec::new(),
        output_spec: json!({}),
        model_operation: None,
        retry_limit: 0,
    }
}

pub fn verify_decision(artifact_path: &str) -> NextActionDecision {
    let mut item = decision(
        TaskAgentDecisionKind::VerifyArtifact,
        "os.verify_artifact",
        "Verify artifact before closure.",
    );
    item.artifact_path = Some(artifact_path.to_string());
    item
}

pub fn decision_model_input_refs(observation: &TaskObservation) -> Vec<String> {
    vec![
        observation.goal_ref.clone(),
        observation.observation_frame_ref.clone(),
    ]
}
