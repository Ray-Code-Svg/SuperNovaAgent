use local_runtime_protocol::{
    ContainerBadges, ContainerMessage, ContainerRecord, ContainerSnapshot, ContainerStatus,
    CreateContainerRequest, TaskRecord, UpdateContainerRequest,
};
use supernova_process_kernel::{AgentContainer, AgentContainerStatus};

use crate::kernel::event_projection::timeline_task_to_record;
use crate::kernel::KernelBridge;
use crate::services::task_service::task_status_from_events;
use crate::state::product_db::ProductDb;

#[derive(Clone)]
pub struct ContainerService {
    db: ProductDb,
    kernel: KernelBridge,
}

impl ContainerService {
    pub fn new(db: ProductDb, kernel: KernelBridge) -> Self {
        Self { db, kernel }
    }

    pub fn list(&self) -> rusqlite::Result<Vec<ContainerRecord>> {
        let records = self.db.list_container_projections(false)?;
        self.with_container_badges(records)
    }

    pub fn list_archived(&self) -> rusqlite::Result<Vec<ContainerRecord>> {
        let records = self.db.list_archived_container_projections()?;
        self.with_container_badges(records)
    }

    pub fn create(&self, request: CreateContainerRequest) -> rusqlite::Result<ContainerRecord> {
        let container = self
            .kernel
            .create_container(request.title, request.model_config, request.context_policy)
            .map_err(sql_io)?;
        let record = self.upsert_kernel_container(container)?;
        self.db
            .set_active_container_projection(&record.container_id)
    }

    pub fn get(&self, container_id: &str) -> rusqlite::Result<ContainerRecord> {
        let container = self.kernel.get_container(container_id).map_err(sql_io)?;
        let record = self.upsert_kernel_container(container)?;
        self.hydrate_container_tasks(container_id)?;
        self.with_container_badges(vec![record])
            .map(|mut records| records.remove(0))
    }

    pub fn update(
        &self,
        container_id: &str,
        request: UpdateContainerRequest,
    ) -> rusqlite::Result<ContainerRecord> {
        let status = request
            .status
            .as_ref()
            .map(container_status_to_kernel)
            .transpose()?;
        let container = self
            .kernel
            .update_container(
                container_id,
                request.title,
                status,
                request.model_config,
                request.context_policy,
            )
            .map_err(sql_io)?;
        self.upsert_kernel_container(container)
    }

    pub fn activate(&self, container_id: &str) -> rusqlite::Result<ContainerRecord> {
        let container = self.kernel.get_container(container_id).map_err(sql_io)?;
        let record = self.upsert_kernel_container(container)?;
        if !matches!(
            record.status,
            ContainerStatus::Active | ContainerStatus::Running
        ) {
            return Ok(record);
        }
        self.db.set_active_container_projection(container_id)
    }

    pub fn active_container_id(&self) -> rusqlite::Result<Option<String>> {
        self.hydrate_kernel_containers()?;
        if let Some(container_id) = self.db.active_container_projection_id()? {
            if let Ok(record) = self.db.get_container_projection(&container_id) {
                if matches!(
                    record.status,
                    ContainerStatus::Active | ContainerStatus::Running
                ) {
                    return Ok(Some(container_id));
                }
            }
        }
        let next = self
            .db
            .list_container_projections(false)?
            .into_iter()
            .find(|item| {
                matches!(
                    item.status,
                    ContainerStatus::Active | ContainerStatus::Running
                )
            });
        if let Some(record) = next {
            let active = self
                .db
                .set_active_container_projection(&record.container_id)?;
            return Ok(Some(active.container_id));
        }
        Ok(None)
    }

    pub fn archive(&self, container_id: &str) -> rusqlite::Result<ContainerRecord> {
        let container = self
            .kernel
            .archive_container(container_id)
            .map_err(sql_io)?;
        self.upsert_kernel_container(container)
    }

