use std::{env, fs, thread, time::Duration};
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|v| v == "--help") {
        eprintln!("PresentMon mock-1\n--process_id --output_file --timed --terminate_after_timed --session_name --no_console_stats");
        std::process::exit(1);
    }
    if args.iter().any(|v| v == "short") { thread::sleep(Duration::from_millis(300)); return; }
    if args.iter().any(|v| v == "target") { thread::sleep(Duration::from_secs(10)); return; }
    let value = |name: &str| &args[args.iter().position(|v| v == name).unwrap() + 1];
    let pid = value("--process_id");
    let output = value("--output_file");
    thread::sleep(Duration::from_secs(value("--timed").parse().unwrap()));
    let mut csv = format!("ProcessID,SwapChainAddress,MsBetweenDisplayChange\n{pid},test,10\n{pid},test,20\n");
    if env::var_os("GDPL_MOCK_MULTIPLE").is_some() { csv.push_str(&format!("{pid},other,30\n")); }
    fs::write(output, csv).unwrap();
}
