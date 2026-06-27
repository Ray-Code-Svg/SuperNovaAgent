use std::io;
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{now_ms, to_json_value, AgentProcess, ProcessTruthStore};

pub fn suppress_child_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChildProcessKind {
    SourceDiscovery,
    CorpusExtraction,
    Synthesis,
    MutationPreview,
    Commit,
    Verify,
    ArtifactAudit,
}

impl ChildProcessKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SourceDiscovery => "source_discovery",
            Self::CorpusExtraction => "corpus_extraction",
            Self::Synthesis => "synthesis",
            Self::MutationPreview => "mutation_preview",
            Self::Commit => "commit",
            Self::Verify => "verify",
            Self::ArtifactAudit => "artifact_audit",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChildProcessReceipt {
    pub child_pid: String,
    pub ppid: String,
    pub job_id: String,
    pub kind: ChildProcessKind,
    pub status: String,
    pub input_refs: Vec<String>,
    pub output_refs: Vec<String>,
    pub attempts: u32,
}

#[derive(Clone, Debug)]
pub struct ChildProcessController {
    truth: ProcessTruthStore,
}

impl ChildProcessController {
    pub fn new(truth: ProcessTruthStore) -> Self {
        Self { truth }
    }

    pub fn fork(
        &self,
        ppid: &str,
        kind: ChildProcessKind,
        input_refs: Vec<String>,
        capabilities: Vec<String>,
    ) -> io::Result<AgentProcess> {
        let child_pid = format!("pid_child_{}_{}", kind.as_str(), now_ms());
        let process = AgentProcess {
            pid: child_pid.clone(),
            ppid: Some(ppid.to_string()),
            job_id: self.truth.job_id().to_string(),
            process_type: format!("child_process:{}", kind.as_str()),
            state: "running".to_string(),
            input_refs: input_refs.clone(),
            output_refs: Vec::new(),
            capability_tokens: capabilities,
            budget_ms: None,
            exit_code: None,
        };
        self.truth.register_process(&process)?;
        self.truth.append_event(
            Some(&child_pid),
            "child_process_started",
            json!({
                "child_pid": child_pid,
                "ppid": ppid,
                "kind": kind.as_str(),
                "input_refs": input_refs,
            }),
        )?;
        Ok(process)
    }

    pub fn join(
        &self,
        mut process: AgentProcess,
        kind: ChildProcessKind,
        status: &str,
        output_refs: Vec<String>,
        attempts: u32,
    ) -> io::Result<ChildProcessReceipt> {
        process.state = if status == "success" {
            "completed".to_string()
        } else {
            "failed".to_string()
        };
        process.output_refs = output_refs.clone();
        process.exit_code = Some(if status == "success" { 0 } else { 1 });
        self.truth.register_process(&process)?;
        let receipt = ChildProcessReceipt {
            child_pid: process.pid.clone(),
            ppid: process.ppid.clone().unwrap_or_default(),
            job_id: process.job_id.clone(),
            kind,
            status: status.to_string(),
            input_refs: process.input_refs.clone(),
            output_refs,
            attempts,
        };
        self.truth.append_event(
            Some(&process.pid),
            "child_process_joined",
            to_json_value(&receipt)?,
        )?;
        Ok(receipt)
    }
}
