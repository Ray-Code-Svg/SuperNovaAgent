use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use product_runtime::services::run_manager::RunManager;
use product_runtime::state::product_db::ProductDb;

fn temp_root(name: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let root = std::env::temp_dir().join(format!("supernova_process_worker_{name}_{now}"));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn worker_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_supernova-product-runtime"))
}

fn manager(name: &str) -> (RunManager, ProductDb) {
    let workspace = temp_root(&format!("{name}_workspace"));
    let state = temp_root(&format!("{name}_state"));
    let provider = temp_root(&format!("{name}_provider"));
    let db = ProductDb::open(&state, format!("workspace_{name}")).unwrap();
    (
        RunManager::with_process_worker_executable(
            db.clone(),
            worker_exe(),
            workspace,
            state,
            provider,
        ),
        db,
    )
}

fn wait_for_worker_heartbeat_after(db: &ProductDb, run_id: &str, previous: Option<u128>) {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(6) {
        let run = db.get_run(run_id).unwrap();
        let worker_heartbeat_seen = match (run.heartbeat_at_ms, previous) {
            (Some(heartbeat), Some(previous)) => heartbeat > previous,
            (Some(_), None) => true,
            _ => false,
        };
        if run.worker_id.starts_with("process:") && worker_heartbeat_seen {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("worker run {run_id} did not report a process heartbeat");
}

#[test]
fn process_worker_probe_records_distinct_worker_pids_and_heartbeat() {
    let (manager, db) = manager("distinct_workers");
    let first = manager.start_task_run("container_a").unwrap();
    let second = manager.start_task_run("container_b").unwrap();
    let first_manager = manager.clone();
    let second_manager = manager.clone();
    let first_run_id = first.run_id.clone();
    let second_run_id = second.run_id.clone();

    let first_thread =
        thread::spawn(move || first_manager.run_probe_sleep_in_process_worker(&first_run_id, 1300));
    let second_thread = thread::spawn(move || {
        second_manager.run_probe_sleep_in_process_worker(&second_run_id, 1300)
    });

    first_thread.join().unwrap().unwrap();
    second_thread.join().unwrap().unwrap();

    let first = db.get_run(&first.run_id).unwrap();
    let second = db.get_run(&second.run_id).unwrap();
    assert_eq!(first.status, "completed");
    assert_eq!(second.status, "completed");
    assert!(first.worker_id.starts_with("process:"));
    assert!(second.worker_id.starts_with("process:"));
    assert_ne!(first.worker_id, second.worker_id);
    assert!(first.heartbeat_at_ms.is_some());
    assert!(second.heartbeat_at_ms.is_some());
}

#[test]
fn process_worker_terminal_event_completes_run_without_waiting_for_worker_eof() {
    let (manager, db) = manager("terminal_before_eof");
    let run = manager.start_task_run("container_a").unwrap();

    let started = Instant::now();
    manager
        .run_probe_terminal_then_sleep_in_process_worker(&run.run_id, 15_000)
        .unwrap();

    assert!(
        started.elapsed() < Duration::from_secs(8),
        "terminal event should close the run without waiting for worker EOF"
    );
    assert_eq!(db.get_run(&run.run_id).unwrap().status, "completed");
}

#[test]
fn process_worker_cancel_kills_only_matching_run() {
    let (manager, db) = manager("cancel");
    let cancelled = manager.start_task_run("container_a").unwrap();
    let other = manager.start_task_run("container_b").unwrap();
    manager
        .bind_task_run(&cancelled.run_id, "task_cancel", "task_cancel")
        .unwrap();
    manager
        .bind_task_run(&other.run_id, "task_other", "task_other")
        .unwrap();
    let initial_cancelled_heartbeat = db.get_run(&cancelled.run_id).unwrap().heartbeat_at_ms;
    let cancel_manager = manager.clone();
    let cancelled_run_id = cancelled.run_id.clone();
    let handle = thread::spawn(move || {
        cancel_manager.run_probe_sleep_in_process_worker(&cancelled_run_id, 5000)
    });

    wait_for_worker_heartbeat_after(&db, &cancelled.run_id, initial_cancelled_heartbeat);
    let cancel_started = Instant::now();
    let runs = manager.request_cancel_task_run("task_cancel").unwrap();

    assert_eq!(runs.len(), 1);
    assert!(handle.join().unwrap().is_err());
    assert!(
        cancel_started.elapsed() < Duration::from_secs(3),
        "cancel should terminate the worker instead of waiting for probe completion"
    );
    assert_eq!(db.get_run(&cancelled.run_id).unwrap().status, "cancelled");
    assert_eq!(db.get_run(&other.run_id).unwrap().status, "running");
}

#[test]
fn process_worker_crash_marks_run_failed_without_breaking_manager() {
    let (manager, db) = manager("crash");
    let crashed = manager.start_task_run("container_a").unwrap();

    let result = manager.run_probe_crash_in_process_worker(&crashed.run_id);

    assert!(result.is_err());
    let crashed = db.get_run(&crashed.run_id).unwrap();
    assert_eq!(crashed.status, "failed");
    assert!(crashed
        .error_message
        .as_deref()
        .unwrap_or_default()
        .contains("worker process exited"));
    let next = manager.start_task_run("container_b").unwrap();
    manager
        .run_probe_sleep_in_process_worker(&next.run_id, 100)
        .unwrap();
    assert_eq!(db.get_run(&next.run_id).unwrap().status, "completed");
}
