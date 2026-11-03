use std::{env, fs, thread, time::Duration};
fn main() {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|v| v == "--help") {
        println!("--process_id --output_file --timed --terminate_after_timed --session_name --no_console_stats");
        return;
    }
    if args.iter().any(|v| v == "target") { thread::sleep(Duration::from_secs(10)); return; }
    let value = |name: &str| &args[args.iter().position(|v| v == name).unwrap() + 1];
    let pid = value("--process_id");
    let output = value("--output_file");
    thread::sleep(Duration::from_secs(value("--timed").parse().unwrap()));
    fs::write(output, format!("ProcessID,SwapChainAddress,MsBetweenDisplayChange\n{pid},test,10\n{pid},test,20\n")).unwrap();
}
