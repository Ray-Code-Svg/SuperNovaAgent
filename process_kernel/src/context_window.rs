use std::io;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model_config::estimate_text_tokens_conservative;
use crate::{ProcessTruthStore, PROCESS_TRUTH_EVENT_SCHEMA_VERSION};

pub const CONTEXT_WINDOW_EVENT_SCHEMA_VERSION: &str = "supernova_context_window_event.v1";
pub const TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT: &str =
    "ProcessTruth is append-only task execution fact and is never compacted or replaced by context-window summaries.";
pub const CHAT_RUNTIME_MUTATION_FORBIDDEN_POLICY: &str =
    "ChatRuntime may execute read-only provider tools only; mutation intent must be rejected or surfaced as a suggested task.";

pub const CONTEXT_WINDOW_EVENT_TYPES: [&str; 11] = [
    "context_window_checked",
    "context_window_advisory",
    "context_window_compaction_required",
    "context_window_checkpoint_created",
    "context_window_compaction_model_call_started",
    "context_window_compaction_model_call_completed",
    "context_window_visible_context_replaced",
    "context_window_protocol_validated",
    "context_window_reestimate_completed",
    "context_window_emergency_trim_applied",
    "context_window_compaction_failed",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Chat,
    Task,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "scope")]
pub enum ContextScope {
    Container,
    Chat {
        container_id: String,
        chat_thread_id: String,
    },
    Task {
        container_id: Option<String>,
        job_id: String,
        process_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextWindowControlConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_advisory_ratio")]
    pub advisory_ratio: f64,
    #[serde(default = "default_proactive_compact_ratio")]
    pub proactive_compact_ratio: f64,
    #[serde(default = "default_hard_compact_ratio")]
    pub hard_compact_ratio: f64,
    #[serde(default = "default_emergency_ratio")]
    pub emergency_ratio: f64,
    #[serde(default = "default_min_live_suffix_turns")]
    pub min_live_suffix_turns: usize,
    #[serde(default = "default_max_summary_tokens")]
    pub max_summary_tokens: u64,
    #[serde(default = "default_reserve_output_tokens")]
    pub reserve_output_tokens: u64,
    #[serde(default = "default_reserve_reasoning_tokens")]
    pub reserve_reasoning_tokens: u64,
}

impl Eq for ContextWindowControlConfig {}

impl Default for ContextWindowControlConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            advisory_ratio: default_advisory_ratio(),
            proactive_compact_ratio: default_proactive_compact_ratio(),
            hard_compact_ratio: default_hard_compact_ratio(),
            emergency_ratio: default_emergency_ratio(),
            min_live_suffix_turns: default_min_live_suffix_turns(),
            max_summary_tokens: default_max_summary_tokens(),
            reserve_output_tokens: default_reserve_output_tokens(),
            reserve_reasoning_tokens: default_reserve_reasoning_tokens(),
        }
    }
}

