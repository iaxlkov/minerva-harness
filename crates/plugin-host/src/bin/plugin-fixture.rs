use serde_json::{json, Value};
use std::io::{self, BufRead};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--install") {
        minerva_plugin_host::install_local(
            std::path::Path::new(&args[2]),
            std::path::Path::new(&args[3]),
        )
        .unwrap();
        return;
    }
    if args.get(1).map(String::as_str) == Some("--descendant") {
        std::thread::sleep(std::time::Duration::from_millis(800));
        std::fs::write(&args[2], "survived").unwrap();
        return;
    }

    // A helper launched before reading initialize exercises the startup cleanup boundary.
    if let Ok(marker) = std::fs::read_to_string("startup-helper-marker") {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--descendant")
            .arg(marker)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::spawn(move || child.wait());
    }
    let mut tools = FixtureTools::default();
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    while let Some(line) = lines.next() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let method = request["method"].as_str().unwrap();
        if method == "initialize" {
            tools.data_dir = request["params"]["dataDir"]
                .as_str()
                .map(std::path::PathBuf::from);
        }
        if method.starts_with("tools.") || method == "test.tool-state" {
            let response = match tools.handle(method, &request["params"]) {
                Ok(result) => json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                Err(message) => {
                    json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32602,"message":message}})
                }
            };
            println!("{response}");
            continue;
        }
        if method == "images.available" {
            let result: Value =
                serde_json::from_slice(&std::fs::read("image-generator.json").unwrap()).unwrap();
            println!(
                "{}",
                json!({"jsonrpc":"2.0","id":request["id"],"result":result})
            );
            continue;
        }
        if method == "initialize" && std::path::Path::new("initialize-service").exists() {
            println!(
                "{}",
                json!({"jsonrpc":"2.0","id":"init-service","method":"services.call","params":{"service":"echo","version":1,"method":"test.pid","params":{}}})
            );
            let reply: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
            if reply.get("error").is_some() {
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":request["id"],"error":reply["error"]})
                );
                continue;
            }
        }
        if method == "shutdown" {
            if let Ok(marker) = std::fs::read_to_string("hang-shutdown") {
                std::fs::write(marker, "started").unwrap();
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
        }
        match method {
            "engine.start" => {
                let bytes = request["params"]["payload"].as_str().map_or(0, str::len);
                let response_bytes = request["params"]["responseBytes"]
                    .as_u64()
                    .unwrap_or(0)
                    .min(2 * 1024 * 1024) as usize;
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":request["id"],"result":{"bytes":bytes,"data":"x".repeat(response_bytes)}})
                );
            }
            "services.invoke" => {
                let result = if request["params"]["service"] == "echo"
                    && request["params"]["version"] == 1
                    && request["params"]["method"] == "test.pid"
                {
                    json!({"pid":std::process::id()})
                } else {
                    Value::Null
                };
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                );
            }
            "test.service" => {
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":"service-1","method":"services.call","params":request["params"]})
                );
                let mut reply: Value =
                    serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                reply["id"] = request["id"].clone();
                println!("{reply}");
            }

            "test.error" => {
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32602,"message":"Invalid fixture params"}})
                );
            }
            "test.hang" => {
                if let Some(marker) = request["params"]["marker"].as_str() {
                    std::fs::write(marker, "started").unwrap();
                }
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            "test.invalid" => {
                println!("invalid");
            }
            _ => {
                if method == "test.delay" {
                    std::fs::write(request["params"]["marker"].as_str().unwrap(), "started")
                        .unwrap();
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                let result = if method == "initialize" {
                    json!({"apiVersion":if std::path::Path::new("bad-handshake").exists() { 2 } else { 1 }})
                } else if method == "engine.models" {
                    serde_json::from_slice(&std::fs::read("catalog.json").unwrap()).unwrap()
                } else if method == "test.pid" || method == "test.delay" {
                    json!({"pid": std::process::id()})
                } else if method == "test.spawn" {
                    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                        .arg("--descendant")
                        .arg(request["params"]["marker"].as_str().unwrap())
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .spawn()
                        .unwrap();
                    let child_id = child.id();
                    std::thread::spawn(move || child.wait());
                    json!({"childId":child_id})
                } else {
                    json!({"lineCount":2})
                };
                println!(
                    "{}",
                    json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                );
                if method == "shutdown" || method == "test.exit" {
                    break;
                }
            }
        }
    }
}

