use local_runtime_protocol::{
    ModelConfig, ModelConfigDescriptor, ModelConfigOption, ModelProviderDescriptor,
};

use crate::state::product_db::ProductDb;

#[derive(Clone)]
pub struct ModelConfigService {
    db: ProductDb,
}

impl ModelConfigService {
    pub fn new(db: ProductDb) -> Self {
        Self { db }
    }

    pub fn descriptor(&self) -> ModelConfigDescriptor {
        let active = self
            .db
            .get_model_config()
            .ok()
            .flatten()
            .map(normalize_model_config)
            .unwrap_or_else(default_model_config);
        descriptor_for(active)
    }

    pub fn update(&self, request: ModelConfig) -> rusqlite::Result<ModelConfigDescriptor> {
        validate_model_config(&request)?;
        let active = self.db.save_model_config(&normalize_model_config(request))?;
        Ok(descriptor_for(active))
    }
}

fn descriptor_for(active: ModelConfig) -> ModelConfigDescriptor {
    let model_options = vec![
        model_option(
            "deepseek-v4-flash",
            "DeepSeek V4 Flash",
            "Low-latency DeepSeek V4 route for everyday chat and task execution.",
        ),
        model_option(
            "deepseek-v4-pro",
            "DeepSeek V4 Pro",
            "Higher-capability DeepSeek V4 route for complex reasoning and longer work.",
        ),
    ];
    let model_label = option_label(&model_options, &active.model);
    ModelConfigDescriptor {
        user_summary: format!(
            "{} / {} / max output {} / {} tools",
            model_label,
            option_label(&reasoning_effort_options(), &active.reasoning_effort),
            active
                .token_budget
                .map(|value| value.to_string())
                .unwrap_or_else(|| "default".into()),
            if active.strict_tools {
                "strict"
            } else {
                "standard"
            }
        ),
        active,
        providers: vec![ModelProviderDescriptor {
            provider: "deepseek".into(),
            display_name: "DeepSeek".into(),
            models: model_options
                .iter()
                .map(|option| option.value.clone())
                .collect(),
            model_options,
            supports_thinking: true,
            supports_strict_tools: false,
        }],
        thinking_options: vec![
            model_option("auto", "Auto", "Use the runtime default thinking mode."),
            model_option(
                "enabled",
                "Enabled",
                "Request explicit thinking when the provider supports it.",
            ),
            model_option(
                "disabled",
                "Disabled",
                "Prefer direct answers without explicit thinking.",
            ),
        ],
        reasoning_effort_options: reasoning_effort_options(),
        token_budget_min: 1,
        token_budget_max: 131_072,
        token_budget_default: 65_536,
        strict_tools_label: "Strict provider tools".into(),
        strict_tools_description:
            "Require provider tool calls to map to registered Product Runtime capabilities.".into(),
        advanced_defaults_collapsed: true,
    }
}

fn default_model_config() -> ModelConfig {
    ModelConfig {
        provider: "deepseek".into(),
        model: "deepseek-v4-flash".into(),
        thinking: "auto".into(),
        reasoning_effort: "high".into(),
        token_budget: Some(65_536),
        strict_tools: true,
    }
}

fn validate_model_config(config: &ModelConfig) -> rusqlite::Result<()> {
    if config.provider != "deepseek" {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported provider '{}'",
            config.provider
        )));
    }
    let supported_models = ["deepseek-v4-flash", "deepseek-v4-pro"];
    if !supported_models.contains(&config.model.as_str()) {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported model '{}'",
            config.model
        )));
    }
    let supported_thinking = ["auto", "enabled", "disabled"];
    if !supported_thinking.contains(&config.thinking.as_str()) {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported thinking '{}'",
            config.thinking
        )));
    }
    let supported_reasoning = ["standard", "high", "max"];
    if !supported_reasoning.contains(&config.reasoning_effort.as_str()) {
        return Err(rusqlite::Error::InvalidParameterName(format!(
            "unsupported reasoning_effort '{}'",
            config.reasoning_effort
        )));
    }
    if let Some(tokens) = config.token_budget {
        if tokens == 0 || tokens > 131_072 {
            return Err(rusqlite::Error::InvalidParameterName(
                "max output token budget must be between 1 and 131072".into(),
            ));
        }
    }
    Ok(())
}

