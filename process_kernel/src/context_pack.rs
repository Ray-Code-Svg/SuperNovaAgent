use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPack {
    pub context_pack_id: String,
    pub container_id: String,
    #[serde(default)]
    pub selected_items: Vec<ContextPackItem>,
    #[serde(default)]
    pub excluded_items: Vec<ContextPackItem>,
    #[serde(default)]
    pub auto_policy: ContextPackAutoPolicy,
    pub summary_ref: Option<String>,
    pub estimated_tokens: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPackItem {
    pub item_kind: ContextPackItemKind,
    pub ref_id: String,
    pub label: Option<String>,
    pub include_mode: ContextPackIncludeMode,
    pub priority: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPackItemKind {
    ChatTurn,
    ChatThread,
    TaskRun,
    TaskArtifact,
    Artifact,
    SourceRef,
    MemorySummary,
    ContainerSummary,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextPackIncludeMode {
    Full,
    Summary,
    MetadataOnly,
    RefOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextPackAutoPolicy {
    #[serde(default = "default_recent_chat_turns")]
    pub include_recent_chat_turns: usize,
    #[serde(default = "default_recent_tasks")]
    pub include_recent_tasks: usize,
    #[serde(default = "default_true")]
    pub prefer_summaries: bool,
}

impl Default for ContextPackAutoPolicy {
    fn default() -> Self {
        Self {
            include_recent_chat_turns: default_recent_chat_turns(),
            include_recent_tasks: default_recent_tasks(),
            prefer_summaries: true,
        }
    }
}

impl ContextPack {
    pub fn empty(context_pack_id: impl Into<String>, container_id: impl Into<String>) -> Self {
        Self {
            context_pack_id: context_pack_id.into(),
            container_id: container_id.into(),
            selected_items: Vec::new(),
            excluded_items: Vec::new(),
            auto_policy: ContextPackAutoPolicy::default(),
            summary_ref: None,
            estimated_tokens: None,
        }
    }
}

impl ContextPackIncludeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Summary => "summary",
            Self::MetadataOnly => "metadata_only",
            Self::RefOnly => "ref_only",
        }
    }
}

fn default_recent_chat_turns() -> usize {
    6
}

fn default_recent_tasks() -> usize {
    3
}

fn default_true() -> bool {
    true
}