#[derive(Default)]
struct FixtureTools {
    data_dir: Option<std::path::PathBuf>,
    jobs: std::collections::BTreeMap<(String, String), FixtureToolJob>,
}
struct FixtureToolJob {
    snapshot: std::sync::Arc<std::sync::Mutex<Value>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl FixtureToolJob {
    fn cancel(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        {
            let mut snapshot = self.snapshot.lock().unwrap();
            if snapshot["status"] == "running" {
                *snapshot = json!({"status":"cancelled","error":"Fixture tool cancelled"});
            }
        }
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}
impl Drop for FixtureToolJob {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl FixtureTools {
    fn audit(&self, method: &str, params: &Value) -> Result<(), String> {
        use std::io::Write;
        let path = self
            .data_dir
            .as_ref()
            .ok_or("fixture was not initialized")?
            .join("tool-audit.jsonl");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        writeln!(
            file,
            "{}",
            json!({"method":method,"runId":params["runId"],"callId":params["callId"]})
        )
        .map_err(|e| e.to_string())
    }
    fn handle(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        if method == "tools.list" {
            if std::path::Path::new("image-generator.json").exists() {
                return Ok(
                    json!([{"name":"generate_image","description":"fixture image generator","inputSchema":{"type":"object"},"outputSchema":{"type":"object"},"sideEffect":true},{"name":"image_status","description":"fixture image status","inputSchema":{"type":"object"},"outputSchema":{"type":"object"},"sideEffect":false}]),
                );
            }
            let output = json!({"type":"object","required":["value","runId","callId","pid"],"properties":{"value":{},"runId":{"type":"string"},"callId":{"type":"string"},"pid":{"type":"integer"}},"additionalProperties":false});
            return Ok(json!([
                {"name":"echo","description":"Echo a value from the fixture process","inputSchema":{"type":"object","properties":{"value":{}},"required":["value"],"additionalProperties":false},"outputSchema":output,"sideEffect":false},
                {"name":"delayed_write","description":"Write a disposable marker after a cancellable delay","inputSchema":{"type":"object","properties":{"value":{},"delayMillis":{"type":"integer","minimum":0,"maximum":30000},"effectPath":{"type":"string"}},"required":["value","delayMillis","effectPath"],"additionalProperties":false},"outputSchema":output,"sideEffect":true}
            ]));
        }
        if method == "test.tool-state" {
            return Ok(json!({"active":self.jobs.len()}));
        }
        let run = params["runId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("missing runId")?
            .to_owned();
        let call = params["callId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("missing callId")?
            .to_owned();
        let key = (run.clone(), call.clone());
        match method {
            "tools.start" => {
                if self.jobs.contains_key(&key) {
                    return Err("tool identity already started".into());
                }
                let name = params["name"].as_str().ok_or("missing name")?;
                if std::path::Path::new("image-generator.json").exists() {
                    self.audit(method, params)?;
                    let config: Value =
                        serde_json::from_slice(&std::fs::read("image-generator.json").unwrap())
                            .unwrap();
                    let result = if config["fail"] == true {
                        json!({"status":"failed","error":"Selected generator failed"})
                    } else {
                        json!({"status":"completed","output":{"jobId":"fixture-image","status":if name=="generate_image" {"running"}else{"completed"}}})
                    };
                    self.jobs.insert(
                        key,
                        FixtureToolJob {
                            snapshot: std::sync::Arc::new(std::sync::Mutex::new(result)),
                            cancelled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                                false,
                            )),
                            worker: None,
                        },
                    );
                    return Ok(Value::Null);
                }
                if !matches!(name, "echo" | "delayed_write") {
                    return Err("unknown fixture tool".into());
                }
                let delay = if name == "delayed_write" {
                    params["arguments"]["delayMillis"]
                        .as_u64()
                        .filter(|n| *n <= 30000)
                        .ok_or("invalid delayMillis")?
                } else {
                    0
                };
                let effect = if name == "delayed_write" {
                    Some(std::path::PathBuf::from(
                        params["arguments"]["effectPath"]
                            .as_str()
                            .ok_or("missing effectPath")?,
                    ))
                } else {
                    None
                };
                self.audit(method, params)?;
                let snapshot =
                    std::sync::Arc::new(std::sync::Mutex::new(json!({"status":"running"})));
                let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let output = json!({"value":params["arguments"]["value"],"runId":run,"callId":call,"pid":std::process::id()});
                let worker = std::thread::spawn({
                    let snapshot = snapshot.clone();
                    let cancelled = cancelled.clone();
                    move || {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_millis(delay);
                        while std::time::Instant::now() < deadline {
                            if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                                return;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        // Cancellation and the final effect share this lock: after cancel
                        // acknowledges, the worker cannot create a late marker.
                        let mut state = snapshot.lock().unwrap();
                        if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                            return;
                        }
                        if let Some(path) = effect {
                            if let Err(error) = std::fs::write(path, "completed") {
                                *state = json!({"status":"failed","error":error.to_string()});
                                return;
                            }
                        }
                        *state = json!({"status":"completed","output":output});
                    }
                });
                self.jobs.insert(
                    key,
                    FixtureToolJob {
                        snapshot,
                        cancelled,
                        worker: Some(worker),
                    },
                );
                Ok(json!({"accepted":true}))
            }
            "tools.poll" => self
                .jobs
                .get(&key)
                .map(|job| job.snapshot.lock().unwrap().clone())
                .ok_or("unknown tool identity".into()),
            "tools.cancel" => {
                self.audit(method, params)?;
                let found = self.jobs.get_mut(&key);
                if let Some(job) = found {
                    job.cancel();
                }
                Ok(json!({"cancelled":true}))
            }
            "tools.release" => {
                self.audit(method, params)?;
                self.jobs.remove(&key);
                Ok(json!({"released":true}))
            }
            _ => Err("unknown tool method".into()),
        }
    }
}
