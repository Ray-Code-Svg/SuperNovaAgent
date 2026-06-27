use std::io;
use std::sync::Arc;

use crate::model_config::ModelInvocationConfig;
use crate::model_runtime::{ModelProvider, ModelStreamSink};
use crate::task_agent::{TaskAgent, TaskAgentRunResult};
use crate::task_agent_session::TaskAgentSession;
use crate::{CapabilityToken, ProcessTruthStore, WorkspaceGuard};

#[derive(Clone, Debug)]
pub struct TaskAgentRuntime {
    agent: TaskAgent,
    max_turns: usize,
}

impl TaskAgentRuntime {
    pub fn new(
        guard: WorkspaceGuard,
        truth: ProcessTruthStore,
        token: CapabilityToken,
        runtime_id: impl Into<String>,
        model_provider: Option<Arc<dyn ModelProvider>>,
        model_config: ModelInvocationConfig,
        model_invocation_config_ref: Option<String>,
        model_stream_sink: Option<Arc<dyn ModelStreamSink>>,
    ) -> Self {
        Self {
            agent: TaskAgent::new_default(
                guard,
                truth,
                token,
                runtime_id,
                model_provider,
                model_config,
                model_invocation_config_ref,
                model_stream_sink,
            ),
            max_turns: 16,
        }
    }

    pub fn with_max_steps(self, max_steps: usize) -> Self {
        self.with_max_turns(max_steps)
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns = max_turns;
        self
    }

    pub fn run(&self, goal: &str) -> io::Result<TaskAgentRunResult> {
        TaskAgentSession::new(self.agent.clone(), self.max_turns).run(goal)
    }

    pub fn runtime_id(&self) -> &str {
        self.agent.runtime_id()
    }
}
