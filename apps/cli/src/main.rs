mod live;

use clap::{Parser, Subcommand};
use gdpl_core::{BenchmarkResult, SCHEMA_VERSION, Sample};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    process::{Child, Command, ExitCode, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Parser)]
#[command(version, about = "Game Developer Performance Lab")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}
#[derive(Subcommand)]
enum Commands {
    /// Print host hardware information as JSON.
    Hardware {
        #[command(subcommand)]
        command: HardwareCommand,
    },
    /// Run an external process benchmark.
    Benchmark {
        #[command(subcommand)]
        command: BenchmarkCommand,
    },
    /// Execute a bounded stress scenario.
    Stress {
        #[command(subcommand)]
        command: StressCommand,
    },
    /// Analyze a previously captured PresentMon CSV.
    Frames {
        #[command(subcommand)]
        command: FrameCommand,
    },
    /// Print environment and feature availability.
    Doctor,
}
#[derive(Subcommand)]
enum StressCommand {
    Run {
        #[arg(long)]
        scenario: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
}
#[derive(Subcommand)]
enum FrameCommand {
    Analyze {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        pid: u32,
        #[arg(long)]
        swap_chain: String,
        #[arg(long, default_value_t = 50.0)]
        stutter_ms: f64,
    },
}
#[derive(Subcommand)]
enum HardwareCommand {
    Inspect,
}
#[derive(Subcommand)]
enum BenchmarkCommand {
    /// Run an experimental supervised PresentMon capture (Windows only).
    Live {
        #[arg(long, required_unless_present_any = ["pid", "process_name"], conflicts_with_all = ["pid", "process_name"])]
        exe: Option<PathBuf>,
        #[arg(long, conflicts_with_all = ["exe", "process_name"])]
        pid: Option<u32>,
        #[arg(long, conflicts_with_all = ["exe", "pid"])]
        process_name: Option<String>,
        #[arg(long)]
        presentmon: Option<PathBuf>,
        #[arg(long)]
        swap_chain: Option<String>,
        #[arg(long, default_value_t = 10)]
        warmup: u64,
        #[arg(long, default_value_t = 60)]
        duration: u64,
        #[arg(long, default_value_t = 5)]
        shutdown_timeout: u64,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        diagnostics: Option<PathBuf>,
        #[arg(last = true)]
        arguments: Vec<String>,
    },
    Run {
        #[arg(long)]
        executable: PathBuf,
        #[arg(long, default_value_t = 1000)]
        warmup_ms: u64,
        #[arg(long, default_value_t = 5000)]
        duration_ms: u64,
        #[arg(long, default_value_t = 500)]
        interval_ms: u64,
        #[arg(long)]
        output: PathBuf,
        #[arg(last = true)]
        arguments: Vec<String>,
    },
}

// The guard also cleans up if sampling or writing fails.
struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Commands::Stress {
            command: StressCommand::Run { scenario, output },
        } => {
            let config: gdpl_stress::Config = serde_json::from_reader(fs::File::open(scenario)?)?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)?;
            let cancelled = Arc::new(AtomicBool::new(false));
            let flag = cancelled.clone();
            ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))?;
            let report = if let Some(external) = config.external.clone() {
                gdpl_stress::run(
                    config,
                    &mut gdpl_stress::external::External::new(external),
                    &cancelled,
                )?
            } else {
                gdpl_stress::run(
                    config,
                    &mut gdpl_stress::SyntheticMemory::default(),
                    &cancelled,
                )?
            };
            serde_json::to_writer_pretty(&mut file, &report)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            println!("Stress run: {}", report.termination);
            if report.cleanup != "completed" {
                return Ok(5);
            }
            return Ok(match report.termination.as_str() {
                "completed" => 0,
                "cancelled" => 3,
                "memory_limit" | "cpu_limit" | "duration_limit" => 4,
                _ => 5,
            });
        }
        Commands::Frames {
            command:
                FrameCommand::Analyze {
                    input,
                    pid,
                    swap_chain,
                    stutter_ms,
                },
        } => {
            let analysis = gdpl_analyzer::analyze_presentmon(
                fs::File::open(input)?,
                pid,
                &swap_chain,
                stutter_ms,
            )?;
            println!("{}", serde_json::to_string_pretty(&analysis)?);
        }
        Commands::Doctor => println!(
            "External process sampling: available; experimental PresentMon capture requires a compatible Windows installation; engine adapters: unavailable. Use hardware inspect for detected host data."
        ),
        Commands::Hardware {
            command: HardwareCommand::Inspect,
        } => println!(
            "{}",
            serde_json::to_string_pretty(&gdpl_hardware::inspect())?
        ),
        Commands::Benchmark {
            command:
                BenchmarkCommand::Live {
                    exe,
                    pid,
                    process_name,
                    presentmon,
                    swap_chain,
                    warmup,
                    duration,
                    shutdown_timeout,
                    output,
                    diagnostics,
                    arguments,
                },
        } => {
            return live::run(live::Options {
                exe,
                pid,
                process_name,
                presentmon,
                swap_chain,
                warmup,
                duration,
                shutdown_timeout,
                output,
                diagnostics,
                arguments,
            })
            .map(|valid| if valid { 0 } else { 2 });
        }
        Commands::Benchmark {
            command:
                BenchmarkCommand::Run {
                    executable,
                    warmup_ms,
                    duration_ms,
                    interval_ms,
                    output,
                    arguments,
                },
        } => {
            if duration_ms == 0
                || interval_ms < 200
                || interval_ms > duration_ms
                || warmup_ms > 3_600_000
                || duration_ms > 3_600_000
            {
                return Err("duration must be 200–3600000 ms, interval 200 ms through duration, and warmup at most 3600000 ms".into());
            }
            let executable = fs::canonicalize(executable)?;
            let started_unix_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
            let cancelled = Arc::new(AtomicBool::new(false));
            let flag = cancelled.clone();
            ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))?;
            let mut child = ChildGuard(
                Command::new(&executable)
                    .args(&arguments)
                    .stdin(Stdio::null())
                    .spawn()?,
            );
            let pid = Pid::from_u32(child.0.id());
            let mut system = System::new();
            let start = Instant::now();
            let mut samples = Vec::new();
            let mut termination = "duration_reached";
            let mut next_sample = Duration::from_millis(warmup_ms);
            let end = Duration::from_millis(warmup_ms + duration_ms);
            loop {
                if cancelled.load(Ordering::SeqCst) {
                    termination = "cancelled";
                    break;
                }
                if child.0.try_wait()?.is_some() {
                    termination = "process_exited";
                    break;
                }
                let elapsed = start.elapsed();
                if elapsed >= end {
                    break;
                }
                if elapsed >= next_sample {
                    system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
                    if elapsed >= Duration::from_millis(warmup_ms)
                        && let Some(process) = system.process(pid)
                    {
                        samples.push(Sample {
                            elapsed_ms: (elapsed.as_millis() as u64).saturating_sub(warmup_ms),
                            process_cpu_percent: process.cpu_usage(),
                            process_memory_bytes: process.memory(),
                        });
                    }
                    next_sample = elapsed + Duration::from_millis(interval_ms);
                }
                thread::sleep(Duration::from_millis(25));
            }
            drop(child);
            let mut notes = Vec::new();
            // sysinfo needs two refreshes for CPU usage; the first sample is not meaningful.
            if !samples.is_empty() {
                samples.remove(0);
            }
            if termination != "duration_reached" {
                notes.push(format!("Measurement ended early: {termination}"));
            }
            if samples.is_empty() {
                notes.push("No usable samples were collected".into());
            }
            notes.push("External process only; child processes are excluded. Environment and benchmark determinism are not verified.".into());
            let valid = termination == "duration_reached" && !samples.is_empty();
            let result = BenchmarkResult {
                schema_version: SCHEMA_VERSION,
                started_unix_ms,
                executable: executable.to_string_lossy().into_owned(),
                arguments,
                warmup_ms,
                requested_duration_ms: duration_ms,
                sample_interval_ms: interval_ms,
                termination: termination.into(),
                valid,
                validity_notes: notes,
                hardware: gdpl_hardware::inspect(),
                samples,
                unavailable_metrics: vec![
                    "frame timing and FPS".into(),
                    "GPU and VRAM".into(),
                    "engine telemetry".into(),
                    "thermal and power".into(),
                ],
            };
            // Create-new prevents accidentally replacing existing evidence.
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)?;
            serde_json::to_writer_pretty(&mut file, &result)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            println!("Benchmark saved; valid: {}", result.valid);
            return Ok(if result.valid { 0 } else { 2 });
        }
    }
    Ok(0)
}
fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
