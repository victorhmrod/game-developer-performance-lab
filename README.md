# Game Developer Performance Lab

GDPL is an early-stage, engine-independent performance tooling project. The implemented slice launches and samples an external process and analyzes existing PresentMon CSV captures. An experimental Windows command supervises an external PresentMon capture, but has not been validated with a real graphics application. A Godot adapter example is present but has not been run or validated with Godot. This is not a release or a complete benchmark suite.

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

Samples contain monotonic elapsed milliseconds, process resident memory bytes and CPU percentage (100% represents one logical CPU). The first CPU observation is discarded because it has no prior interval. Exact sampling intervals can vary with scheduler and collection overhead. CPU and memory come from sysinfo; permissions, OS and process lifetime affect availability. Live frame timings, GPU, VRAM, engine, thermal, power and child-process metrics are unavailable, not zero. The result includes raw samples, hardware metadata, limitations and a UTC Unix timestamp. No verified live frame capture, statistical comparison, HTML report, engine-specific stress workloads, remote execution, project registration, persistence database, GUI or game engine support is implemented.

Tested locally on Windows 11; other platforms have not been verified. See [contributing](CONTRIBUTING.md). GPL-3.0-only licensed.

## Analyzing existing frame captures

`frames analyze` reads an independently captured PresentMon CSV containing `ProcessID`, `SwapChainAddress`, and `MsBetweenDisplayChange`. Select one process and swap chain explicitly; multiple streams must not be pooled. The command emits JSON on stdout, including the raw selected intervals, counts, distribution and warnings. PresentMon is not bundled or installed by GDPL; the separate experimental live command can invoke it. Confirm the capture source, version, chosen process, swap chain and capture settings independently. An empty selection, missing columns, malformed values, or nonpositive/nonfinite interval fails. NA/empty intervals are counted as unavailable; they are not automatically interpreted as dropped frames. Event loss and capture coverage are unknown, not zero. CSV alone cannot prove capture completeness.

