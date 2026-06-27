use std::env;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::context_window::ContextWindowControlConfig;
use crate::model_runtime::{ModelBudget, ModelOperation, ModelProvider};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskAgentDecisionProtocol {
    #[serde(alias = "super_nova_json_decision", alias = "auto")]
    ProviderNativeToolCalls,
}

impl Default for TaskAgentDecisionProtocol {
    fn default() -> Self {
        Self::ProviderNativeToolCalls
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRouteMode {
    Auto,
    Flash,
    Pro,
    Fixed,
}

impl Default for ModelRouteMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRoutePreference {
    #[serde(default)]
    pub mode: ModelRouteMode,
    #[serde(default)]
    pub fixed_model: Option<String>,
}

impl Default for ModelRoutePreference {
    fn default() -> Self {
        Self {
            mode: ModelRouteMode::Auto,
            fixed_model: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingMode {
    Auto,
    Enabled,
    Disabled,
}

impl Default for ThinkingMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl ThinkingMode {
    pub fn deepseek_type(&self) -> &'static str {
        match self {
            Self::Auto | Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    pub fn is_effectively_enabled(&self) -> bool {
        matches!(self, Self::Auto | Self::Enabled)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    #[serde(alias = "low", alias = "medium")]
    High,
    #[serde(alias = "xhigh")]
    Max,
}

impl Default for ReasoningEffort {
    fn default() -> Self {
        Self::High
    }
}

impl ReasoningEffort {
    pub fn as_deepseek_value(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Max => "max",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ResponseLanguage {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en-US")]
    EnUs,
}

impl Default for ResponseLanguage {
    fn default() -> Self {
        Self::EnUs
    }
}

impl ResponseLanguage {
    pub fn prompt_instruction(&self) -> &'static str {
        match self {
            Self::ZhCn => {
                "[Response Language]\n- Use Simplified Chinese for natural-language reasoning_content, task reasoning notes, clarification questions, and final answer body.\n- Keep JSON keys, enum values, tool names, file paths, code, commands, schema literals, and quoted source text exactly as required; do not translate those literals."
            }
            Self::EnUs => {
                "[Response Language]\n- Use English for natural-language reasoning_content, task reasoning notes, clarification questions, and final answer body.\n- Keep JSON keys, enum values, tool names, file paths, code, commands, schema literals, and quoted source text exactly as required; do not translate those literals."
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThinkingConfig {
    #[serde(default)]
    pub mode: ThinkingMode,
    #[serde(default)]
    pub reasoning_effort: ReasoningEffort,
    #[serde(default = "default_true")]
    pub store_reasoning_content: bool,
    #[serde(default = "default_true")]
    pub expose_reasoning_to_ui: bool,
}

impl Default for ThinkingConfig {
    fn default() -> Self {
        Self {
            mode: ThinkingMode::Auto,
            reasoning_effort: ReasoningEffort::High,
            store_reasoning_content: true,
            expose_reasoning_to_ui: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoicePolicy {
    Auto,
    None,
    Required,
}

impl Default for ToolChoicePolicy {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderToolsetMode {
    MinimalDecision,
    DomainScoped,
    StateAwareExpanded,
    IndexedGroups,
    Rc0FullVisible,
    FullRegistered,
}

impl Default for ProviderToolsetMode {
    fn default() -> Self {
        Self::Rc0FullVisible
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCallingConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub strict_mode: bool,
    #[serde(default)]
    pub tool_choice: ToolChoicePolicy,
    #[serde(default)]
    pub toolset_mode: ProviderToolsetMode,
    #[serde(default = "default_max_provider_tools")]
    pub max_provider_tools_per_request: usize,
    #[serde(default = "default_max_provider_subturns")]
    pub max_provider_subturns: usize,
    #[serde(default = "default_max_tool_calls_per_subturn")]
    pub max_tool_calls_per_subturn: usize,
    #[serde(default = "default_max_tool_calls_per_task")]
    pub max_tool_calls_per_task: usize,
    #[serde(default = "default_max_tool_calls_per_chat_turn")]
    pub max_tool_calls_per_chat_turn: usize,
    #[serde(default = "default_max_chat_read_bytes_per_turn")]
    pub max_chat_read_bytes_per_turn: u64,
    #[serde(default = "default_max_chat_read_tokens_per_turn")]
    pub max_chat_read_tokens_per_turn: u64,
    #[serde(default)]
    pub allow_parallel_readonly: bool,
}

impl Default for ToolCallingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strict_mode: true,
            tool_choice: ToolChoicePolicy::Required,
            toolset_mode: ProviderToolsetMode::Rc0FullVisible,
            max_provider_tools_per_request: default_max_provider_tools(),
            max_provider_subturns: default_max_provider_subturns(),
            max_tool_calls_per_subturn: default_max_tool_calls_per_subturn(),
            max_tool_calls_per_task: default_max_tool_calls_per_task(),
            max_tool_calls_per_chat_turn: default_max_tool_calls_per_chat_turn(),
            max_chat_read_bytes_per_turn: default_max_chat_read_bytes_per_turn(),
            max_chat_read_tokens_per_turn: default_max_chat_read_tokens_per_turn(),
            allow_parallel_readonly: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TokenBudgetConfig {
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub max_input_tokens: Option<u64>,
    #[serde(default)]
    pub max_input_bytes: Option<u64>,
}

impl Default for TokenBudgetConfig {
    fn default() -> Self {
        Self {
            max_tokens: None,
            max_input_tokens: None,
            max_input_bytes: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelInvocationConfig {
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub model_route: ModelRoutePreference,
    #[serde(default)]
    pub decision_protocol: TaskAgentDecisionProtocol,
    #[serde(default)]
    pub thinking: ThinkingConfig,
    #[serde(default)]
    pub tool_calling: ToolCallingConfig,
    #[serde(default)]
    pub input_budget: TokenBudgetConfig,
    #[serde(default)]
    pub output_budget: TokenBudgetConfig,
    #[serde(default)]
    pub context_window: ContextWindowControlConfig,
    #[serde(default)]
    pub response_language: ResponseLanguage,
}

impl Default for ModelInvocationConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model_route: ModelRoutePreference::default(),
            decision_protocol: TaskAgentDecisionProtocol::ProviderNativeToolCalls,
            thinking: ThinkingConfig::default(),
            tool_calling: ToolCallingConfig::default(),
            input_budget: TokenBudgetConfig::default(),
            output_budget: TokenBudgetConfig::default(),
            context_window: ContextWindowControlConfig::default(),
            response_language: ResponseLanguage::default(),
        }
    }
}

impl ModelInvocationConfig {
    pub fn enforce_task_agent_provider_native_tools(&mut self) {
        self.decision_protocol = TaskAgentDecisionProtocol::ProviderNativeToolCalls;
        self.tool_calling.enabled = true;
        self.tool_calling.strict_mode = true;
        self.tool_calling.tool_choice = ToolChoicePolicy::Required;
    }

    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Ok(value) = env::var("SUPERNOVA_DEEPSEEK_THINKING") {
            config.thinking.mode = parse_thinking_mode(&value);
        }
        if let Ok(value) = env::var("SUPERNOVA_DEEPSEEK_REASONING_EFFORT") {
            config.thinking.reasoning_effort = parse_reasoning_effort(&value);
        }
        if let Ok(value) = env::var("SUPERNOVA_DEEPSEEK_TOOL_CALLS") {
            match value.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "yes" | "on" | "native" | "provider_native" => {
                    config.enforce_task_agent_provider_native_tools();
                }
                _ => {
                    config.enforce_task_agent_provider_native_tools();
                }
            }
        }
        if let Ok(value) = env::var("SUPERNOVA_CONTEXT_WINDOW_ENABLED")
            .or_else(|_| env::var("SUPERNOVA_CONTEXT_WINDOW"))
        {
            config.context_window.enabled = parse_bool_enabled(&value);
        }
        if let Ok(value) = env::var("SUPERNOVA_MAX_TOOL_CALLS_PER_CHAT_TURN") {
            if let Ok(parsed) = value.parse::<usize>() {
                config.tool_calling.max_tool_calls_per_chat_turn = parsed.max(1);
            }
        }
        if let Ok(value) = env::var("SUPERNOVA_MAX_CHAT_READ_BYTES_PER_TURN") {
            if let Ok(parsed) = value.parse::<u64>() {
                config.tool_calling.max_chat_read_bytes_per_turn = parsed.max(1);
            }
        }
        if let Ok(value) = env::var("SUPERNOVA_MAX_CHAT_READ_TOKENS_PER_TURN") {
            if let Ok(parsed) = value.parse::<u64>() {
                config.tool_calling.max_chat_read_tokens_per_turn = parsed.max(1);
            }
        }
        config.enforce_task_agent_provider_native_tools();
        config
    }

    pub fn effective_model_for_operation(
        &self,
        provider: &dyn ModelProvider,
        operation: &ModelOperation,
        provider_snapshot: &Value,
    ) -> String {
        match self.model_route.mode {
            ModelRouteMode::Auto => provider.model_name_for_operation(operation),
            ModelRouteMode::Flash => provider_snapshot
                .pointer("/routing/simple_model")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| provider.model_name_for_operation(operation)),
            ModelRouteMode::Pro => provider_snapshot
                .pointer("/routing/complex_model")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| provider.model_name_for_operation(operation)),
            ModelRouteMode::Fixed => self
                .model_route
                .fixed_model
                .as_ref()
                .filter(|model| !model.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| provider.model_name_for_operation(operation)),
        }
    }

    pub fn canonical_model_id(&self) -> String {
        match self.model_route.mode {
            ModelRouteMode::Auto => "auto".to_string(),
            ModelRouteMode::Flash => "deepseek-v4-flash".to_string(),
            ModelRouteMode::Pro => "deepseek-v4-pro".to_string(),
            ModelRouteMode::Fixed => self
                .model_route
                .fixed_model
                .as_ref()
                .filter(|model| !model.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "fixed".to_string()),
        }
    }

    pub fn redacted_binding_summary(&self) -> Value {
        json!({
            "schema": "supernova_model_config_binding.v1",
            "provider_id": self.provider.clone(),
            "model_id": self.canonical_model_id(),
            "model_route": self.model_route.clone(),
            "thinking": {
                "mode": self.thinking.mode.clone(),
                "enabled": self.thinking.mode.is_effectively_enabled(),
                "reasoning_effort": self.thinking.reasoning_effort.clone(),
            },
            "max_output_tokens": self.output_budget.max_tokens,
            "strict_tools": self.tool_calling.strict_mode,
            "provider_tool_calls_enabled": self.tool_calling.enabled,
            "tool_choice": self.tool_calling.tool_choice.clone(),
            "decision_protocol": self.decision_protocol.clone(),
            "response_language": self.response_language,
            "redacted": true,
        })
    }

    pub fn apply_budget_overrides(&self, budget: &mut ModelBudget) {
        if let Some(max_input_bytes) = self.input_budget.max_input_bytes {
            budget.max_input_bytes = max_input_bytes;
        } else if let Some(max_input_tokens) = self.input_budget.max_input_tokens {
            budget.max_input_bytes = max_input_tokens.saturating_mul(4).max(1);
        }
        if let Some(max_tokens) = self.output_budget.max_tokens {
            budget.max_output_tokens = max_tokens;
        }
    }

    pub fn sampling_ignored_by_provider(&self) -> bool {
        self.thinking.mode.is_effectively_enabled()
    }
}

pub fn estimate_text_tokens_conservative(text: &str) -> u64 {
    if text.is_empty() {
        return 0;
    }
    let chars = text.chars().count() as u64;
    let bytes = text.len() as u64;
    chars.max(bytes.saturating_add(3) / 4)
}

fn parse_thinking_mode(value: &str) -> ThinkingMode {
    match value.trim().to_ascii_lowercase().as_str() {
        "enabled" | "enable" | "on" | "1" | "true" | "yes" => ThinkingMode::Enabled,
        "disabled" | "disable" | "off" | "0" | "false" | "no" => ThinkingMode::Disabled,
        _ => ThinkingMode::Auto,
    }
}

fn parse_reasoning_effort(value: &str) -> ReasoningEffort {
    match value.trim().to_ascii_lowercase().as_str() {
        "max" | "xhigh" => ReasoningEffort::Max,
        _ => ReasoningEffort::High,
    }
}

fn parse_bool_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "enabled" | "enable"
    )
}

fn default_provider() -> String {
    "deepseek".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_provider_tools() -> usize {
    128
}

fn default_max_provider_subturns() -> usize {
    16
}

fn default_max_tool_calls_per_subturn() -> usize {
    32
}

fn default_max_tool_calls_per_task() -> usize {
    512
}

fn default_max_tool_calls_per_chat_turn() -> usize {
    256
}

fn default_max_chat_read_bytes_per_turn() -> u64 {
    64 * 1024 * 1024
}

fn default_max_chat_read_tokens_per_turn() -> u64 {
    1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacted_binding_summary_exposes_effective_product_model_route() {
        let mut config = ModelInvocationConfig::default();
        config.provider = "deepseek".to_string();
        config.model_route = ModelRoutePreference {
            mode: ModelRouteMode::Pro,
            fixed_model: None,
        };
        config.output_budget.max_tokens = Some(8192);
        config.tool_calling.enabled = true;
        config.tool_calling.strict_mode = true;
        config.decision_protocol = TaskAgentDecisionProtocol::ProviderNativeToolCalls;

        let summary = config.redacted_binding_summary();

        assert_eq!(summary["provider_id"], "deepseek");
        assert_eq!(summary["model_id"], "deepseek-v4-pro");
        assert_eq!(summary["max_output_tokens"], 8192);
        assert_eq!(summary["strict_tools"], true);
        assert_eq!(summary["response_language"], "en-US");
        assert_eq!(summary["redacted"], true);
    }

    #[test]
    fn response_language_defaults_for_legacy_model_invocation_config_json() {
        let config: ModelInvocationConfig = serde_json::from_value(json!({
            "provider": "deepseek",
            "model_route": {"mode": "auto"}
        }))
        .unwrap();

        assert_eq!(config.response_language, ResponseLanguage::EnUs);
    }

    #[test]
    fn task_agent_model_config_defaults_to_provider_native_tools() {
        let config = ModelInvocationConfig::default();

        assert_eq!(
            config.decision_protocol,
            TaskAgentDecisionProtocol::ProviderNativeToolCalls
        );
        assert!(config.tool_calling.enabled);
        assert!(config.tool_calling.strict_mode);
        assert_eq!(config.tool_calling.tool_choice, ToolChoicePolicy::Required);
    }

    #[test]
    fn legacy_json_decision_wire_value_deserializes_to_provider_native() {
        let config: ModelInvocationConfig = serde_json::from_value(json!({
            "decision_protocol": "super_nova_json_decision"
        }))
        .unwrap();

        assert_eq!(
            config.decision_protocol,
            TaskAgentDecisionProtocol::ProviderNativeToolCalls
        );
        assert!(config.tool_calling.enabled);
        assert!(config.tool_calling.strict_mode);
    }

    #[test]
    fn provider_native_enforcement_overrides_disabled_tool_calling() {
        let mut config: ModelInvocationConfig = serde_json::from_value(json!({
            "decision_protocol": "super_nova_json_decision",
            "tool_calling": {
                "enabled": false,
                "strict_mode": false,
                "tool_choice": "none"
            }
        }))
        .unwrap();

        config.enforce_task_agent_provider_native_tools();

        assert_eq!(
            config.decision_protocol,
            TaskAgentDecisionProtocol::ProviderNativeToolCalls
        );
        assert!(config.tool_calling.enabled);
        assert!(config.tool_calling.strict_mode);
        assert_eq!(config.tool_calling.tool_choice, ToolChoicePolicy::Required);
    }

    #[test]
    fn response_language_roundtrips_wire_values() {
        let mut config = ModelInvocationConfig::default();
        config.response_language = ResponseLanguage::ZhCn;

        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["response_language"], "zh-CN");
        let roundtrip: ModelInvocationConfig = serde_json::from_value(value).unwrap();

        assert_eq!(roundtrip.response_language, ResponseLanguage::ZhCn);
    }
}
