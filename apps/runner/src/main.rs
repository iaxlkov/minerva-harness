use minerva_execution::{
    docker::{DockerConfig, DockerDriver},
    Assignment, Request, Runner,
};
use std::{
    io::{self, BufRead, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
};
struct Pending(Arc<AtomicBool>);
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
struct Queued {
    request: Request,
    cancel: Arc<AtomicBool>,
    _pending: Pending,
}
/// Only one ordinary request and one stop may be outstanding. The channel cannot
/// fill behind an execute, so the reader always examines a subsequent stop.
struct InputGate {
    sender: mpsc::Sender<Queued>,
    ordinary: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    active: Vec<(Assignment, Weak<AtomicBool>)>,
}
impl InputGate {
    fn new(sender: mpsc::Sender<Queued>) -> Self {
        Self {
            sender,
            ordinary: Default::default(),
            stop: Default::default(),
            active: Vec::new(),
        }
    }
    fn enqueue(&mut self, request: Request) -> Result<(), &'static str> {
        self.active.retain(|(_, token)| token.strong_count() > 0);
        let gate = if let Request::Stop { assignment } = &request {
            for (scope, token) in &self.active {
                if scope == assignment {
                    if let Some(token) = token.upgrade() {
                        token.store(true, Ordering::Release);
                    }
                }
            }
            self.stop.clone()
        } else {
            self.ordinary.clone()
        };
        if gate
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("runner busy: one ordinary request and one stop may be pending");
        }
        let pending = Pending(gate);
        let cancel = Arc::new(AtomicBool::new(false));
        if let Request::Execute { assignment, .. } = &request {
            self.active
                .push((assignment.clone(), Arc::downgrade(&cancel)));
        }
        self.sender
            .send(Queued {
                request,
                cancel,
                _pending: pending,
            })
            .map_err(|_| "control service closed")
    }
    fn cancel_active(&self) {
        for (_, token) in &self.active {
            if let Some(token) = token.upgrade() {
                token.store(true, Ordering::Release);
            }
        }
    }
}
fn write_response(output: &Mutex<io::Stdout>, response: &serde_json::Value) -> io::Result<()> {
    let mut output = output.lock().unwrap();
    serde_json::to_writer(&mut *output, response)?;
    output.write_all(b"\n")?;
    output.flush()
}
fn main() {
    if let Err(error) = serve() {
        eprintln!("runner unavailable: {error}");
        std::process::exit(1)
    }
}
fn serve() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::env::args_os()
        .nth(1)
        .ok_or("usage: minerva-runner <operator-config.json>")?;
    let config: DockerConfig = serde_json::from_slice(&std::fs::read(config_path)?)?;
    let journal = config.root.join(".runner-control");
    let mut runner = Runner::open(DockerDriver::new(config)?, journal)?;
    let (sender, receiver) = mpsc::channel();
    let output = Arc::new(Mutex::new(io::stdout()));
    let reader_output = output.clone();
    std::thread::spawn(move || {
        let mut gate = InputGate::new(sender);
        let mut input = io::stdin().lock();
        loop {
            let mut line = Vec::new();
            let bytes = match input
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_until(b'\n', &mut line)
            {
                Ok(n) => n,
                Err(_) => break,
            };
            if bytes == 0 || bytes > 1024 * 1024 || !line.ends_with(b"\n") {
                break;
            }
            let result = match serde_json::from_slice::<Request>(&line) {
                Ok(request) => gate.enqueue(request),
                Err(_) => Err("invalid request"),
            };
            if let Err(error) = result {
                if write_response(
                    &reader_output,
                    &serde_json::json!({"ok":false,"error":error}),
                )
                .is_err()
                {
                    break;
                }
            }
        }
        gate.cancel_active();
    });
    for queued in receiver {
        runner.set_cancellation(queued.cancel);
        let response = match runner.handle(queued.request) {
            Ok(response) => serde_json::json!({"ok":true,"result":response}),
            Err(error) => serde_json::json!({"ok":false,"error":error.to_string()}),
        };
        write_response(&output, &response)?;
        // _pending releases the ordinary/stop slot only after its response is emitted.
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn assignment() -> Assignment {
        serde_json::from_value(serde_json::json!({"tenantId":"11111111-1111-4111-8111-111111111111","workspaceId":"22222222-2222-4222-8222-222222222222","assignmentId":"33333333-3333-4333-8333-333333333333","workerId":"44444444-4444-4444-8444-444444444444","generation":1})).unwrap()
    }
    fn execute(a: Assignment) -> Request {
        Request::Execute {
            assignment: a,
            command: minerva_execution::Execution {
                argv: vec!["/bin/true".into()],
                env: Default::default(),
                timeout_ms: 1000,
            },
        }
    }
    #[test]
    fn extra_executions_are_rejected_without_blocking_stop_or_growing_queue() {
        let (sender, receiver) = mpsc::channel();
        let mut gate = InputGate::new(sender);
        let a = assignment();
        gate.enqueue(execute(a.clone())).unwrap();
        for _ in 0..100 {
            assert!(gate.enqueue(execute(a.clone())).is_err());
        }
        gate.enqueue(Request::Stop { assignment: a }).unwrap();
        let queued: Vec<_> = receiver.try_iter().collect();
        assert_eq!(queued.len(), 2);
        assert!(queued[0].cancel.load(Ordering::Acquire));
        assert!(matches!(queued[1].request, Request::Stop { .. }));
    }
    #[test]
    fn pending_slot_is_held_through_processing_and_released_on_completion() {
        let (sender, receiver) = mpsc::channel();
        let mut gate = InputGate::new(sender);
        let a = assignment();
        gate.enqueue(execute(a.clone())).unwrap();
        let running = receiver.recv().unwrap();
        assert!(gate.enqueue(execute(a.clone())).is_err());
        drop(running);
        gate.enqueue(execute(a)).unwrap();
        assert!(receiver.try_recv().is_ok());
    }
    #[test]
    fn stale_stop_cannot_cancel_current_assignment() {
        let (sender, receiver) = mpsc::channel();
        let mut gate = InputGate::new(sender);
        let a = assignment();
        gate.enqueue(execute(a.clone())).unwrap();
        let mut stale = a;
        stale.generation += 1;
        gate.enqueue(Request::Stop { assignment: stale }).unwrap();
        let running = receiver.recv().unwrap();
        assert!(!running.cancel.load(Ordering::Acquire));
    }
}