impl ContextWindowControlConfig {
    pub fn validate(&self) -> io::Result<()> {
        let ratios = [
            self.advisory_ratio,
            self.proactive_compact_ratio,
            self.hard_compact_ratio,
            self.emergency_ratio,
        ];
        if ratios
            .iter()
            .any(|ratio| !ratio.is_finite() || *ratio <= 0.0 || *ratio > 1.0)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "context-window ratios must be finite values in 0.0..=1.0",
            ));
        }
        if !(self.advisory_ratio <= self.proactive_compact_ratio
            && self.proactive_compact_ratio <= self.hard_compact_ratio
            && self.hard_compact_ratio <= self.emergency_ratio)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "context-window ratios must be ordered advisory <= proactive <= hard <= emergency",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenEstimatorKind {
    ApproxChars,
    TiktokenCompatible,
    ProviderUsageCalibrated,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextWindowBreakdown {
    pub system_prompt_tokens: u64,
    pub developer_prompt_tokens: u64,
    pub user_message_tokens: u64,
    pub input_payload_tokens: u64,
    pub provider_transcript_tokens: u64,
    pub tool_schema_tokens: u64,
    pub tool_choice_tokens: u64,
    pub context_pack_tokens: u64,
    pub provider_options_tokens: u64,
}

impl ContextWindowBreakdown {
    pub fn estimated_input_tokens(&self) -> u64 {
        self.system_prompt_tokens
            .saturating_add(self.developer_prompt_tokens)
            .saturating_add(self.user_message_tokens)
            .saturating_add(self.input_payload_tokens)
            .saturating_add(self.provider_transcript_tokens)
            .saturating_add(self.tool_schema_tokens)
            .saturating_add(self.tool_choice_tokens)
            .saturating_add(self.context_pack_tokens)
            .saturating_add(self.provider_options_tokens)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextWindowEstimate {
    pub provider: String,
    pub model: String,
    pub context_window_tokens: u64,
    pub estimated_input_tokens: u64,
    pub reserved_output_tokens: u64,
    pub reserved_reasoning_tokens: u64,
    pub estimated_total_tokens: u64,
    pub usage_ratio: f64,
    pub breakdown: ContextWindowBreakdown,
    pub estimator: TokenEstimatorKind,
    pub provider_reported_last_prompt_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ContextWindowRequestParts {
    pub provider: String,
    pub model: String,
    pub context_window_tokens: u64,
    pub system_prompt: String,
    pub developer_prompt: String,
    pub user_message: String,
    pub input_payloads: Vec<String>,
    pub provider_transcript_messages: Vec<Value>,
    pub tool_schema: Value,
    pub tool_choice: Option<Value>,
    pub context_pack_payload: Value,
    pub provider_options: Value,
    pub reserved_output_tokens: Option<u64>,
    pub reserved_reasoning_tokens: Option<u64>,
    pub provider_reported_last_prompt_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextWindowDecisionKind {
    SendAsIs,
    Advisory,
    CompactProactive,
    CompactRequired,
    Emergency,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextWindowDecision {
    pub kind: ContextWindowDecisionKind,
    pub reason: String,
    pub compact_before_send: bool,
    pub hard_block_if_compaction_fails: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextWindowPreflight {
    pub scope: ContextScope,
    pub estimate: ContextWindowEstimate,
    pub decision: ContextWindowDecision,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ContextWindowEvent {
    pub schema_version: String,
    pub event_type: String,
    pub scope: ContextScope,
    pub estimate: ContextWindowEstimate,
    pub decision: ContextWindowDecision,
    pub data: Value,
}

pub trait ContextWindowScopeAdapter {
    fn scope(&self) -> ContextScope;
    fn build_visible_request_parts(&self) -> io::Result<ContextWindowRequestParts>;
    fn build_compaction_input(
        &self,
        _estimate: &ContextWindowEstimate,
    ) -> io::Result<crate::context_compaction::ContextCompactionInput> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "context-window scope adapter does not implement compaction input",
        ))
    }
    fn run_pre_compaction_checkpoint(
        &mut self,
        _estimate: &ContextWindowEstimate,
    ) -> io::Result<crate::context_compaction::ContextCheckpointReceipt> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "context-window scope adapter does not implement pre-compaction checkpoint",
        ))
    }
    fn replace_visible_context(
        &mut self,
        _compaction: crate::context_compaction::ContextCompactionReceipt,
    ) -> io::Result<crate::context_compaction::ProviderTranscriptReplacement> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "context-window scope adapter does not implement visible context replacement",
        ))
    }
    fn validate_provider_protocol(
        &self,
        _replacement: &crate::context_compaction::ProviderTranscriptReplacement,
    ) -> io::Result<crate::context_compaction::ProviderTranscriptValidationReceipt> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "context-window scope adapter does not implement provider protocol validation",
        ))
    }
    fn append_context_event(&mut self, event: ContextWindowEvent) -> io::Result<()>;
}

#[derive(Clone, Debug, Default)]
pub struct ContextWindowController;

impl ContextWindowController {
    pub fn preflight(
        scope: ContextScope,
        config: &ContextWindowControlConfig,
        parts: &ContextWindowRequestParts,
    ) -> io::Result<ContextWindowPreflight> {
        config.validate()?;
        let estimate = Self::estimate(config, parts);
        let decision = Self::decide(config, &estimate);
        Ok(ContextWindowPreflight {
            scope,
            estimate,
            decision,
        })
    }

