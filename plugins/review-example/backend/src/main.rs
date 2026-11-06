use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
fn main() {
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            break;
        };
        let method = request["method"].as_str().unwrap_or("");
        let result = match method {
            "initialize" => json!({"apiVersion":1,"methods":["review.summary"]}),
            "shutdown" => Value::Null,
            "review.summary" => {
                let files = request["params"]["files"].as_array();
                let count = files.map_or(0, Vec::len);
                let lines = files.map_or(0, |files| {
                    files
                        .iter()
                        .map(|f| f["content"].as_str().map_or(0, |s| s.lines().count()))
                        .sum::<usize>()
                });
                json!({"fileCount":count,"lineCount":lines,"readOnly":true})
            }
            _ => {
                let response = json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"Unknown method"}});
                println!("{response}");
                continue;
            }
        };
        println!(
            "{}",
            json!({"jsonrpc":"2.0","id":request["id"],"result":result})
        );
        let _ = io::stdout().flush();
        if method == "shutdown" {
            break;
        }
    }
}
