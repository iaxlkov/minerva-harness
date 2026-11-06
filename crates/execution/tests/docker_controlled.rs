use minerva_execution::{docker::*, *};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
#[derive(Default)]
struct Engine {
    containers: HashMap<String, Vec<String>>,
    removed: Vec<String>,
    killed: Vec<String>,
    creates: Vec<Vec<String>>,
    fail_remove: bool,
    cancel_at: Option<(String, Arc<std::sync::atomic::AtomicBool>)>,
    finished: bool,
    starts: usize,
}
struct Controlled(Arc<Mutex<Engine>>);
impl ControlCommand for Controlled {
    fn run(&mut self, program: &str, args: &[String]) -> Result<String> {
        assert_eq!(program, "docker");
        assert_eq!(args[0], "--host=unix:///var/run/docker.sock");
        let args = &args[1..];
        let mut state = self.0.lock().unwrap();
        if let Some((op, token)) = &state.cancel_at {
            if op == &args[0] {
                token.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        match args[0].as_str() {
            "ps" => {
                assert!(args.contains(&"--no-trunc".into()));
                Ok(if state.containers.is_empty() {
                    String::new()
                } else {
                    "a".repeat(64)
                })
            }
            "create" => {
                let name = args
                    .iter()
                    .find_map(|a| a.strip_prefix("--name="))
                    .unwrap()
                    .to_string();
                state.containers.insert(name, args.to_vec());
                state.creates.push(args.to_vec());
                Ok("id".into())
            }
            "start" => {
                state.starts += 1;
                assert!(state.containers.contains_key(&args[1]));
                Ok(args[1].clone())
            }
            "inspect" => {
                assert!(state.containers.contains_key(&args[2]));
                Ok(if state.finished {
                    r#"{"Running":false,"ExitCode":0}"#
                } else {
                    r#"{"Running":true,"ExitCode":0}"#
                }
                .into())
            }
            "kill" => {
                assert!(state.containers.contains_key(&args[1]));
                state.killed.push(args[1].clone());
                Ok("killed".into())
            }
            "logs" => Ok("partial output".into()),
            "rm" => {
                if state.fail_remove {
                    return Err(Error("remove failed".into()));
                }
                if args[2] == "a".repeat(64) {
                    if let Some(key) = state.containers.keys().next().cloned() {
                        state.containers.remove(&key);
                    }
                } else {
                    state.containers.remove(&args[2]);
                }
                state.removed.push(args[2].clone());
                Ok("removed".into())
            }
            _ => panic!("unexpected operation {args:?}"),
        }
    }
}
fn config(root: PathBuf) -> DockerConfig {
    DockerConfig {
        root,
        host_image: format!("host@sha256:{}", "a".repeat(64)),
        execution_image: format!("execution@sha256:{}", "b".repeat(64)),
        control_network: "minerva-control-test".into(),
    }
}
fn launch(root: PathBuf) -> Launch {
    Launch {
        assignment: Assignment {
            tenant_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            assignment_id: Uuid::new_v4(),
            worker_id: Uuid::new_v4(),
            generation: 1,
        },
        limits: Limits {
            memory_mib: 128,
            cpu_milli: 250,
            pids: 16,
            disk_mib: 128,
        },
        mounts: Mounts {
            workspace: root.clone(),
            data: root.clone(),
            config: root,
        },
    }
}
#[test]
fn concrete_driver_timeout_removes_container_and_never_exposes_private_mounts() {
    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(Mutex::new(Engine::default()));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    let command = Execution {
        argv: vec![
            "/bin/sh".into(),
            "-c".into(),
            "sleep 100 & wait; touch /outside".into(),
        ],
        env: Default::default(),
        timeout_ms: 1,
    };
    let result = driver
        .execute(&launch(root.path().into()), &command)
        .unwrap();
    assert!(result.timed_out);
    let state = engine.lock().unwrap();
    assert!(state.containers.is_empty());
    assert_eq!(state.killed.len(), 1);
    assert_eq!(state.removed, state.killed);
    let create = &state.creates[0];
    assert!(create.contains(&"--runtime=runsc".into()));
    assert!(create.contains(&"--network=none".into()));
    assert_eq!(
        create
            .iter()
            .filter(|s| s.starts_with("type=bind,"))
            .count(),
        1
    );
    assert!(!create.iter().any(|s| s.contains("dst=/data")
        || s.contains("dst=/config")
        || s.contains("docker.sock")
        || s == "--privileged"));
    assert_eq!(&create[create.len() - 3..], command.argv.as_slice());
    assert!(create.contains(&"--memory=128m".into()));
    assert!(create.contains(&"--pids-limit=16".into()));
}
#[test]
fn cleanup_failure_is_not_reported_as_execution_success() {
    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(Mutex::new(Engine {
        fail_remove: true,
        ..Default::default()
    }));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    assert!(driver
        .execute(
            &launch(root.path().into()),
            &Execution {
                argv: vec!["/bin/sleep".into(), "100".into()],
                env: Default::default(),
                timeout_ms: 1
            }
        )
        .is_err());
    assert_eq!(engine.lock().unwrap().containers.len(), 1);
}
#[test]
fn concrete_host_gets_private_mounts_only_on_internal_control_network() {
    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(Mutex::new(Engine::default()));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    driver.start_host(&launch(root.path().into())).unwrap();
    let state = engine.lock().unwrap();
    assert_eq!(state.containers.len(), 1);
    let create = &state.creates[0];
    assert!(create.contains(&"--runtime=runsc".into()));
    assert!(create.contains(&"--network=minerva-control-test".into()));
    assert_eq!(
        create
            .iter()
            .filter(|s| s.starts_with("type=bind,"))
            .count(),
        3
    );
    assert!(!create
        .iter()
        .any(|s| s.contains("docker.sock") || s == "--privileged" || s == "--pid=host"));
}
#[test]
fn mutable_images_and_untrusted_network_names_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut c = config(root.path().into());
    c.execution_image = "alpine:latest".into();
    assert!(DockerDriver::new(c).is_err());
    let mut c = config(root.path().into());
    c.control_network = "host".into();
    assert!(DockerDriver::new(c).is_err());
}
#[test]
fn concrete_driver_explicit_cancel_removes_orphans() {
    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(Mutex::new(Engine::default()));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    let token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    engine.lock().unwrap().cancel_at = Some(("inspect".into(), token.clone()));
    driver.set_cancellation(token);
    let result = driver
        .execute(
            &launch(root.path().into()),
            &Execution {
                argv: vec!["/bin/sleep".into(), "100".into()],
                env: Default::default(),
                timeout_ms: 300000,
            },
        )
        .unwrap();
    assert_eq!(result.exit_code, -1);
    assert!(!result.timed_out);
    let state = engine.lock().unwrap();
    assert!(state.containers.is_empty());
    assert_eq!(state.removed.len(), 1);
}

#[test]
fn concrete_stop_confirms_no_container_remains() {
    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(Mutex::new(Engine::default()));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    let launch = launch(root.path().into());
    driver.start_host(&launch).unwrap();
    driver
        .stop_workspace(launch.assignment.tenant_id, launch.assignment.workspace_id)
        .unwrap();
    let state = engine.lock().unwrap();
    assert!(state.containers.is_empty());
    assert_eq!(state.removed.len(), 1);
}
#[test]
fn precancelled_execution_never_creates_or_starts_a_container() {
    let root = tempfile::tempdir().unwrap();
    let engine = Arc::new(Mutex::new(Engine {
        finished: true,
        ..Default::default()
    }));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    driver.set_cancellation(Arc::new(std::sync::atomic::AtomicBool::new(true)));
    let output = driver
        .execute(
            &launch(root.path().into()),
            &Execution {
                argv: vec!["/bin/true".into()],
                env: Default::default(),
                timeout_ms: 1000,
            },
        )
        .unwrap();
    assert_eq!(output.exit_code, -1);
    assert!(!output.timed_out);
    let state = engine.lock().unwrap();
    assert!(state.creates.is_empty());
    assert_eq!(state.starts, 0);
    assert!(state.containers.is_empty());
    assert!(state.killed.is_empty());
    assert!(state.removed.is_empty());
}

#[test]
fn cancellation_during_create_prevents_start_and_removes_created_container() {
    let root = tempfile::tempdir().unwrap();
    let token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let engine = Arc::new(Mutex::new(Engine {
        cancel_at: Some(("create".into(), token.clone())),
        ..Default::default()
    }));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    driver.set_cancellation(token);
    let result = driver
        .execute(
            &launch(root.path().into()),
            &Execution {
                argv: vec!["/bin/true".into()],
                env: Default::default(),
                timeout_ms: 1000,
            },
        )
        .unwrap();
    assert_eq!(result.exit_code, -1);
    let state = engine.lock().unwrap();
    assert_eq!(state.creates.len(), 1);
    assert_eq!(state.starts, 0);
    assert!(state.containers.is_empty());
    assert_eq!(state.removed.len(), 1);
}
#[test]
fn cancellation_during_completed_state_inspection_cannot_report_success() {
    let root = tempfile::tempdir().unwrap();
    let token = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let engine = Arc::new(Mutex::new(Engine {
        cancel_at: Some(("inspect".into(), token.clone())),
        finished: true,
        ..Default::default()
    }));
    let mut driver = DockerDriver::with_control(
        config(root.path().into()),
        Box::new(Controlled(engine.clone())),
    )
    .unwrap();
    driver.set_cancellation(token);
    let result = driver
        .execute(
            &launch(root.path().into()),
            &Execution {
                argv: vec!["/bin/true".into()],
                env: Default::default(),
                timeout_ms: 1000,
            },
        )
        .unwrap();
    assert_eq!(result.exit_code, -1);
    assert!(!result.timed_out);
    let state = engine.lock().unwrap();
    assert_eq!(state.starts, 1);
    assert!(state.containers.is_empty());
    assert_eq!(state.removed.len(), 1);
}