PresentMon describes `MsBetweenDisplayChange` as the interval between display changes; it is distinct from `MsBetweenPresents` (interval between Present calls), and neither is an engine simulation-frame duration. Presentation submissions may not be displayed, and generated frames may be included. GDPL does not derive actual timestamps from these intervals. Consult [PresentMon's console documentation](https://github.com/GameTechDev/PresentMon/blob/main/README-ConsoleApplication.md) for capture options and metric definitions.

Percentiles use linear interpolation at `(n - 1) * p` in sorted frame-time samples. Average FPS is `1000 / mean frame time`; median FPS is `1000 / median frame time`. The 1% and 0.1% lows are `1000 / mean` of the slowest `ceil(n * 0.01)` and `ceil(n * 0.001)` frame times, respectively; they are unavailable below 100 and 1,000 samples. Standard deviation uses Bessel's correction; coefficient of variation divides that deviation by the mean. Tukey outliers are observations outside 1.5 interquartile ranges, not automatically discarded. A stutter is an interval strictly over the user-selected threshold (default 50 ms); excess is the sum of time above that threshold. These statistics do not establish confidence, reproducibility, or causal bottlenecks.

## Experimental supervised capture (Windows)

```powershell
cargo run -p gdpl -- benchmark live --exe C:\Games\Game.exe --presentmon C:\Tools\PresentMon.exe --warmup 10 --duration 60 --output live.json --diagnostics capture-diagnostics
```

If exactly one swap chain is observed for the process, GDPL selects it. If multiple are observed, the report lists their addresses and row counts and remains inconclusive; use `--swap-chain ADDRESS` to select explicitly. No streams are pooled. Use exactly one of `--exe PATH`, `--pid ID`, or `--process-name NAME`. Name matching is case-insensitive and requires exactly one match; specify a PID if ambiguous. GDPL opens a Windows process handle and records the creation time, and does not terminate an attached process. It does not follow launcher children. The executable path can be omitted only for PresentMon itself if `PresentMon.exe` is on PATH. GDPL probes both standard output and standard error of the installed executable’s help for required switches, and records any reported version. This is a capability check, not a guarantee of compatibility. The backend and actual graphics capture remain unverified. Arguments following `--` are passed to the launched target.

Warmup starts when the game is launched and completes before PresentMon starts. Capture startup creates an unmeasured gap; the requested capture duration is passed to PresentMon. If a CSV has `CPUStartTime`, GDPL reports first/last values and duplicate/out-of-order counts and rejects nonfinite or negative values. These are CPU frame-start timestamps, not display timestamps, and GDPL does not map them to the requested window. GDPL does not verify actual display timestamps or window boundaries, event loss, or coverage. Reports therefore always mark measurement validity as `inconclusive` and exit with code 2, even when capture execution completes. Missing or invalid CSV, premature target/backend exit, cancellation, and timeouts are failures. The report separates execution, capture integrity, statistical validity, reproducibility, and target evaluation; integrity remains unknown. Capture does not collect process CPU/memory. `--diagnostics` must name a new directory and retains backend help, logs and any CSV. Temporary capture files are otherwise removed. The output must not already exist.

For `--exe`, the command force-terminates the launched game at completion or cancellation; it does not supervise descendants. On cancellation, target exit or timeout, GDPL requests CTRL+BREAK for the PresentMon process group, waits up to `--shutdown-timeout` seconds (default 5, maximum 60), then force-terminates the backend if necessary. The request requires a shared Windows console and can fail. The report records the attempt; ETW session cleanup remains unknown and may require manual inspection. Do not use it with unsaved work. Startup failures after reserving the output path may leave an empty file, and diagnostic preservation failures are recorded in the report. The generated session name is included for troubleshooting, but ETW cleanup is not verified. Only use against authorized applications.

### Manual graphics validation procedure (not yet verified)

On Windows, obtain PresentMon separately and provide its path. Start an authorized rendering application, identify its PID with Task Manager, then run:

```powershell
cargo run -p gdpl -- benchmark live --pid 1234 --presentmon C:\Tools\PresentMon.exe --warmup 10 --duration 60 --output live.json --diagnostics gdpl-diagnostics
```

Inspect `live.json`, the selected PID and `target_creation_time_100ns` (Windows FILETIME ticks since 1601), `swap_chains`, the CSV, and PresentMon log in `gdpl-diagnostics`. If several swap chains appear, repeat the capture with `--swap-chain ADDRESS`, noting that addresses can change between runs. A completed run is still inconclusive: PresentMon readiness, frame-event window boundaries, ETW loss and ETW session cleanup are not verified. GDPL rechecks the process creation time and termination immediately before and after backend launch. PresentMon still attaches by numeric PID, so a target exiting and its PID being reused between those checks remains a race; the held handle cannot make attachment atomic. Check the reported ETW session after failures; GDPL does not stop or verify leftover sessions. `--process-name` is a convenience for unique matches, not proof of identity.

## Synthetic stress runner

```powershell
cargo run -p gdpl -- stress run --scenario scenarios/synthetic-memory.json --output stress-results.json
```

The built-in `synthetic_memory` scenario allocates and touches a configurable number of KiB in the GDPL process, capped at 32 MiB. It does not represent gameplay or stress an external game. The JSON plan specifies fixed or linear load, a per-stage warmup and measurement duration, repetitions, a total duration ceiling, and a resident-memory limit for the GDPL process. CPU percentage is per logical core and can exceed 100%; its optional ceiling uses consecutive samples (`sustained_samples`). Memory is polled about every 250 ms and the limit may be exceeded between polls; use conservative limits. Ctrl+C requests cancellation. CPU and resident-memory observations are real process measurements, not frame measurements.

The result contains raw process samples and distinguishes attempted from fully completed load; `maximum_validated_load` is always null because completion is not evidence of game performance or hardware suitability. Cleanup is attempted on normal completion, stop conditions, and reported provider/telemetry errors; an externally killed process cannot write a final report. For `stress run`, exit codes are 0 for completion, 3 for cancellation, 4 for resource/time limits, 5 for provider/telemetry or cleanup failure, and 1 for configuration or other errors. The output path must be new; a configuration error after creating it may leave an empty file.

The runner also supports explicit increasing `steps` and supervised external processes as described below. Binary-search refinement, frame-time/FPS thresholds, engine adapters and hardware validation are not implemented. Configuring `minimum_fps` fails explicitly. Parameters in the built-in synthetic scenario must be empty; unknown configuration fields are rejected. Self-monitoring includes GDPL and monitoring overhead.


## External stress workloads

On Windows, build both binaries and run the included synthetic external scenario from the repository root:

```powershell
cargo build --workspace --locked
cargo run -p gdpl -- stress run --scenario scenarios/external-process.json --output external-stress.json
```

The example starts `gdpl-workload.exe`, a separate bounded synthetic memory workload; its KiB units do not represent game complexity. Set `external.executable`, `arguments`, `working_directory`, and `timeout_ms` for another cooperative executable. `controlled: true` requires the protocol below. A launched process is owned by GDPL: controlled processes receive a shutdown request, and an unresponsive owned process is terminated. A launched observation-only process (`controlled: false`) is terminated at cleanup because it has no negotiated shutdown mechanism. Children are not supervised or aggregated.

For an already running process, use `"external": {"mode":"attach", "pid":1234}`. Attached processes are observation-only and are never terminated by GDPL. Observation-only plans require `initial_load == maximum_load`; they do not acknowledge or change application workload. On Windows GDPL holds a process handle and checks creation time; other platforms use process start time with one-second precision. Process exit, missing memory, identity mismatch, failed control requests and protocol disconnection stop the run. Startup/control timeouts are per I/O operation, not a hard real-time deadline; provider callbacks and cleanup can extend total elapsed time beyond the configured run limit. User cancellation is observed between operations.

`steps` strategy uses an explicit strictly increasing `steps` array beginning at `initial_load` and ending at `maximum_load`. Fixed and linear strategies remain available. The CPU consecutive-sample counter resets for each repetition and includes that repetition's warmup. The first recorded CPU observation in each measurement window is unavailable because its interval could overlap warmup or the previous stage. Memory limits are sampled and may be exceeded between samples. Reports include target PID, raw process samples, provider-reported metrics, stage counts and summaries, and distinct attempted, acknowledged and completed loads. `maximum_validated_load` remains null: completed workloads have not passed a game-performance target. Initialization, workload, target and telemetry failures are identified separately; cleanup errors are retained separately from the original failure.

### Local workload protocol v1

Controlled launched processes exchange UTF-8 JSON lines over inherited stdin/stdout, with one request in flight and a maximum of 8192 bytes per line including newline. Stdout is reserved for protocol messages; stderr is inherited for diagnostics. Each request has `version: 1`, an increasing numeric `id`, `command`, and `data`. Each response echoes `version` and `id`, has boolean `ok`, and supplies `error` on failure. Unexpected output, malformed messages, EOF, a version or ID mismatch, missing acknowledgement, or I/O timeout fails the run. Writes and responses are individually timed; a blocked process can extend cleanup by the configured timeout.

The runner sends `hello` with `data.versions: [1]`; the response advertises a `capabilities` array including `set_load`. It then sends `initialize` with scenario name, parameters and maximum load. If `reset` is advertised, it is requested before each repetition. `set_load` includes `data.load`; its response must echo `load` and include `ready: true` before warmup begins. If `metrics` is advertised, responses may include up to 64 finite numeric values under `metrics`. Optional `provider_info` and `metric_metadata` maps in `hello` describe provider identity, units and provenance; both are unverified provider statements. The report preserves polled observations, per-stage snapshot summaries, and declared metrics missing from a stage. These are not independent measurements or complete frame distributions. Finally `shutdown` is requested and the owned process is waited on. See [the reference executable](apps/cli/src/bin/gdpl-workload.rs) for a runnable example. This preliminary protocol has no engine SDK, independently verified frame telemetry, external attach control, or distributed transport; it is not yet a stable adapter API.


## Experimental Godot adapter example

The project in `adapters/godot/example` targets Godot 4.5+. It has been exercised with Godot 4.7.2 on Windows, headless and with OpenGL on an AMD Radeon RX 7900 XTX. Install or provide Godot separately, then replace `external.executable` in `scenarios/godot-example.json` with its executable path and run from the repository root:

```powershell
cargo run -p gdpl -- stress run --scenario scenarios/godot-example.json --output godot-results.json
```

The example uses `--headless` and `--no-header`; do not use `--quiet`, which silences stdout. Godot 4.5 documents `--no-header`, needed because stdout carries protocol replies. On Windows launch the main engine executable directly, not the `_console.exe` wrapper: the wrapper starts another process, so monitoring it would measure the wrong CPU and memory. The adapter reports its PID and GDPL rejects a mismatch. To exercise a renderer, remove `--headless` and verify stdout contains only protocol messages. A headless run does not validate rendering performance.

Choose `scenario.parameters.workload` from `nodes`, `physics`, `rendering`, and `animation`. One load unit means one active node, rigid body, mesh instance, or animation player respectively; ceilings are 1000, 128, 512, and 256. The physics case also creates a static floor. Scenes use generated geometry and no proprietary assets. These workloads are synthetic and do not represent game scalability. The adapter clears each workload on reset and shutdown and acknowledges after a process frame only if its observed object count matches the requested load. The tested runs shut down cleanly; abrupt termination and all engine versions remain unverified. The project has no editor plugin or automatic installation.

The adapter declares descriptions in `provider_metric_metadata`. It intends to report Godot's monotonic startup clock, the last adapter `_process` interval, Godot FPS, process/physics time and node count; rendering counters are omitted in headless mode. FPS is Godot's periodically updated monitor, not an independently counted display FPS. The interval is between script callbacks, not display presents. GDPL polls provider values roughly every 250 ms after process sampling: the last callback interval is a snapshot, and means/medians/P95 describe polled snapshots, not every frame. GDPL and Godot clocks are recorded separately without synchronization; `missing_provider_metrics` records declared values absent in a stage. GDPL excludes a provider response received after the measurement boundary or cancellation and counts it in `late_provider_responses`; this bounds the request/response interval but does not verify when the provider produced a cached value. Queue/event loss and graphics presentation coverage are unknown. Unavailable values are omitted, and thresholds using FPS are not implemented.

After installing a compatible Godot engine executable, the opt-in engine test can be run with:

```powershell
$env:GDPL_GODOT = 'C:\path\to\godot.exe'
cargo test -p gdpl --test godot --locked -- --ignored
```

The opt-in test exercises all four workloads headlessly. The graphical rendering run verified object, primitive and draw-call counters; comparison with PresentMon and validation of displayed-frame accuracy remain pending. These measurements do not establish game performance or validated hardware requirements.


### Adapter protocol v1 foundation

The Rust `gdpl-adapter-sdk` crate defines the v1 request envelope, metric definitions, clock-domain labels and a lifecycle state machine. The wire format remains newline-delimited JSON over the controlled process stdin/stdout; requests include `version`, `id`, `command`, and `data`, and replies include the matching version and ID and `ok`. Messages are limited to 8192 bytes, and the runner uses a configurable bounded timeout. See `schemas/adapter-protocol-v1.json` for the envelope and metric definition schema. Existing v1 peers without structured metric definitions remain supported.

Adapters may advertise `list_scenarios`, `set_load`, `reset`, `metrics` and `shutdown`; the Godot example additionally provides scenario discovery and metric definitions. The runner verifies `set_load`, validates advertised scenarios, and rejects unknown or explicitly unavailable metrics when definitions are supplied. Initialization remains responsible for validating workload parameters. The `set_load` response acknowledges readiness and may provide an observed load. A provider should return `ok: false`, an `error` string and optionally an `error_code` when an operation fails.

Metric definitions describe units, source, snapshot method, availability and timestamp domain; the existing descriptions and raw samples are preserved in reports. Godot metric samples do not have individual engine timestamps, so their domain is marked unavailable. Host, engine, OS and graphics-capture clocks are not synchronized; cross-source correlation and per-frame reconstruction are not supported. The current SDK foundation does not yet include clock correlation, complete cross-language bindings, or verified Unreal/Unity integrations.

Runnable protocol inspectors are provided in `crates/gdpl-adapter-sdk/examples/inspect_adapter.rs` and `sdk/csharp/`; pass the adapter executable and its arguments. Both inspect `hello` and `list_scenarios` and then request shutdown. They are examples, not production supervisors: they do not implement timeout or forced cleanup. C and C++ examples are not yet available.
