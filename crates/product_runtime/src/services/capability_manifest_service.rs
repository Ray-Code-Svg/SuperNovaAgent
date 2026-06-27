use local_runtime_protocol::{
    ComposerTokenDescriptor, ContextConfigDescriptor, UiActionDescriptor, UiCapabilityManifest,
    UiCommandDescriptor,
};

#[derive(Clone)]
pub struct CapabilityManifestService;

impl CapabilityManifestService {
    pub fn new() -> Self {
        Self
    }

    pub fn manifest(
        &self,
        model_config: local_runtime_protocol::ModelConfigDescriptor,
    ) -> UiCapabilityManifest {
        UiCapabilityManifest {
            commands: vec![
                command(
                    "command.chat",
                    "Chat",
                    "Switch to AgentChat mode",
                    "mode.switch.chat",
                ),
                command(
                    "command.task",
                    "Task",
                    "Switch to Agent TASK mode",
                    "mode.switch.task",
                ),
                command(
                    "command.model",
                    "Model",
                    "Open model configuration",
                    "model.config.read",
                ),
                command(
                    "command.context",
                    "Context",
                    "Open context pack configuration",
                    "context.pack.read",
                ),
            ],
            workspace_actions: vec![action(
                "workspace.create",
                "Add Workspace",
                "workspace.create",
                "writes_app_state",
            )],
            container_actions: vec![
                action(
                    "container.create",
                    "Add Container",
                    "container.create",
                    "writes_app_state",
                ),
                action(
                    "container.archive",
                    "Archive Container",
                    "container.archive",
                    "writes_app_state",
                ),
                action(
                    "container.restore",
                    "Restore Container",
                    "container.restore",
                    "writes_app_state",
                ),
                action(
                    "container.delete",
                    "Delete Container",
                    "container.delete",
                    "destructive_app_state",
                ),
            ],
            composer_tokens: vec![
                token("/", "Command", "command.resolve"),
                token("@", "Source", "workspace.source.pick"),
                token("$", "Artifact target", "artifact.target.pick"),
            ],
            model_config,
            context_config: ContextConfigDescriptor {
                supports_history_chat: true,
                supports_history_task: true,
                supports_container_default_pack: true,
                supports_compaction: true,
            },
            settings: vec![action(
                "settings.open",
                "Settings",
                "app.settings.read",
                "read_app_state",
            )],
        }
    }
}

fn command(
    command_id: &str,
    label: &str,
    description: &str,
    capability_id: &str,
) -> UiCommandDescriptor {
    UiCommandDescriptor {
        command_id: command_id.into(),
        label: label.into(),
        description: description.into(),
        capability_id: capability_id.into(),
    }
}

fn action(
    action_id: &str,
    label: &str,
    capability_id: &str,
    side_effect: &str,
) -> UiActionDescriptor {
    UiActionDescriptor {
        action_id: action_id.into(),
        label: label.into(),
        capability_id: capability_id.into(),
        side_effect: side_effect.into(),
    }
}

fn token(token: &str, label: &str, capability_id: &str) -> ComposerTokenDescriptor {
    ComposerTokenDescriptor {
        token: token.into(),
        label: label.into(),
        capability_id: capability_id.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use local_runtime_protocol::{
        ModelConfig, ModelConfigDescriptor, ModelConfigOption, ModelProviderDescriptor,
    };

    use super::*;

    #[test]
    fn capability_manifest_exposes_single_task_command() {
        let manifest = CapabilityManifestService::new().manifest(model_descriptor());
        let command_ids = manifest
            .commands
            .iter()
            .map(|command| command.command_id.as_str())
            .collect::<Vec<_>>();

        assert!(command_ids.contains(&"command.task"));
        assert!(!command_ids.contains(&"command.start_task"));
        assert_eq!(
            command_ids
                .iter()
                .filter(|command_id| **command_id == "command.task" || **command_id == "command.start_task")
                .count(),
            1
        );
        assert_eq!(
            command_ids.iter().collect::<HashSet<_>>().len(),
            command_ids.len()
        );
    }

    fn model_descriptor() -> ModelConfigDescriptor {
        ModelConfigDescriptor {
            active: ModelConfig {
                provider: "deepseek".into(),
                model: "deepseek-v4-flash".into(),
                thinking: "auto".into(),
                reasoning_effort: "high".into(),
                token_budget: Some(4096),
                strict_tools: true,
            },
            providers: vec![ModelProviderDescriptor {
                provider: "deepseek".into(),
                display_name: "DeepSeek".into(),
                models: vec!["deepseek-v4-flash".into()],
                model_options: vec![ModelConfigOption {
                    value: "deepseek-v4-flash".into(),
                    label: "DeepSeek V4 Flash".into(),
                    description: "test model".into(),
                }],
                supports_thinking: true,
                supports_strict_tools: false,
            }],
            thinking_options: Vec::new(),
            reasoning_effort_options: Vec::new(),
            token_budget_min: 1,
            token_budget_max: 65536,
            token_budget_default: 4096,
            strict_tools_label: "Strict provider tools".into(),
            strict_tools_description: "Require provider tool calls to map to registered capabilities.".into(),
            advanced_defaults_collapsed: true,
            user_summary: "DeepSeek V4 Flash".into(),
        }
    }
}