    pub fn estimate(
        config: &ContextWindowControlConfig,
        parts: &ContextWindowRequestParts,
    ) -> ContextWindowEstimate {
        let tool_schema_text = if parts.tool_schema.is_null() {
            String::new()
        } else {
            serde_json::to_string(&parts.tool_schema).unwrap_or_default()
        };
        let tool_choice_text = parts
            .tool_choice
            .as_ref()
            .map(|value| serde_json::to_string(value).unwrap_or_default())
            .unwrap_or_default();
        let context_pack_text = if parts.context_pack_payload.is_null() {
            String::new()
        } else {
            serde_json::to_string(&parts.context_pack_payload).unwrap_or_default()
        };
        let provider_options_text = if parts.provider_options.is_null() {
            String::new()
        } else {
            serde_json::to_string(&parts.provider_options).unwrap_or_default()
        };
        let provider_transcript_text =
            serde_json::to_string(&parts.provider_transcript_messages).unwrap_or_default();
        let breakdown = ContextWindowBreakdown {
            system_prompt_tokens: estimate_text_tokens_conservative(&parts.system_prompt),
            developer_prompt_tokens: estimate_text_tokens_conservative(&parts.developer_prompt),
            user_message_tokens: estimate_text_tokens_conservative(&parts.user_message),
            input_payload_tokens: parts
                .input_payloads
                .iter()
                .map(|item| estimate_text_tokens_conservative(item))
                .sum(),
            provider_transcript_tokens: estimate_text_tokens_conservative(
                &provider_transcript_text,
            ),
            tool_schema_tokens: estimate_text_tokens_conservative(&tool_schema_text),
            tool_choice_tokens: estimate_text_tokens_conservative(&tool_choice_text),
            context_pack_tokens: estimate_text_tokens_conservative(&context_pack_text),
            provider_options_tokens: estimate_text_tokens_conservative(&provider_options_text),
        };
        let estimated_input_tokens = breakdown.estimated_input_tokens();
        let reserved_output_tokens = parts
            .reserved_output_tokens
            .unwrap_or(config.reserve_output_tokens);
        let reserved_reasoning_tokens = parts
            .reserved_reasoning_tokens
            .unwrap_or(config.reserve_reasoning_tokens);
        let estimated_total_tokens = estimated_input_tokens
            .saturating_add(reserved_output_tokens)
            .saturating_add(reserved_reasoning_tokens);
        let context_window_tokens = parts.context_window_tokens.max(1);
        let usage_ratio = estimated_total_tokens as f64 / context_window_tokens as f64;
        ContextWindowEstimate {
            provider: parts.provider.clone(),
            model: parts.model.clone(),
            context_window_tokens,
            estimated_input_tokens,
            reserved_output_tokens,
            reserved_reasoning_tokens,
            estimated_total_tokens,
            usage_ratio,
            breakdown,
            estimator: TokenEstimatorKind::ApproxChars,
            provider_reported_last_prompt_tokens: parts.provider_reported_last_prompt_tokens,
        }
    }

    pub fn decide(
        config: &ContextWindowControlConfig,
        estimate: &ContextWindowEstimate,
    ) -> ContextWindowDecision {
        if !config.enabled {
            return ContextWindowDecision {
                kind: ContextWindowDecisionKind::SendAsIs,
                reason: "context-window controller disabled".to_string(),
                compact_before_send: false,
                hard_block_if_compaction_fails: false,
            };
        }
        let usage = estimate.usage_ratio;
        if usage >= config.emergency_ratio {
            ContextWindowDecision {
                kind: ContextWindowDecisionKind::Emergency,
                reason: "usage reached emergency threshold".to_string(),
                compact_before_send: true,
                hard_block_if_compaction_fails: true,
            }
        } else if usage >= config.hard_compact_ratio {
            ContextWindowDecision {
                kind: ContextWindowDecisionKind::CompactRequired,
                reason: "usage reached hard compaction threshold".to_string(),
                compact_before_send: true,
                hard_block_if_compaction_fails: true,
            }
        } else if usage >= config.proactive_compact_ratio {
            ContextWindowDecision {
                kind: ContextWindowDecisionKind::CompactProactive,
                reason: "usage reached proactive compaction threshold".to_string(),
                compact_before_send: true,
                hard_block_if_compaction_fails: false,
            }
        } else if usage >= config.advisory_ratio {
            ContextWindowDecision {
                kind: ContextWindowDecisionKind::Advisory,
                reason: "usage reached advisory threshold".to_string(),
                compact_before_send: false,
                hard_block_if_compaction_fails: false,
            }
        } else {
            ContextWindowDecision {
                kind: ContextWindowDecisionKind::SendAsIs,
                reason: "usage below advisory threshold".to_string(),
                compact_before_send: false,
                hard_block_if_compaction_fails: false,
            }
        }
    }
}