    pub fn restore(&self, container_id: &str) -> rusqlite::Result<ContainerRecord> {
        let container = self
            .kernel
            .update_container(container_id, None, Some("active".into()), None, None)
            .map_err(sql_io)?;
        self.upsert_kernel_container(container)
    }

    pub fn delete(&self, container_id: &str) -> rusqlite::Result<ContainerRecord> {
        let container = self
            .kernel
            .update_container(container_id, None, Some("deleted".into()), None, None)
            .map_err(sql_io)?;
        self.upsert_kernel_container(container)
    }

    pub fn messages(&self, container_id: &str) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.db.list_container_messages(container_id)
    }

    pub fn messages_page(
        &self,
        container_id: &str,
        lane: Option<&local_runtime_protocol::MessageLane>,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.db.list_projected_container_messages_page_for_lane(
            container_id,
            lane,
            after_event_id,
            limit,
        )
    }

    pub fn runtime_messages_page(
        &self,
        after_event_id: Option<i64>,
        limit: Option<usize>,
    ) -> rusqlite::Result<Vec<ContainerMessage>> {
        self.db.list_runtime_messages_page(after_event_id, limit)
    }

    pub fn snapshot(&self, container_id: &str) -> rusqlite::Result<ContainerSnapshot> {
        self.hydrate_container_tasks(container_id)?;
        let mut container = self.get(container_id)?;
        container.badges = self.container_badges(container_id)?;
        Ok(ContainerSnapshot {
            container,
            messages: self.db.list_container_messages(container_id)?,
            chat_threads: self.db.list_chat_threads(container_id)?,
            tasks: self.db.list_tasks(container_id)?,
            context_pack: self.db.get_context_pack(container_id)?,
        })
    }

    fn hydrate_kernel_containers(&self) -> rusqlite::Result<()> {
        let containers = self.kernel.list_containers().map_err(sql_io)?;
        for container in containers {
            let _ = self.upsert_kernel_container(container)?;
        }
        Ok(())
    }

    fn upsert_kernel_container(
        &self,
        container: AgentContainer,
    ) -> rusqlite::Result<ContainerRecord> {
        let record = kernel_container_to_record(&self.db.workspace_uid, container)?;
        self.db.upsert_container_projection(&record)
    }

    fn with_container_badges(
        &self,
        mut records: Vec<ContainerRecord>,
    ) -> rusqlite::Result<Vec<ContainerRecord>> {
        for record in &mut records {
            record.badges = self.container_badges(&record.container_id)?;
        }
        Ok(records)
    }

    fn hydrate_container_tasks(&self, container_id: &str) -> rusqlite::Result<()> {
        if let Ok(items) = self.kernel.list_container_tasks(container_id, 500) {
            for item in items {
                let record =
                    self.merge_existing_task_record(timeline_task_to_record(container_id, item))?;
                let _ = self.db.upsert_task(&record)?;
            }
        }

        for task in self.db.list_tasks(container_id)? {
            let Some(job_id) = task.job_id.as_deref() else {
                continue;
            };
            let Ok(events) = self.kernel.read_process_events(job_id) else {
                continue;
            };
            let Some((status, updated_at_ms)) = task_status_from_events(&events) else {
                continue;
            };
            if task.status == status && task.updated_at_ms >= updated_at_ms {
                continue;
            }
            let mut updated = task;
            updated.status = status;
            updated.updated_at_ms = updated.updated_at_ms.max(updated_at_ms);
            let _ = self.db.upsert_task(&updated)?;
        }
        Ok(())
    }

    fn merge_existing_task_record(&self, mut record: TaskRecord) -> rusqlite::Result<TaskRecord> {
        if let Ok(existing) = self.db.get_task(&record.task_id) {
            record.created_at_ms = existing.created_at_ms.min(record.created_at_ms);
            record.updated_at_ms = existing.updated_at_ms.max(record.updated_at_ms);
            if record.goal.trim().is_empty() {
                record.goal = existing.goal;
            }
            if record.title.trim().is_empty() || record.title == "Task" {
                record.title = existing.title;
            }
            if record.job_id.is_none() {
                record.job_id = existing.job_id;
            }
        }
        Ok(record)
    }

    fn container_badges(&self, container_id: &str) -> rusqlite::Result<ContainerBadges> {
        let tasks = self.db.list_tasks(container_id)?;
        let mut badges = ContainerBadges::default();
        for task in tasks {
            match task.status.as_str() {
                "running" => badges.running = badges.running.saturating_add(1),
                "blocked" | "failed" | "interrupted" => {
                    badges.blocked = badges.blocked.saturating_add(1)
                }
                _ => {}
            }
            badges.blocked = badges.blocked.saturating_add(task.badges.blocked);
            badges.artifact_ready = badges
                .artifact_ready
                .saturating_add(task.badges.artifact_ready);
        }
        Ok(badges)
    }
}

