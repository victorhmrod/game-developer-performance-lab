use gdpl_adapter_sdk::{PROTOCOL_VERSION, Request};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let executable = args
        .next()
        .ok_or("usage: inspect_adapter EXECUTABLE [ARGS...]")?;
    let mut child = Command::new(executable)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("missing stdin")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("missing stdout")?);
    for (id, command, data) in [
        (1, "hello", json!({"versions":[1]})),
        (2, "list_scenarios", json!({})),
        (3, "shutdown", json!({})),
    ] {
        let request = Request {
            version: PROTOCOL_VERSION,
            id,
            command: command.into(),
            data,
        };
        writeln!(input, "{}", serde_json::to_string(&request)?)?;
        input.flush()?;
        let mut line = String::new();
        output.read_line(&mut line)?;
        let reply: Value = serde_json::from_str(&line)?;
        if reply["id"].as_f64() != Some(id as f64)
            || reply["version"].as_f64() != Some(1.0)
            || reply["ok"] != true
        {
            return Err(format!("invalid {command} reply: {reply}").into());
        }
        println!("{command}: {reply}");
    }
    if !child.wait()?.success() {
        return Err("adapter exited unsuccessfully".into());
    }
    Ok(())
}
