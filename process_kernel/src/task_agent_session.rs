use std::io;

use serde_json::json;

use crate::task_agent::{TaskAgent, TaskAgentRunResult};
use crate::CheckpointRef;

#[derive(Clone, Debug)]
pub struct TaskAgentSession {
    agent: TaskAgent,
    max_turns: usize,
}

impl TaskAgentSession {
    pub fn new(agent: TaskAgent, max_turns: usize) -> Self {
        Self { agent, max_turns }
    }

    pub fn run(&self, goal: &str) -> io::Result<TaskAgentRunResult> {
        let initial_context = self.agent.task_context_state()?;
        let mut context = self.agent.start_or_resume_session(goal, &initial_context)?;
        let mut checkpoints: Vec<CheckpointRef> = Vec::new();
        let mut turn_count = context.current_turn_index;
        let mut turn_index = context.next_turn_index.max(1);
        let mut turns_run_this_invocation = 0usize;
        loop {
            if self.max_turns != 0 && turns_run_this_invocation >= self.max_turns {
                break;
            }
            turns_run_this_invocation = turns_run_this_invocation.saturating_add(1);
            turn_count = turn_index;
            let turn_id = self.agent.start_turn(turn_index)?;
            let (observation, observation_checkpoint) =
                self.agent.observe_and_checkpoint(goal, &turn_id)?;
            checkpoints.push(observation_checkpoint);
            let (decision, status) = self
                .agent
                .run_provider_tool_call_loop(goal, &turn_id, &observation)?;
            checkpoints.push(self.agent.checkpoint_after_action(
                &turn_id,
                &observation,
                &decision,
                &status,
            )?);
            self.agent
                .complete_turn(turn_index, &turn_id, &decision.decision_id, &status)?;
            context = self.agent.task_context_state()?;
            self.agent
                .record_task_context_state(&context, "turn_completed")?;
            match status.as_str() {
                "completed" => {
                    return self.agent.result(
                        "completed",
                        checkpoints,
                        context.current_turn_index,
                        None,
                        None,
                    )
                }
                "failed" => {
                    return self.agent.result(
                        "failed",
                        checkpoints,
                        context.current_turn_index,
                        None,
                        Some(json!({"code": "TASK_AGENT_FAILED"})),
                    )
                }
                "blocked" => {
                    return self.agent.result(
                        "blocked",
                        checkpoints,
                        context.current_turn_index,
                        None,
                        Some(json!({"code": "TASK_AGENT_BLOCKED"})),
                    )
                }
                "interrupted" => {
                    return self.agent.result(
                        "interrupted",
                        checkpoints,
                        context.current_turn_index,
                        None,
                        context
                            .last_model_protocol_error
                            .or_else(|| Some(json!({"code": "MODEL_PROTOCOL_INTERRUPTED"}))),
                    )
                }
                "waiting_user" => {
                    return self.agent.result(
                        "waiting_user",
                        checkpoints,
                        context.current_turn_index,
                        Some("user_input".to_string()),
                        None,
                    )
                }
                "waiting_approval" => {
                    return self.agent.result(
                        "waiting_approval",
                        checkpoints,
                        context.current_turn_index,
                        Some("approval".to_string()),
                        None,
                    )
                }
                _ => {}
            }
            turn_index = context.next_turn_index.max(turn_index.saturating_add(1));
        }
        self.agent.fail_max_steps()?;
        self.agent.result(
            "failed",
            checkpoints,
            turn_count.max(context.current_turn_index),
            None,
            Some(json!({"code": "TASK_AGENT_MAX_TURNS"})),
        )
    }
}