fn container_status_to_kernel(status: &ContainerStatus) -> rusqlite::Result<String> {
    match status {
        ContainerStatus::Active => Ok("active".into()),
        ContainerStatus::Archived => Ok("archived".into()),
        ContainerStatus::Deleted => Ok("deleted".into()),
        ContainerStatus::Running | ContainerStatus::Approval | ContainerStatus::Blocked => Err(
            rusqlite::Error::InvalidParameterName(
                "running/approval/blocked are UI projections, not Kernel container lifecycle statuses"
                    .into(),
            ),
        ),
    }
}

fn kernel_container_to_record(
    workspace_uid: &str,
    container: AgentContainer,
) -> rusqlite::Result<ContainerRecord> {
    Ok(ContainerRecord {
        container_id: container.container_id,
        workspace_uid: workspace_uid.to_string(),
        title: container.title.unwrap_or_else(|| "Container".to_string()),
        status: match container.status {
            AgentContainerStatus::Active => ContainerStatus::Active,
            AgentContainerStatus::Archived => ContainerStatus::Archived,
            AgentContainerStatus::Deleted => ContainerStatus::Deleted,
            AgentContainerStatus::Paused => ContainerStatus::Active,
        },
        badges: ContainerBadges::default(),
        created_at_ms: container.created_at_ms.max(0) as u128,
        updated_at_ms: container.updated_at_ms.max(0) as u128,
        last_active_at_ms: container.updated_at_ms.max(0) as u128,
        default_model_config: Some(
            serde_json::to_value(container.default_model_config)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?,
        ),
        context_policy: Some(
            serde_json::to_value(container.context_policy)
                .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)))?,
        ),
    })
}

