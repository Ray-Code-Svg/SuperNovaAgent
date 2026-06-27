use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceSourceDirective {
    pub source_kind: String,
    pub ref_id: String,
    pub label: Option<String>,
    pub usage: String,
    pub include_mode: String,
    pub selection_source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceGuidance {
    pub semantics: String,
    pub materialized_content: bool,
    pub source_scope_enforcement: String,
    pub selected_sources: Vec<ReferenceSourceDirective>,
    pub user_intent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactDestinationGuidance {
    pub semantics: String,
    pub enforcement: String,
    pub materialized_artifact: bool,
    pub selected_output_dir: String,
    pub label: Option<String>,
}

impl SourceGuidance {
    pub fn is_effective(&self) -> bool {
        !self.selected_sources.is_empty()
    }

    pub fn normalized(mut self) -> Self {
        self.semantics = "model_guidance_only".to_string();
        self.materialized_content = false;
        self.source_scope_enforcement = "none".to_string();
        for source in &mut self.selected_sources {
            if source.usage.trim().is_empty() {
                source.usage = "primary_reference_scope".to_string();
            }
            if source.include_mode.trim().is_empty() {
                source.include_mode = "reference_only".to_string();
            }
            if source.selection_source.trim().is_empty() {
                source.selection_source = "composer_at_token".to_string();
            }
        }
        self
    }

    pub fn provider_visible_text(&self) -> String {
        let sources = self
            .selected_sources
            .iter()
            .map(|source| {
                let label = source.label.as_deref().unwrap_or(&source.ref_id);
                format!(
                    "- kind: {}\n  path: {}\n  label: {}\n  usage: {}\n  include_mode: {}\n  rule: Prefer this source when planning. If file contents or directory listings are needed, inspect them with workspace read capabilities before making claims.",
                    source.source_kind,
                    source.ref_id,
                    label,
                    source.usage,
                    source.include_mode
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "User selected reference sources for this turn/task.\nsemantics: model_guidance_only\nmaterialized_content: false\nsource_scope_enforcement: none\n{}\n{}",
            self.user_intent
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .map(|value| format!("user_intent: {value}"))
                .unwrap_or_else(|| "user_intent: Prefer these reference sources when relevant.".to_string()),
            sources
        )
    }

    pub fn audit_payload(&self, guidance_ref: String) -> serde_json::Value {
        json!({
            "semantics": "model_guidance_only",
            "materialized_content": false,
            "source_scope_enforcement": "none",
            "selected_sources": self.selected_sources,
            "selected_source_count": self.selected_sources.len(),
            "guidance_ref": guidance_ref,
            "user_intent": self.user_intent,
        })
    }
}

impl ArtifactDestinationGuidance {
    pub fn is_effective(&self) -> bool {
        !self.selected_output_dir.trim().is_empty()
    }

    pub fn normalized(mut self) -> Self {
        self.semantics = "model_guidance_only".to_string();
        self.enforcement = "none".to_string();
        self.materialized_artifact = false;
        self
    }

    pub fn provider_visible_text(&self) -> String {
        format!(
            "User selected an output destination for this TASK.\nsemantics: model_guidance_only\nenforcement: none\nmaterialized_artifact: false\nselected_output_dir: {}\nrule: Prefer this workspace directory for final user-visible artifacts when the task requires artifact output. Use Kernel write/preview/verify capabilities for actual writes; do not claim a file was written without a receipt.",
            self.selected_output_dir
        )
    }

    pub fn audit_payload(&self, guidance_ref: String) -> serde_json::Value {
        json!({
            "semantics": "model_guidance_only",
            "enforcement": "none",
            "materialized_artifact": false,
            "selected_output_dir": self.selected_output_dir,
            "label": self.label,
            "guidance_ref": guidance_ref,
        })
    }
}
