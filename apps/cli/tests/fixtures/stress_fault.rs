use std::{io::{self, BufRead, Write}, time::Duration};
fn main() {
    let mode = std::env::args().nth(1).unwrap();
    let stdin = io::stdin();
    let mut output = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let id = line.split("\"id\":").nth(1).unwrap().split([',','}']).next().unwrap();
        if mode == "hang" { std::thread::sleep(Duration::from_secs(10)); return; }
        if mode == "stop_reading" && line.contains("\"hello\"") {
            writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"]}}").unwrap(); output.flush().unwrap();
            std::thread::sleep(Duration::from_secs(10)); return;
        }
        if mode == "bad_json" { writeln!(output, "not json").unwrap(); output.flush().unwrap(); return; }
        if mode == "bad_id" { writeln!(output, "{{\"version\":1,\"id\":999,\"ok\":true}}").unwrap(); output.flush().unwrap(); return; }
        if mode == "fragmented" && line.contains("\"hello\"") {
            write!(output, "{{\"version\":1,").unwrap(); output.flush().unwrap();
            std::thread::sleep(Duration::from_millis(20));
            writeln!(output, "\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"]}}").unwrap(); output.flush().unwrap(); continue;
        }
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
            if mode == "missing_capability" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[]}}").unwrap(); }
            else if mode == "missing_scenario" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\",\"list_scenarios\"]}}").unwrap(); }
            else if mode == "bad_pid" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"],\"provider_info\":{{\"process_id\":\"0\"}}}}").unwrap(); }
            else if mode == "bad_metadata" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"],\"metric_metadata\":{{\"fps\":42}}}}").unwrap(); }
            else if mode == "invalid_metric" || mode == "missing_metric" {
                writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\",\"metrics\"],\"metric_metadata\":{{\"frame_ms\":\"milliseconds; test provider\"}}}}").unwrap();
            } else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"capabilities\":[\"set_load\"]}}").unwrap(); }
        } else if line.contains("\"list_scenarios\"") { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"scenarios\":[]}}").unwrap();
        } else if line.contains("\"set_load\"") {
            if mode == "bad_observed" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"ready\":true,\"load\":1,\"observed_load\":0}}").unwrap(); }
            else if mode == "bad_ack" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"ready\":false}}").unwrap(); }
            else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"ready\":true,\"load\":1}}").unwrap(); if mode == "early" { output.flush().unwrap(); std::thread::sleep(Duration::from_millis(400)); return; } }
        } else if line.contains("\"metrics\"") {
            if mode == "invalid_metric" { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"metrics\":{{\"frame_ms\":\"NaN\"}}}}").unwrap(); }
            else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true,\"metrics\":{{}}}}").unwrap(); }
        } else if line.contains("\"shutdown\"") { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true}}").unwrap(); output.flush().unwrap(); return; }
        else { writeln!(output, "{{\"version\":1,\"id\":{id},\"ok\":true}}").unwrap(); }
        output.flush().unwrap();
    }
}
