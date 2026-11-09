use serde_json::{Value, json};
use std::io::{self, BufRead, Read, Write};

// Synthetic local control example, not a model of a real game workload.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let mut memory = Vec::<u8>::new();
    let mut initialized = false;
    let mut last_id = 0_u64;
    loop {
        let mut line = String::new();
        if (&mut input).take(8193).read_line(&mut line)? == 0 {
            break;
        }
        if line.len() > 8192 || !line.ends_with('\n') {
            return Err("invalid protocol message length".into());
        }
        let request: Value = serde_json::from_str(&line)?;
        let id = request["id"].as_u64().ok_or("missing id")?;
        if request["version"] != 1 || id <= last_id {
            return Err("invalid protocol version or sequence".into());
        }
        last_id = id;
        let mut reply = json!({"version": 1, "id": id, "ok": true});
        match request["command"].as_str() {
            Some("hello") => {
                reply["capabilities"] = json!(["set_load", "shutdown", "reset", "metrics"]);
            }
            Some("initialize") => {
                if request["data"]["maximum_load"]
                    .as_u64()
                    .is_none_or(|v| v > 32768)
                {
                    reply["ok"] = false.into();
                    reply["error"] = "maximum load exceeds 32768 KiB".into();
                } else {
                    initialized = true;
                }
            }
            Some("set_load") if initialized => {
                let load = request["data"]["load"].as_u64().ok_or("invalid load")?;
                if load > 32768 {
                    reply["ok"] = false.into();
                    reply["error"] = "load exceeds 32768 KiB".into();
                } else {
                    memory.resize((load * 1024) as usize, 1);
                    reply["load"] = load.into();
                    reply["ready"] = true.into();
                }
            }
            Some("reset") if initialized => {
                memory.clear();
            }
            Some("metrics") if initialized => {
                reply["metrics"] = json!({"allocated_bytes": memory.len()});
            }
            Some("shutdown") => {
                writeln!(output, "{reply}")?;
                output.flush()?;
                break;
            }
            _ => {
                reply["ok"] = false.into();
                reply["error"] = "unsupported command or not initialized".into();
            }
        }
        writeln!(output, "{reply}")?;
        output.flush()?;
    }
    Ok(())
}
