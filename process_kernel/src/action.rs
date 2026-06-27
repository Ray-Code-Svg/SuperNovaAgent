use std::collections::BTreeSet;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CapabilityDescriptor, CapabilityToken};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessActionKind {
    RunCapability,
    ModelCall,
    ForkProcess,
    RequestPreview,
    Commit,
    Verify,
    Audit,
    Clarify,
    Complete,
    Fail,
}

impl ProcessActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RunCapability => "run_capability",
            Self::ModelCall => "model_call",
            Self::ForkProcess => "fork_process",
            Self::RequestPreview => "request_preview",
            Self::Commit => "commit",
            Self::Verify => "verify",
            Self::Audit => "audit",
            Self::Clarify => "clarify",
            Self::Complete => "complete",
            Self::Fail => "fail",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProcessAction {
    pub action_id: String,
    pub job_id: String,
    pub pid: String,
    pub reasoning_step_id: String,
    pub action_kind: ProcessActionKind,
    pub capability_id: String,
    pub input_refs: Vec<String>,
    pub output_spec: Value,
    pub policy: Value,
    pub verify_plan: Value,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ProcessActionValidator {
    capability_ids: BTreeSet<String>,
}

impl ProcessActionValidator {
    pub fn new(registry: &[CapabilityDescriptor]) -> Self {
        Self {
            capability_ids: registry
                .iter()
                .map(|item| item.capability_id.clone())
                .collect(),
        }
    }

    pub fn validate(
        &self,
        action: &ProcessAction,
        token: &CapabilityToken,
    ) -> ActionValidationResult {
        let mut errors = Vec::new();
        if action.job_id != token.job_id {
            errors.push("action job_id does not match capability token".to_string());
        }
        if action.pid != token.pid {
            errors.push("action pid does not match capability token".to_string());
        }
        if !self.capability_ids.contains(&action.capability_id) {
            errors.push(format!("unknown capability: {}", action.capability_id));
        }
        if !token
            .capabilities
            .iter()
            .any(|item| item == &action.capability_id)
        {
            errors.push(format!(
                "capability token does not grant {}",
                action.capability_id
            ));
        }
        if action.action_id.trim().is_empty() {
            errors.push("action_id is required".to_string());
        }
        if action.reasoning_step_id.trim().is_empty() {
            errors.push("reasoning_step_id is required".to_string());
        }
        ActionValidationResult {
            valid: errors.is_empty(),
            errors,
        }
    }

    pub fn validate_or_err(
        &self,
        action: &ProcessAction,
        token: &CapabilityToken,
    ) -> io::Result<()> {
        let result = self.validate(action, token);
        if result.valid {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            result.errors.join("; "),
        ))
    }
}
