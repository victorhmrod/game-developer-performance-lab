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
        if mode == "float_numbers" {
            if line.contains("\"hello\"") { writeln!(output, "{{\"version\":1.0,\"id\":{id}.0,\"ok\":true,\"capabilities\":[\"set_load\"]}}").unwrap(); }
            else if line.contains("\"set_load\"") { writeln!(output, "{{\"version\":1.0,\"id\":{id}.0,\"ok\":true,\"ready\":true,\"load\":1.0}}").unwrap(); }
            else { writeln!(output, "{{\"version\":1.0,\"id\":{id}.0,\"ok\":true}}").unwrap(); }
            output.flush().unwrap();
            if line.contains("\"shutdown\"") { return; }
            continue;
        }
        if line.contains("\"hello\"") {
            if mode == "bad_metadata" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"],\"metric_metadata\":{{\"fps\":42}}}}").unwrap(); }
            else if mode == "invalid_metric" || mode == "missing_metric" {
                writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\",\"metrics\"],\"metric_metadata\":{{\"frame_ms\":\"milliseconds; test provider\"}}}}").unwrap();
            } else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"]}}").unwrap(); }
        } else if line.contains("\"set_load\"") {
            if mode == "bad_ack" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"ready\":false}}").unwrap(); }
            else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"ready\":true,\"load\":1}}").unwrap(); if mode == "early" { output.flush().unwrap(); std::thread::sleep(Duration::from_millis(400)); return; } }
        } else if line.contains("\"metrics\"") {
            if mode == "invalid_metric" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"metrics\":{{\"frame_ms\":\"NaN\"}}}}").unwrap(); }
            else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"metrics\":{{}}}}").unwrap(); }
        } else if line.contains("\"shutdown\"") { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true}}").unwrap(); output.flush().unwrap(); return; }
        else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true}}").unwrap(); }
        output.flush().unwrap();
    }
}