pub fn append_task_context_window_events(
    truth: &ProcessTruthStore,
    pid: &str,
    preflight: &ContextWindowPreflight,
    request_ref: Option<String>,
) -> io::Result<Vec<String>> {
    let mut event_types = vec!["context_window_checked"];
    match preflight.decision.kind {
        ContextWindowDecisionKind::Advisory => event_types.push("context_window_advisory"),
        ContextWindowDecisionKind::CompactProactive
        | ContextWindowDecisionKind::CompactRequired
        | ContextWindowDecisionKind::Emergency => {
            event_types.push("context_window_compaction_required")
        }
        ContextWindowDecisionKind::SendAsIs => {}
    }

    for event_type in &event_types {
        truth.append_event(
            Some(pid),
            event_type,
            json!({
                "schema_version": CONTEXT_WINDOW_EVENT_SCHEMA_VERSION,
                "process_truth_event_schema_version": PROCESS_TRUTH_EVENT_SCHEMA_VERSION,
                "scope": preflight.scope,
                "estimate": preflight.estimate,
                "decision": preflight.decision,
                "request_ref": request_ref,
                "task_process_truth_invariant": TASK_PROCESS_TRUTH_NOT_COMPRESSED_INVARIANT,
            }),
        )?;
    }
    Ok(event_types.into_iter().map(str::to_string).collect())
}

fn default_true() -> bool {
    true
}

fn default_advisory_ratio() -> f64 {
    0.70
}

fn default_proactive_compact_ratio() -> f64 {
    0.80
}

fn default_hard_compact_ratio() -> f64 {
    0.85
}

fn default_emergency_ratio() -> f64 {
    0.95
}

fn default_min_live_suffix_turns() -> usize {
    4
}

fn default_max_summary_tokens() -> u64 {
    2048
}

fn default_reserve_output_tokens() -> u64 {
    4096
}

fn default_reserve_reasoning_tokens() -> u64 {
    2048
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_defaults_match_threshold_policy() {
        let config = ContextWindowControlConfig::default();
        assert_eq!(config.advisory_ratio, 0.70);
        assert_eq!(config.proactive_compact_ratio, 0.80);
        assert_eq!(config.hard_compact_ratio, 0.85);
        assert_eq!(config.emergency_ratio, 0.95);
        config.validate().unwrap();
    }

    #[test]
    fn estimate_counts_tools_reserves_and_provider_options() {
        let config = ContextWindowControlConfig::default();
        let parts = ContextWindowRequestParts {
            provider: "deepseek".to_string(),
            model: "deepseek-v4-pro".to_string(),
            context_window_tokens: 100,
            system_prompt: "system prompt".to_string(),
            developer_prompt: "developer prompt".to_string(),
            user_message: "user asks for work".to_string(),
            input_payloads: vec!["payload".repeat(20)],
            provider_transcript_messages: vec![json!({"role": "assistant", "content": "prior"})],
            tool_schema: json!({"tools": [{"name": "os.read_file"}]}),
            tool_choice: Some(json!("auto")),
            context_pack_payload: json!({"selected": ["artifact://a"]}),
            provider_options: json!({"thinking": "enabled"}),
            reserved_output_tokens: Some(10),
            reserved_reasoning_tokens: Some(5),
            provider_reported_last_prompt_tokens: Some(12),
        };
        let estimate = ContextWindowController::estimate(&config, &parts);
        assert!(estimate.breakdown.tool_schema_tokens > 0);
        assert!(estimate.breakdown.tool_choice_tokens > 0);
        assert!(estimate.breakdown.provider_options_tokens > 0);
        assert_eq!(estimate.reserved_output_tokens, 10);
        assert_eq!(estimate.reserved_reasoning_tokens, 5);
        assert_eq!(estimate.provider_reported_last_prompt_tokens, Some(12));
    }

    #[test]
    fn hard_threshold_requires_compaction() {
        let config = ContextWindowControlConfig::default();
        let parts = ContextWindowRequestParts {
            provider: "deepseek".to_string(),
            model: "deepseek-v4-pro".to_string(),
            context_window_tokens: 100,
            input_payloads: vec!["x".repeat(92)],
            reserved_output_tokens: Some(0),
            reserved_reasoning_tokens: Some(0),
            ..ContextWindowRequestParts::default()
        };
        let preflight =
            ContextWindowController::preflight(ContextScope::Container, &config, &parts).unwrap();
        assert!(matches!(
            preflight.decision.kind,
            ContextWindowDecisionKind::CompactRequired | ContextWindowDecisionKind::Emergency
        ));
        assert!(preflight.decision.compact_before_send);
    }
}
