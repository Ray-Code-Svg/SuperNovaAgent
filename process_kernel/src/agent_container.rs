use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::context_window::ContextWindowControlConfig;
use crate::ModelInvocationConfig;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentContainer {
    pub container_id: String,
    pub title: Option<String>,
    pub workspace_root: PathBuf,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub status: AgentContainerStatus,
    pub default_model_config: ModelInvocationConfig,
    pub context_policy: ContextWindowControlConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentContainerStatus {
    Active,
    Archived,
    Paused,
    Deleted,
}

impl AgentContainerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Paused => "paused",
            Self::Deleted => "deleted",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "archived" => Self::Archived,
            "paused" => Self::Paused,
            "deleted" | "tombstoned" => Self::Deleted,
            _ => Self::Active,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerTimelineItem {
    pub container_id: String,
    pub item_id: String,
    pub item_kind: ContainerTimelineItemKind,
    pub title: Option<String>,
    pub status: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub ref_id: String,
    pub summary_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContainerTimelineItemKind {
    ChatTurn,
    ChatThread,
    TaskRun,
    Artifact,
    ApprovalRequest,
    MemorySnapshot,
    ContextCompaction,
}

impl ContainerTimelineItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChatTurn => "chat_turn",
            Self::ChatThread => "chat_thread",
            Self::TaskRun => "task_run",
            Self::Artifact => "artifact",
            Self::ApprovalRequest => "approval_request",
            Self::MemorySnapshot => "memory_snapshot",
            Self::ContextCompaction => "context_compaction",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "chat_turn" => Self::ChatTurn,
            "chat_thread" => Self::ChatThread,
            "artifact" => Self::Artifact,
            "approval_request" => Self::ApprovalRequest,
            "memory_snapshot" => Self::MemorySnapshot,
            "context_compaction" => Self::ContextCompaction,
            _ => Self::TaskRun,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryBinding {
    pub binding_id: String,
    pub container_id: String,
    pub memory_ref: String,
    pub include_mode: String,
    pub priority: u8,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