fn normalize_model_config(mut config: ModelConfig) -> ModelConfig {
    if config.provider != "deepseek" {
        config.provider = "deepseek".into();
    }
    if !["deepseek-v4-flash", "deepseek-v4-pro"].contains(&config.model.as_str()) {
        config.model = "deepseek-v4-flash".into();
    }
    if !["auto", "enabled", "disabled"].contains(&config.thinking.as_str()) {
        config.thinking = "auto".into();
    }
    if !["standard", "high", "max"].contains(&config.reasoning_effort.as_str()) {
        config.reasoning_effort = "high".into();
    }
    if config
        .token_budget
        .map(|value| value == 0 || value > 131_072)
        .unwrap_or(false)
    {
        config.token_budget = Some(65_536);
    }
    config.strict_tools = true;
    config
}

fn model_option(value: &str, label: &str, description: &str) -> ModelConfigOption {
    ModelConfigOption {
        value: value.into(),
        label: label.into(),
        description: description.into(),
    }
}

fn reasoning_effort_options() -> Vec<ModelConfigOption> {
    vec![
        model_option(
            "standard",
            "Standard",
            "Balanced latency and reasoning depth.",
        ),
        model_option("high", "High", "Deeper reasoning for complex tasks."),
        model_option("max", "Max", "Maximum reasoning depth for difficult tasks."),
    ]
}

fn option_label(options: &[ModelConfigOption], value: &str) -> String {
    options
        .iter()
        .find(|option| option.value == value)
        .map(|option| option.label.clone())
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_uses_current_deepseek_v4_models() {
        let descriptor = descriptor_for(default_model_config());
        let provider = descriptor.providers.first().unwrap();
        assert_eq!(
            provider.models,
            vec!["deepseek-v4-flash", "deepseek-v4-pro"]
        );
        assert_eq!(provider.model_options[0].label, "DeepSeek V4 Flash");
        assert_eq!(provider.model_options[1].label, "DeepSeek V4 Pro");
        assert!(!provider.models.iter().any(|model| model == "deepseek-chat"));
        assert!(!provider
            .models
            .iter()
            .any(|model| model == "deepseek-reasoner"));
        assert_eq!(descriptor.token_budget_default, 65_536);
        assert_eq!(descriptor.token_budget_max, 131_072);
        assert!(!provider.supports_strict_tools);
        assert!(descriptor.user_summary.contains("DeepSeek V4 Flash"));
        assert!(descriptor.user_summary.contains("max output"));
    }

    #[test]
    fn validation_rejects_legacy_models_and_unknown_options() {
        let mut config = default_model_config();
        config.model = "deepseek-chat".into();
        assert!(validate_model_config(&config).is_err());

        let mut config = default_model_config();
        config.thinking = "legacy".into();
        assert!(validate_model_config(&config).is_err());

        let mut config = default_model_config();
        config.reasoning_effort = "extreme".into();
        assert!(validate_model_config(&config).is_err());
    }

    #[test]
    fn normalize_legacy_model_config_for_display() {
        let config = ModelConfig {
            provider: "legacy".into(),
            model: "deepseek-reasoner".into(),
            thinking: "legacy".into(),
            reasoning_effort: "extreme".into(),
            token_budget: Some(0),
            strict_tools: true,
        };
        let normalized = normalize_model_config(config);
        assert_eq!(normalized.provider, "deepseek");
        assert_eq!(normalized.model, "deepseek-v4-flash");
        assert_eq!(normalized.thinking, "auto");
        assert_eq!(normalized.reasoning_effort, "high");
        assert_eq!(normalized.token_budget, Some(65_536));
        assert!(normalized.strict_tools);
    }

    #[test]
    fn normalize_forces_provider_native_tools_for_rc0() {
        let mut config = default_model_config();
        config.strict_tools = false;

        let normalized = normalize_model_config(config);

        assert!(normalized.strict_tools);
    }
}
