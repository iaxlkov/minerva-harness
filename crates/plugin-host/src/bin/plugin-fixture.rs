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
    for line in io::stdin().lock().lines() {
        let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let method = request["method"].as_str().unwrap();
        match method {
            "test.hang" => {
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            "test.invalid" => {
                println!("invalid");
            }
            _ => {
                let result = if method == "initialize" {
                    json!({"apiVersion":1})
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
