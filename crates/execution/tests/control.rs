use minerva_execution::*;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
#[derive(Default)]
struct State {
    events: Vec<String>,
    running: bool,
    fail_stop: bool,
    fail_start: bool,
    fail_exec: bool,
}
struct Fake(Arc<Mutex<State>>);
impl OciDriver for Fake {
    fn ready(&mut self) -> Result<()> {
        Ok(())
    }
    fn prepare(&mut self, _: &Assignment, _: &Limits) -> Result<Mounts> {
        self.0.lock().unwrap().events.push("mount".into());
        Ok(Mounts {
            workspace: PathBuf::from("workspace"),
            data: PathBuf::from("private-data"),
            config: PathBuf::from("private-config"),
        })
    }
    fn stop_workspace(&mut self, _: Uuid, _: Uuid) -> Result<()> {
        let mut s = self.0.lock().unwrap();
        s.events.push("stop-confirm".into());
        if s.fail_stop {
            return Err(Error("still alive".into()));
        }
        s.running = false;
        Ok(())
    }
    fn start_host(&mut self, l: &Launch) -> Result<()> {
        let mut s = self.0.lock().unwrap();
        s.events.push(format!("start:{}", l.assignment.generation));
        s.running = true;
        if s.fail_start {
            Err(Error("start failed".into()))
        } else {
            Ok(())
        }
    }
    fn running(&mut self, _: &Assignment) -> Result<bool> {
        Ok(self.0.lock().unwrap().running)
    }
    fn execute(&mut self, _: &Launch, c: &Execution) -> Result<Output> {
        self.0
            .lock()
            .unwrap()
            .events
            .push(format!("exec:{}", c.argv[0]));
        if self.0.lock().unwrap().fail_exec {
            return Err(Error("cleanup failed".into()));
        }
        Ok(Output {
            exit_code: 0,
            stdout: "sandbox".into(),
            stderr: String::new(),
            timed_out: false,
        })
    }
}
fn a() -> Assignment {
    Assignment {
        tenant_id: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        assignment_id: Uuid::new_v4(),
        worker_id: Uuid::new_v4(),
        generation: 1,
    }
}
fn limits() -> Limits {
    Limits {
        memory_mib: 128,
        cpu_milli: 100,
        pids: 16,
        disk_mib: 128,
    }
}
#[test]
fn stop_confirmation_precedes_mount_and_generation_is_fenced() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let mut runner = Runner::open(Fake(state.clone()), dir.path().into()).unwrap();
    let old = a();
    runner
        .handle(Request::Start {
            assignment: old.clone(),
            limits: limits(),
        })
        .unwrap();
    assert_eq!(
        state.lock().unwrap().events,
        ["stop-confirm", "mount", "start:1"]
    );
    state.lock().unwrap().fail_stop = true;
    let mut new = old.clone();
    new.generation = 2;
    assert!(runner
        .handle(Request::Start {
            assignment: new.clone(),
            limits: limits()
        })
        .is_err());
    assert_eq!(state.lock().unwrap().events.last().unwrap(), "stop-confirm");
    assert_eq!(
        state
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|e| *e == "mount")
            .count(),
        1
    );
    state.lock().unwrap().fail_stop = false;
    runner
        .handle(Request::Start {
            assignment: new,
            limits: limits(),
        })
        .unwrap();
    assert!(runner.handle(Request::Status { assignment: old }).is_err());
}
#[test]
fn restart_stops_old_containers_and_retains_fence() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let old = a();
    {
        let mut runner = Runner::open(Fake(state.clone()), dir.path().into()).unwrap();
        runner
            .handle(Request::Start {
                assignment: old.clone(),
                limits: limits(),
            })
            .unwrap();
        assert!(Runner::open(Fake(state.clone()), dir.path().into()).is_err());
    }
    let mut recovered = Runner::open(Fake(state.clone()), dir.path().into()).unwrap();
    assert!(!state.lock().unwrap().running);
    assert!(recovered
        .handle(Request::Start {
            assignment: old.clone(),
            limits: limits()
        })
        .is_err());
    let mut newer = old;
    newer.generation += 1;
    recovered
        .handle(Request::Start {
            assignment: newer,
            limits: limits(),
        })
        .unwrap();
}
#[test]
fn failed_start_is_cleaned_and_burns_generation() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(State {
        fail_start: true,
        ..Default::default()
    }));
    let mut runner = Runner::open(Fake(state.clone()), dir.path().into()).unwrap();
    let assignment = a();
    assert!(runner
        .handle(Request::Start {
            assignment: assignment.clone(),
            limits: limits()
        })
        .is_err());
    assert!(!state.lock().unwrap().running);
    assert_eq!(state.lock().unwrap().events.last().unwrap(), "stop-confirm");
    state.lock().unwrap().fail_start = false;
    assert!(runner
        .handle(Request::Start {
            assignment,
            limits: limits()
        })
        .is_err());
}
#[test]
fn rejected_environment_and_stale_scope_have_no_execution_side_effect() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let mut runner = Runner::open(Fake(state.clone()), dir.path().into()).unwrap();
    let assignment = a();
    runner
        .handle(Request::Start {
            assignment: assignment.clone(),
            limits: limits(),
        })
        .unwrap();
    let mut command = Execution {
        argv: vec!["/bin/true".into()],
        env: Default::default(),
        timeout_ms: 100,
    };
    command
        .env
        .insert("AWS_SECRET_ACCESS_KEY".into(), "secret".into());
    assert!(runner.execute(assignment.clone(), command.clone()).is_err());
    command.env.clear();
    let mut stale = assignment.clone();
    stale.assignment_id = Uuid::new_v4();
    assert!(runner.execute(stale, command.clone()).is_err());
    assert!(!state
        .lock()
        .unwrap()
        .events
        .iter()
        .any(|e| e.starts_with("exec:")));
    assert_eq!(
        runner.execute(assignment, command).unwrap().stdout,
        "sandbox"
    );
}
#[test]
fn protocol_uses_documented_limit_names_and_rejects_unknown_launch_options() {
    let assignment = a();
    let request = serde_json::json!({"op":"start","assignment":assignment,"limits":{"memoryMiB":128,"cpuMilli":100,"pids":16,"diskMiB":128}});
    assert!(serde_json::from_value::<Request>(request.clone()).is_ok());
    let mut bad = request;
    bad["network"] = serde_json::json!("host");
    assert!(serde_json::from_value::<Request>(bad).is_err());
}

#[test]
fn execution_failure_fences_assignment_and_stops_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(Mutex::new(State {
        fail_exec: true,
        ..Default::default()
    }));
    let mut runner = Runner::open(Fake(state.clone()), dir.path().into()).unwrap();
    let assignment = a();
    runner
        .handle(Request::Start {
            assignment: assignment.clone(),
            limits: limits(),
        })
        .unwrap();
    let command = Execution {
        argv: vec!["/bin/true".into()],
        env: Default::default(),
        timeout_ms: 100,
    };
    assert!(runner.execute(assignment.clone(), command.clone()).is_err());
    assert!(!state.lock().unwrap().running);
    let events = state.lock().unwrap().events.clone();
    assert_eq!(events.last().unwrap(), "stop-confirm");
    assert!(runner.execute(assignment, command).is_err());
    assert_eq!(events, state.lock().unwrap().events);
}
