use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model_runtime::{ModelCallReceipt, ProviderToolCall};
use crate::provider_tool::provider_tool_call_name;
use crate::safe_blob_name;
use crate::RuntimeKind;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolLoopStatus {
    Continue,
    Answered,
    Clarifying,
    NeedsTask,
    Running,
    WaitingUser,
    WaitingApproval,
    Blocked,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolLoopBudgetErrorKind {
    PerSubturnToolCallLimit,
    TotalToolCallLimit,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolLoopBudgetError {
    pub kind: ProviderToolLoopBudgetErrorKind,
    pub requested_tool_calls: usize,
    pub limit: usize,
    pub executed_tool_calls_before: usize,
    pub all_read_only_or_control: bool,
    pub message: String,
}

impl ProviderToolLoopBudgetError {
    pub fn error_code(&self) -> &'static str {
        "MODEL_TOOL_LOOP_BUDGET_EXCEEDED"
    }

    pub fn budget_kind(&self) -> &'static str {
        match self.kind {
            ProviderToolLoopBudgetErrorKind::PerSubturnToolCallLimit => {
                "per_subturn_tool_call_limit"
            }
            ProviderToolLoopBudgetErrorKind::TotalToolCallLimit => "total_tool_call_limit",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolExecution {
    pub provider_tool_batch_id: String,
    pub provider_tool_call_id: String,
    pub provider_tool_call_index: usize,
    pub provider_tool_name: Option<String>,
    pub capability_id: Option<String>,
    pub status: ProviderToolLoopStatus,
    pub tool_result: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolLoopSubturn {
    pub runtime_kind: RuntimeKind,
    pub subturn_index: usize,
    pub model_call_id: String,
    pub provider_tool_batch_id: String,
    pub tool_call_count: usize,
    pub max_tool_calls_per_subturn: usize,
    pub max_tool_calls_total: usize,
    pub executed_tool_calls_before_subturn: usize,
    pub all_read_only_or_control: bool,
    pub mutation_allowed: bool,
    pub allow_parallel_readonly: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ProviderToolLoopOutcome {
    pub status: ProviderToolLoopStatus,
    pub model_receipts: Vec<ModelCallReceipt>,
    pub executions: Vec<ProviderToolExecution>,
    pub terminal_payload: Option<Value>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DecodedProviderToolCall {
    pub provider_tool_call_id: String,
    pub provider_tool_name: Option<String>,
    pub capability_id: Option<String>,
    pub arguments: Value,
    pub raw: ProviderToolCall,
}

pub trait ProviderToolLoopAdapter {
    fn runtime_kind(&self) -> RuntimeKind;
    fn loop_policy(&self) -> ProviderToolLoopPolicy;
    fn provider_tool_calls_are_limit_exempt(&self, tool_calls: &[ProviderToolCall]) -> bool;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderToolLoopPolicy {
    pub max_provider_subturns: usize,
    pub max_tool_calls_per_subturn: usize,
    pub max_tool_calls_total: usize,
    pub allow_parallel_readonly: bool,
    pub mutation_allowed: bool,
}

impl ProviderToolLoopPolicy {
    pub fn normalized(self) -> Self {
        Self {
            max_provider_subturns: self.max_provider_subturns.max(1),
            max_tool_calls_per_subturn: self.max_tool_calls_per_subturn.max(1),
            max_tool_calls_total: self.max_tool_calls_total.max(1),
            allow_parallel_readonly: self.allow_parallel_readonly,
            mutation_allowed: self.mutation_allowed,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProviderToolLoopExecutor {
    policy: ProviderToolLoopPolicy,
    executed_tool_calls: usize,
}

impl ProviderToolLoopExecutor {
    pub fn new(policy: ProviderToolLoopPolicy) -> Self {
        Self {
            policy: policy.normalized(),
            executed_tool_calls: 0,
        }
    }

    pub fn from_adapter(adapter: &impl ProviderToolLoopAdapter) -> Self {
        Self::new(adapter.loop_policy())
    }

    pub fn policy(&self) -> &ProviderToolLoopPolicy {
        &self.policy
    }

    pub fn executed_tool_calls(&self) -> usize {
        self.executed_tool_calls
    }

    pub fn mark_executed(&mut self, count: usize) {
        self.executed_tool_calls = self.executed_tool_calls.saturating_add(count);
    }

    pub fn seed_executed_tool_calls(&mut self, count: usize) {
        self.executed_tool_calls = count;
    }

    pub fn batch_id(&self, model_call_id: &str, subturn_index: usize) -> String {
        format!(
            "ptbatch_{}_{}",
            safe_blob_name(model_call_id),
            subturn_index
        )
    }

    pub fn begin_subturn(
        &self,
        adapter: &impl ProviderToolLoopAdapter,
        model_call_id: &str,
        subturn_index: usize,
        tool_calls: &[ProviderToolCall],
    ) -> Result<ProviderToolLoopSubturn, ProviderToolLoopBudgetError> {
        let all_read_only_or_control = adapter.provider_tool_calls_are_limit_exempt(tool_calls);
        self.check_subturn_budget(tool_calls, all_read_only_or_control)?;
        Ok(ProviderToolLoopSubturn {
            runtime_kind: adapter.runtime_kind(),
            subturn_index,
            model_call_id: model_call_id.to_string(),
            provider_tool_batch_id: self.batch_id(model_call_id, subturn_index),
            tool_call_count: tool_calls.len(),
            max_tool_calls_per_subturn: self.policy.max_tool_calls_per_subturn,
            max_tool_calls_total: self.policy.max_tool_calls_total,
            executed_tool_calls_before_subturn: self.executed_tool_calls,
            all_read_only_or_control,
            mutation_allowed: self.policy.mutation_allowed,
            allow_parallel_readonly: self.policy.allow_parallel_readonly,
        })
    }

    pub fn validate_subturn(
        &self,
        tool_calls: &[ProviderToolCall],
        all_read_only_or_control: bool,
    ) -> io::Result<()> {
        self.check_subturn_budget(tool_calls, all_read_only_or_control)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err.message))
    }

    pub fn check_subturn_budget(
        &self,
        tool_calls: &[ProviderToolCall],
        all_read_only_or_control: bool,
    ) -> Result<(), ProviderToolLoopBudgetError> {
        let requested = tool_calls.len();
        if requested > self.policy.max_tool_calls_per_subturn && !all_read_only_or_control {
            return Err(ProviderToolLoopBudgetError {
                kind: ProviderToolLoopBudgetErrorKind::PerSubturnToolCallLimit,
                requested_tool_calls: requested,
                limit: self.policy.max_tool_calls_per_subturn,
                executed_tool_calls_before: self.executed_tool_calls,
                all_read_only_or_control,
                message: format!(
                    "provider tool loop returned {requested} non-read-only tool calls in one subturn, exceeding limit {}",
                    self.policy.max_tool_calls_per_subturn
                ),
            });
        }
        let total = self.executed_tool_calls.saturating_add(requested);
        if total > self.policy.max_tool_calls_total {
            return Err(ProviderToolLoopBudgetError {
                kind: ProviderToolLoopBudgetErrorKind::TotalToolCallLimit,
                requested_tool_calls: requested,
                limit: self.policy.max_tool_calls_total,
                executed_tool_calls_before: self.executed_tool_calls,
                all_read_only_or_control,
                message: format!(
                    "provider tool loop would execute {total} total tool calls, exceeding limit {}",
                    self.policy.max_tool_calls_total
                ),
            });
        }
        Ok(())
    }

    pub fn skipped_result(
        &self,
        call: &ProviderToolCall,
        reason: &str,
        batch_id: &str,
        index: usize,
    ) -> Value {
        json!({
            "status": "skipped",
            "reason": reason,
            "provider_tool_batch_id": batch_id,
            "provider_tool_call_id": call.id,
            "provider_tool_call_index": index,
            "provider_tool_name": provider_tool_call_name(call).ok(),
        })
    }
}
