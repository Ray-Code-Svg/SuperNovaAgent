use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ReferenceSourceDirective {
    pub source_kind: String,
    pub ref_id: String,
    pub label: Option<String>,
    pub usage: String,
    pub include_mode: String,
    pub selection_source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SourceGuidance {
    pub semantics: String,
    pub materialized_content: bool,
    pub source_scope_enforcement: String,
    pub selected_sources: Vec<ReferenceSourceDirective>,
    pub user_intent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ArtifactDestinationGuidance {
    pub semantics: String,
    pub enforcement: String,
    pub materialized_artifact: bool,
    pub selected_output_dir: String,
    pub label: Option<String>,
}
