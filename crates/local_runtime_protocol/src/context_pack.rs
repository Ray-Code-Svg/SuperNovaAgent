use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContextPack {
    pub context_pack_id: String,
    pub container_id: String,
    pub selected_items: Vec<ContextPackItem>,
    pub excluded_items: Vec<ContextPackItem>,
    pub auto_policy: ContextPackAutoPolicy,
    pub summary_ref: Option<String>,
    pub estimated_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContextPackItem {
    pub item_kind: String,
    pub ref_id: String,
    pub label: Option<String>,
    pub include_mode: String,
    pub priority: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SourceCandidate {
    pub item: ContextPackItem,
    pub source_kind: String,
    pub detail: Option<String>,
    pub selected: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
pub struct SourceCandidateRequest {
    pub q: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContextPackAutoPolicy {
    pub include_recent_chat_turns: usize,
    pub include_recent_tasks: usize,
    pub prefer_summaries: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ContextPackEstimate {
    pub context_pack: ContextPack,
    pub estimated_tokens: u64,
    pub context_window_tokens: u64,
    pub usage_ratio: String,
}
