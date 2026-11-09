use std::{io::{self, BufRead, Write}, time::Duration};
fn main() {
    let mode = std::env::args().nth(1).unwrap();
    let stdin = io::stdin();
    let mut output = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let id = line.split("\"id\":").nth(1).unwrap().split([',','}']).next().unwrap();
        if mode == "hang" { std::thread::sleep(Duration::from_secs(10)); return; }
        if mode == "bad_version" { writeln!(output, "{{\"version\":2,\"id\":{id},\"ok\":true}}").unwrap(); output.flush().unwrap(); return; }
        if line.contains("\"hello\"") { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"]}}").unwrap(); }
        else if line.contains("\"set_load\"") {
            if mode == "bad_ack" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"ready\":false}}").unwrap(); }
            else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"ready\":true,\"load\":1}}").unwrap(); output.flush().unwrap(); std::thread::sleep(Duration::from_millis(400)); return; }
        }
        else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true}}").unwrap(); }
        output.flush().unwrap();
    }
}
