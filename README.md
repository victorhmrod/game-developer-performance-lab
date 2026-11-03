# Game Developer Performance Lab

GDPL is an early-stage, engine-independent performance tooling project. The implemented slice launches and samples an external process and analyzes existing PresentMon CSV captures. It does **not** capture frames itself or provide engine integrations. This is not a release or a complete benchmark suite.

## Build and run

Install the current stable Rust toolchain and the platform's native linker, then:

```sh
cargo test --workspace
cargo run -p gdpl -- doctor
cargo run -p gdpl -- hardware inspect
cargo run -p gdpl -- frames analyze --input capture.csv --pid 1234 --swap-chain 0x1234 --stutter-ms 50 > analysis.json
cargo run -p gdpl -- benchmark run --executable /path/to/game --warmup-ms 2000 --duration-ms 10000 --interval-ms 500 --output run.json -- --game-argument
```

On Windows use a Windows executable path. Arguments after `--` are passed directly to the executable. GDPL launches a new process, samples the parent process, and terminates it when the window expires or Ctrl+C is received. Do not use it on programs with unsaved work. It does not supervise or terminate child processes. The output path must not already exist. Exit code 0 means the requested sampling window completed with usable samples; 2 means the run ended early or had no usable samples; 1 means an error. A valid flag only describes this basic capture, not reproducibility or suitability for hardware qualification.

Samples contain monotonic elapsed milliseconds, process resident memory bytes and CPU percentage (100% represents one logical CPU). The first CPU observation is discarded because it has no prior interval. Exact sampling intervals can vary with scheduler and collection overhead. CPU and memory come from sysinfo; permissions, OS and process lifetime affect availability. Live frame timings, GPU, VRAM, engine, thermal, power and child-process metrics are unavailable, not zero. The result includes raw samples, hardware metadata, limitations and a UTC Unix timestamp. No statistical comparison, live frame capture, HTML report, stress workloads, remote execution, project registration, persistence database, GUI or game engine support is implemented.

Tested locally on Windows 11; other platforms have not been verified. See [contributing](CONTRIBUTING.md). GPL-3.0-only licensed.

## Analyzing existing frame captures

`frames analyze` reads an independently captured PresentMon CSV containing `ProcessID`, `SwapChainAddress`, and `MsBetweenDisplayChange`. Select one process and swap chain explicitly; multiple streams must not be pooled. The command emits JSON on stdout, including the raw selected intervals, counts, distribution and warnings. PresentMon is not bundled, invoked, or verified by GDPL. Confirm the capture source, version, chosen process, swap chain and capture settings independently. An empty selection, missing columns, malformed values, or nonpositive/nonfinite interval fails. NA/empty intervals are counted as unavailable; they are not automatically interpreted as dropped frames. Event loss and capture coverage are unknown, not zero. CSV alone cannot prove capture completeness.

PresentMon describes `MsBetweenDisplayChange` as the interval between display changes; it is distinct from `MsBetweenPresents` (interval between Present calls), and neither is an engine simulation-frame duration. Presentation submissions may not be displayed, and generated frames may be included. GDPL does not derive actual timestamps from these intervals. Consult [PresentMon's console documentation](https://github.com/GameTechDev/PresentMon/blob/main/README-ConsoleApplication.md) for capture options and metric definitions.

Percentiles use linear interpolation at `(n - 1) * p` in sorted frame-time samples. Average FPS is `1000 / mean frame time`; median FPS is `1000 / median frame time`. The 1% and 0.1% lows are `1000 / mean` of the slowest `ceil(n * 0.01)` and `ceil(n * 0.001)` frame times, respectively; they are unavailable below 100 and 1,000 samples. Standard deviation uses Bessel's correction; coefficient of variation divides that deviation by the mean. Tukey outliers are observations outside 1.5 interquartile ranges, not automatically discarded. A stutter is an interval strictly over the user-selected threshold (default 50 ms); excess is the sum of time above that threshold. These statistics do not establish confidence, reproducibility, or causal bottlenecks.