fn sql_io(err: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::state::workspace_registry::now_ms;

    fn test_service(name: &str) -> ContainerService {
        let workspace_root = temp_root(&format!("{name}_workspace"));
        let state_root = temp_root(&format!("{name}_state"));
        let provider_root = temp_root(&format!("{name}_provider"));
        let db = ProductDb::open(&state_root, format!("workspace_{name}")).unwrap();
        let kernel = KernelBridge::new(workspace_root, state_root, provider_root);
        ContainerService::new(db, kernel)
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("supernova_{name}_{}", now_ms()));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn create_request(title: &str) -> CreateContainerRequest {
        CreateContainerRequest {
            workspace_uid: None,
            title: Some(title.to_string()),
            model_config: None,
            context_policy: None,
        }
    }

    #[test]
    fn list_projects_running_task_badge_from_product_projection() {
        let service = test_service("running_badge");
        let container = service.create(create_request("Badge Container")).unwrap();
        service
            .db
            .upsert_task(&local_runtime_protocol::TaskRecord {
                task_id: "job_running".into(),
                container_id: container.container_id.clone(),
                job_id: Some("job_running".into()),
                title: "Running".into(),
                goal: "long task".into(),
                status: "running".into(),
                badges: Default::default(),
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();

        let containers = service.list().unwrap();
        let projected = containers
            .into_iter()
            .find(|item| item.container_id == container.container_id)
            .unwrap();
        assert_eq!(projected.badges.running, 1);
    }

    #[test]
    fn list_uses_product_projection_without_hydrating_kernel_containers() {
        let service = test_service("list_projection_only");
        let kernel_container = service
            .kernel
            .create_container(Some("Kernel only".into()), None, None)
            .unwrap();

        let listed = service.list().unwrap();

        assert!(listed
            .iter()
            .all(|item| item.container_id != kernel_container.container_id));
    }

    #[test]
    fn container_badge_ignores_stale_approval_badge_after_waiting_user() {
        let service = test_service("stale_approval_badge");
        let container = service.create(create_request("Badge Container")).unwrap();
        service
            .db
            .upsert_task(&local_runtime_protocol::TaskRecord {
                task_id: "job_waiting_user".into(),
                container_id: container.container_id.clone(),
                job_id: Some("job_waiting_user".into()),
                title: "Waiting user".into(),
                goal: "clarify deletion target".into(),
                status: "waiting_user".into(),
                badges: local_runtime_protocol::ContainerBadges {
                    approval: 1,
                    ..Default::default()
                },
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();

        let badges = service.container_badges(&container.container_id).unwrap();
        assert_eq!(badges.approval, 0);
    }

    #[test]
    fn container_lifecycle_is_kernel_backed_and_product_db_is_projection() {
        let service = test_service("container_authority");

        let created = service.create(create_request("Kernel Container")).unwrap();
        assert_eq!(created.title, "Kernel Container");
        assert_eq!(created.status, ContainerStatus::Active);
        assert_eq!(
            service.active_container_id().unwrap().as_deref(),
            Some(created.container_id.as_str())
        );
        assert_eq!(
            service
                .kernel
                .get_container(&created.container_id)
                .unwrap()
                .status,
            AgentContainerStatus::Active
        );

        let archived = service.archive(&created.container_id).unwrap();
        assert_eq!(archived.status, ContainerStatus::Archived);
        assert_eq!(
            service
                .kernel
                .get_container(&created.container_id)
                .unwrap()
                .status,
            AgentContainerStatus::Archived
        );
        assert!(service
            .list()
            .unwrap()
            .iter()
            .all(|item| item.container_id != created.container_id));
        assert!(service
            .list_archived()
            .unwrap()
            .iter()
            .any(|item| item.container_id == created.container_id));

        let restored = service.restore(&created.container_id).unwrap();
        assert_eq!(restored.status, ContainerStatus::Active);
        assert_eq!(
            service
                .kernel
                .get_container(&created.container_id)
                .unwrap()
                .status,
            AgentContainerStatus::Active
        );

        let deleted = service.delete(&created.container_id).unwrap();
        assert_eq!(deleted.status, ContainerStatus::Deleted);
        assert_eq!(
            service
                .kernel
                .get_container(&created.container_id)
                .unwrap()
                .status,
            AgentContainerStatus::Deleted
        );
        assert!(service
            .list()
            .unwrap()
            .iter()
            .all(|item| item.container_id != created.container_id));
        assert!(service
            .list_archived()
            .unwrap()
            .iter()
            .all(|item| item.container_id != created.container_id));
    }

    #[test]
    fn active_container_recovers_from_kernel_projection_without_ui_fallback() {
        let service = test_service("container_recent_active");
        let first = service.create(create_request("First")).unwrap();
        let second = service.create(create_request("Second")).unwrap();
        service.archive(&second.container_id).unwrap();

        let recovered = service.active_container_id().unwrap();
        assert_eq!(recovered.as_deref(), Some(first.container_id.as_str()));
        assert_ne!(recovered.as_deref(), Some(second.container_id.as_str()));
    }
}
