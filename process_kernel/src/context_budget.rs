use crate::model_runtime::{ModelBudget, ModelOperation, ModelProvider};

const ESTIMATED_BYTES_PER_TOKEN: u64 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelContextProfile {
    pub provider: String,
    pub model: String,
    pub max_context_tokens: u32,
    pub max_decision_input_bytes: u64,
    pub max_generation_input_bytes: u64,
    pub max_dataset_input_bytes: u64,
    pub default_output_tokens: u32,
    pub long_output_tokens: u32,
}

impl ModelContextProfile {
    pub fn for_provider(provider: &dyn ModelProvider, operation: &ModelOperation) -> Self {
        let provider_name = provider.provider_name().to_string();
        let model = provider.model_name_for_operation(operation);
        let lower_provider = provider_name.to_ascii_lowercase();
        let lower_model = model.to_ascii_lowercase();
        if lower_provider.contains("deepseek") || lower_model.contains("deepseek") {
            return Self {
                provider: provider_name,
                model,
                max_context_tokens: 1_000_000,
                max_decision_input_bytes: input_bytes_for_window(1_000_000, 65_536),
                max_generation_input_bytes: input_bytes_for_window(1_000_000, 65_536),
                max_dataset_input_bytes: input_bytes_for_window(1_000_000, 65_536),
                default_output_tokens: 65_536,
                long_output_tokens: 65_536,
            };
        }
        Self {
            provider: provider_name,
            model,
            max_context_tokens: 128_000,
            max_decision_input_bytes: input_bytes_for_window(128_000, 8_192),
            max_generation_input_bytes: input_bytes_for_window(128_000, 16_384),
            max_dataset_input_bytes: input_bytes_for_window(128_000, 8_192),
            default_output_tokens: 8_192,
            long_output_tokens: 16_384,
        }
    }

    pub fn budget_for(&self, operation: &ModelOperation) -> ModelBudget {
        let mut budget = ModelBudget::default();
        match operation {
            ModelOperation::DecideNextAction
            | ModelOperation::ChatTurn
            | ModelOperation::CompactContainerContext
            | ModelOperation::CompactChatContext
            | ModelOperation::CompactTaskContext => {
                budget.max_input_bytes = self.max_decision_input_bytes;
                budget.max_output_tokens = self.default_output_tokens;
                budget.timeout_ms = 90_000;
            }
            ModelOperation::ExtractJson | ModelOperation::Summarize | ModelOperation::Audit => {
                budget.max_input_bytes = self.max_dataset_input_bytes;
                budget.max_output_tokens = self.default_output_tokens;
                budget.timeout_ms = 180_000;
            }
            ModelOperation::Rewrite | ModelOperation::GenerateArtifact => {
                budget.max_input_bytes = self.max_generation_input_bytes;
                budget.max_output_tokens = self.long_output_tokens;
                budget.timeout_ms = 300_000;
            }
            ModelOperation::RenderEntityReply => {
                budget.max_input_bytes = self.max_decision_input_bytes;
                budget.max_output_tokens = self.default_output_tokens;
                budget.timeout_ms = 120_000;
            }
        }
        budget
    }

    pub fn clamp_budget_to_context_window(&self, budget: &mut ModelBudget) {
        if budget.max_output_tokens >= self.max_context_tokens {
            budget.max_output_tokens = self.max_context_tokens.saturating_sub(1).max(1);
        }
        let max_input_bytes =
            input_bytes_for_window(self.max_context_tokens, budget.max_output_tokens);
        budget.max_input_bytes = budget.max_input_bytes.min(max_input_bytes);
    }
}

pub fn context_window_tokens_for_budget(budget: &ModelBudget) -> u64 {
    budget
        .max_input_bytes
        .saturating_div(ESTIMATED_BYTES_PER_TOKEN)
        .saturating_add(budget.max_output_tokens as u64)
        .max(1)
}

const fn input_bytes_for_window(max_context_tokens: u32, output_tokens: u32) -> u64 {
    let available_input_tokens = if max_context_tokens > output_tokens {
        (max_context_tokens - output_tokens) as u64
    } else {
        1
    };
    available_input_tokens * ESTIMATED_BYTES_PER_TOKEN
}
