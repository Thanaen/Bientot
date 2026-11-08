//! Runs every module of `tests/hostile/` on the real `plugins` thread, as the app would (T041,
//! FR-072, SC-004):
//!
//! ```sh
//! cargo run -p bientot-plugins --example hostile-corpus --release
//! ```
//!
//! The runner plays the app. It loads the corpus, sends the day and keeps pressing each plugin's
//! command, and unloads a plugin at its third failure (FR-016). A UI loop runs on the main
//! thread at 60 frames a second and never waits for the plugins thread, and `probe.wat`, a plugin
//! that behaves, times how long a user command takes while the others misbehave.
//!
//! It fails, with a non-zero exit, when:
//! - the process aborts (the run then ends without its report);
//! - a module marked `expect: fails` is not stopped by the three-failure rule, or a module marked
//!   `expect: lives` fails;
//! - the host keeps more than 60 log lines a minute of a plugin;
//! - a UI frame took longer than [`FRAME_LIMIT`], or a probe command longer than
//!   [`PROBE_LIMIT`];
//! - memory is left in the pool, or the working set ends more than 5 MiB above its start once
//!   the instances are dropped.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bientot_plugin_api::event::CommandSource;
use bientot_plugin_api::snapshot::DaySnapshot;
use bientot_plugin_api::{Language, LocaleTexts, Manifest, PluginId};
use bientot_plugins::host::{LogLevel, LogSink};
use bientot_plugins::runtime::{POOL_BYTES, Pool, Runtime, WasmSandbox};
use bientot_plugins::thread::{self, HostUpdate, PluginSpec, Request};

/// What a UI frame may spend on the plugins: sending requests and reading updates.
const FRAME_LIMIT: Duration = Duration::from_millis(50);
/// What a user command of a plugin that behaves may take while the others misbehave. Calls run
/// in slices of about 5 ms, and a user command of another plugin runs between two slices; the
/// longest wait is a start function, which runs unsliced under the 100 M background cap.
const PROBE_LIMIT: Duration = Duration::from_secs(1);
const WORKING_SET_SLACK: u64 = 5 << 20;
const FAILURES_TO_UNLOAD: u32 = 3;
const RUN_LIMIT: Duration = Duration::from_secs(120);
const FRAME: Duration = Duration::from_millis(16);
const PRESS_EVERY: Duration = Duration::from_millis(250);
/// A pressed plugin that has not answered by then is pressed again.
const PRESS_TIMEOUT: Duration = Duration::from_secs(3);
const PROBE_SAMPLES: usize = 40;
const LIVES_PRESSES: u32 = 10;
const LOG_LINES_PER_MINUTE: u32 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    Fails,
    Lives,
}

struct Case {
    name: String,
    expect: Expect,
    spec: PluginSpec,
}

fn main() -> ExitCode {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/hostile");
    let cases = match load_cases(&dir) {
        Ok(cases) => cases,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::from(2);
        }
    };
    let report = run(&cases);
    report.print(&cases);
    if report.problems.is_empty() {
        println!("\nhostile corpus: {} modules, no problem", cases.len());
        ExitCode::SUCCESS
    } else {
        println!();
        for problem in &report.problems {
            println!("FAIL {problem}");
        }
        ExitCode::FAILURE
    }
}

fn load_cases(dir: &Path) -> Result<Vec<Case>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "wat"))
        .collect();
    paths.sort();
    paths.iter().map(|path| load_case(path)).collect()
}

/// A case is a `.wat` file whose first comment lines say `;; expect: fails|lives` and, when it
/// needs more than 2 MiB, `;; memory: <size>`.
fn load_case(path: &Path) -> Result<Case, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{}: not a UTF-8 name", path.display()))?
        .to_owned();
    let header = |key: &str| {
        text.lines()
            .take_while(|line| line.starts_with(";;"))
            .find_map(|line| {
                line.strip_prefix(";; ")?
                    .strip_prefix(key)?
                    .strip_prefix(": ")
            })
            .map(str::trim)
    };
    let expect = match header("expect") {
        Some("fails") => Expect::Fails,
        Some("lives") => Expect::Lives,
        _ => return Err(format!("{name}.wat: no `;; expect: fails|lives` line")),
    };
    let memory = header("memory").unwrap_or("2MiB");
    let wasm = wat::parse_str(&text).map_err(|e| format!("{name}.wat: {e}"))?;
    let manifest = serde_json::json!({
        "id": format!("hostile.{name}"),
        "name": { "en": name },
        "version": "0.1.0",
        "api": "1.0",
        "memory": memory,
        "locales": ["en"],
        "capabilities": ["read:day", "ui:flyout"],
        "events": ["day.refreshed"],
        "commands": [{ "name": "go", "title": { "en": "Go" } }]
    });
    let manifest = Manifest::parse(manifest.to_string().as_bytes(), &[])
        .map_err(|e| format!("{name}: manifest refused: {e:?}"))?;
    let granted = manifest.capabilities.clone();
    let locales = BTreeMap::from([(
        "en".to_owned(),
        LocaleTexts::from_pairs([("title", name.as_str())]),
    )]);
    Ok(Case {
        expect,
        spec: PluginSpec {
            manifest: Arc::new(manifest),
            locales: Arc::new(locales),
            wasm: Arc::new(wasm),
            granted,
            pending: None,
        },
        name,
    })
}

/// Counts the lines the host keeps and drops, per plugin.
#[derive(Default)]
struct Logs(Mutex<HashMap<PluginId, (u32, u32)>>);

impl LogSink for Logs {
    fn write(&self, plugin: &PluginId, _: LogLevel, _: &str) {
        self.0.lock().unwrap().entry(plugin.clone()).or_default().0 += 1;
    }

    fn dropped(&self, plugin: &PluginId, count: u32) {
        self.0.lock().unwrap().entry(plugin.clone()).or_default().1 += count;
    }

    fn host(&self, _: &PluginId, _: &str) {}
}

#[derive(Default)]
struct Track {
    failures: u32,
    first_error: Option<String>,
    unloaded_after: Option<Duration>,
    pressed_at: Option<Instant>,
    presses: u32,
}

#[derive(Default)]
struct Report {
    tracks: HashMap<PluginId, Track>,
    logs: HashMap<PluginId, (u32, u32)>,
    probe: Vec<Duration>,
    longest_frame: Duration,
    elapsed: Duration,
    dropped_requests: u64,
    working_set: Option<(u64, u64, u64)>,
    problems: Vec<String>,
}

fn run(cases: &[Case]) -> Report {
    let mut report = Report::default();
    let before = working_set();
    let pool = Pool::new(POOL_BYTES);
    let logs = Arc::new(Logs::default());
    let (updates, inbox) = mpsc::channel();
    let thread = {
        let pool = pool.clone();
        thread::spawn(
            move || WasmSandbox::new(Runtime::new(pool)),
            Language::En,
            logs.clone(),
            Arc::new(jiff::Timestamp::now),
            Box::new(move |update| {
                let _ = updates.send((Instant::now(), update));
            }),
        )
        .expect("the plugins thread starts")
    };
    let probe = PluginId::parse("hostile.probe").expect("the probe's id is valid");
    let day: DaySnapshot = serde_json::from_str(include_str!(
        "../../bientot-plugin-api/tests/fixtures/api-1.0/day-snapshot.json"
    ))
    .expect("the fixture is a day");

    let started = Instant::now();
    for case in cases {
        report
            .tracks
            .insert(case.spec.manifest.id.clone(), Track::default());
        thread.send(Request::Load(Box::new(case.spec.clone())));
    }
    thread.send(Request::Day(Arc::new(day)));
    let press = |plugin: &PluginId| Request::Command {
        plugin: plugin.clone(),
        command: "go".into(),
        arg: None,
        source: CommandSource::Palette,
    };

    let mut last_press = started;
    while started.elapsed() < RUN_LIMIT {
        let frame = Instant::now();
        while let Ok((at, update)) = inbox.try_recv() {
            match update {
                HostUpdate::Failed { plugin, error, .. } => {
                    let track = report.tracks.entry(plugin.clone()).or_default();
                    track.failures += 1;
                    track.pressed_at = None;
                    track.first_error.get_or_insert(error);
                    if track.failures == FAILURES_TO_UNLOAD {
                        track.unloaded_after = Some(started.elapsed());
                        thread.send(Request::Unload(plugin));
                    }
                }
                HostUpdate::Card {
                    plugin,
                    card: Some(_),
                } if plugin == probe => {
                    let track = report.tracks.entry(plugin).or_default();
                    if let Some(sent) = track.pressed_at.take() {
                        report.probe.push(at.saturating_duration_since(sent));
                    }
                }
                _ => {}
            }
        }
        let now = Instant::now();
        let probe_track = report
            .tracks
            .get_mut(&probe)
            .expect("the corpus has a probe");
        match probe_track.pressed_at {
            Some(sent) if now - sent > PROBE_LIMIT * 5 => {
                report.probe.push(now - sent);
                probe_track.pressed_at = None;
            }
            Some(_) => {}
            None if report.probe.len() < PROBE_SAMPLES => {
                probe_track.pressed_at = Some(now);
                thread.send(press(&probe));
            }
            None => {}
        }
        if now - last_press >= PRESS_EVERY {
            last_press = now;
            for case in cases {
                let id = &case.spec.manifest.id;
                let track = report.tracks.get_mut(id).expect("every case is tracked");
                let due = match case.expect {
                    Expect::Fails => {
                        track.unloaded_after.is_none()
                            && track
                                .pressed_at
                                .is_none_or(|sent| now - sent > PRESS_TIMEOUT)
                    }
                    Expect::Lives => *id != probe && track.presses < LIVES_PRESSES,
                };
                if due {
                    track.presses += 1;
                    if case.expect == Expect::Fails {
                        track.pressed_at = Some(now);
                    }
                    thread.send(press(id));
                }
            }
        }
        report.longest_frame = report.longest_frame.max(frame.elapsed());
        let done = report.probe.len() >= PROBE_SAMPLES
            && cases.iter().all(|case| {
                let track = &report.tracks[&case.spec.manifest.id];
                match case.expect {
                    Expect::Fails => track.unloaded_after.is_some(),
                    Expect::Lives => case.name == "probe" || track.presses >= LIVES_PRESSES,
                }
            });
        if done {
            break;
        }
        std::thread::sleep(FRAME.saturating_sub(frame.elapsed()));
    }
    report.elapsed = started.elapsed();
    // Give the lives cases' last presses time to land before the instances go.
    std::thread::sleep(Duration::from_millis(500));
    while let Ok((_, update)) = inbox.try_recv() {
        if let HostUpdate::Failed { plugin, error, .. } = update {
            let track = report.tracks.entry(plugin).or_default();
            track.failures += 1;
            track.first_error.get_or_insert(error);
        }
    }
    for case in cases {
        thread.send(Request::Unload(case.spec.manifest.id.clone()));
    }
    report.dropped_requests = thread.dropped();
    drop(thread);
    report.logs = logs.0.lock().unwrap().clone();
    if pool.used() != 0 {
        report.problems.push(format!(
            "{} bytes are left in the memory pool after every instance was dropped",
            pool.used()
        ));
    }
    let after = working_set();
    if let (Some(before), Some(after)) = (before, after) {
        report.working_set = Some((before, after, after.saturating_sub(before)));
    }
    report.check(cases);
    report
}

impl Report {
    fn check(&mut self, cases: &[Case]) {
        if self.elapsed >= RUN_LIMIT {
            self.problems
                .push(format!("the corpus did not finish in {RUN_LIMIT:?}"));
        }
        for case in cases {
            let id = &case.spec.manifest.id;
            let track = &self.tracks[id];
            match case.expect {
                Expect::Fails if track.unloaded_after.is_none() => self.problems.push(format!(
                    "{}: {} failure(s), not stopped by the three-failure rule",
                    case.name, track.failures
                )),
                Expect::Lives if track.failures > 0 => self.problems.push(format!(
                    "{}: failed {} time(s): {}",
                    case.name,
                    track.failures,
                    track.first_error.as_deref().unwrap_or("")
                )),
                _ => {}
            }
            let (kept, _) = self.logs.get(id).copied().unwrap_or_default();
            let minutes = self.elapsed.as_secs().div_ceil(60).max(1) as u32;
            if kept > LOG_LINES_PER_MINUTE * minutes {
                self.problems.push(format!(
                    "{}: the host kept {kept} log lines in {minutes} minute(s)",
                    case.name
                ));
            }
        }
        if let Some(flood) = cases.iter().find(|case| case.name == "log-flood") {
            let (_, dropped) = self
                .logs
                .get(&flood.spec.manifest.id)
                .copied()
                .unwrap_or_default();
            if dropped == 0 {
                self.problems
                    .push("log-flood: no line was dropped".to_owned());
            }
        }
        if self.longest_frame > FRAME_LIMIT {
            self.problems.push(format!(
                "a UI frame took {:?}, above {FRAME_LIMIT:?}: it waited for the plugins thread",
                self.longest_frame
            ));
        }
        if self.probe.len() < PROBE_SAMPLES {
            self.problems.push(format!(
                "the probe answered {} of {PROBE_SAMPLES} commands",
                self.probe.len()
            ));
        }
        if let Some(slowest) = self.probe.iter().max()
            && *slowest > PROBE_LIMIT
        {
            self.problems.push(format!(
                "a probe command took {slowest:?}, above {PROBE_LIMIT:?}"
            ));
        }
        if let Some((_, _, grown)) = self.working_set
            && grown > WORKING_SET_SLACK
        {
            self.problems.push(format!(
                "the working set ended {} KiB above its start",
                grown >> 10
            ));
        }
    }

    fn print(&self, cases: &[Case]) {
        println!(
            "{:<28} {:<6} {:>8} {:>10}  first error",
            "module", "expect", "failures", "stopped"
        );
        for case in cases {
            let track = &self.tracks[&case.spec.manifest.id];
            let stopped = track
                .unloaded_after
                .map(|at| format!("{:.2} s", at.as_secs_f64()))
                .unwrap_or_else(|| "-".into());
            let expect = match case.expect {
                Expect::Fails => "fails",
                Expect::Lives => "lives",
            };
            let mut error = track.first_error.clone().unwrap_or_default();
            if error.len() > 90 {
                let cut = (0..=90)
                    .rev()
                    .find(|&i| error.is_char_boundary(i))
                    .unwrap_or(0);
                error.truncate(cut);
                error.push('…');
            }
            println!(
                "{:<28} {:<6} {:>8} {:>10}  {error}",
                case.name, expect, track.failures, stopped
            );
        }
        let mut probe = self.probe.clone();
        probe.sort();
        if let (Some(median), Some(slowest)) = (probe.get(probe.len() / 2), probe.last()) {
            println!(
                "\nprobe commands: {} answered, median {:.1} ms, slowest {:.1} ms",
                probe.len(),
                median.as_secs_f64() * 1e3,
                slowest.as_secs_f64() * 1e3
            );
        }
        println!(
            "longest UI frame: {:.2} ms",
            self.longest_frame.as_secs_f64() * 1e3
        );
        println!(
            "run: {:.1} s, requests dropped by the full queue: {}",
            self.elapsed.as_secs_f64(),
            self.dropped_requests
        );
        if let Some(flood) = cases.iter().find(|case| case.name == "log-flood") {
            let (kept, dropped) = self
                .logs
                .get(&flood.spec.manifest.id)
                .copied()
                .unwrap_or_default();
            println!("log-flood: {kept} lines kept, {dropped} dropped");
        }
        match self.working_set {
            Some((before, after, grown)) => println!(
                "working set: {} KiB before, {} KiB after the instances were dropped (+{} KiB)",
                before >> 10,
                after >> 10,
                grown >> 10
            ),
            None => println!("working set: not measured on this system"),
        }
    }
}

/// The process's working set (resident set on Linux), in bytes.
#[cfg(windows)]
fn working_set() -> Option<u64> {
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }
    let mut counters = ProcessMemoryCounters {
        cb: size_of::<ProcessMemoryCounters>() as u32,
        ..Default::default()
    };
    // SAFETY: the pseudo-handle of the current process needs no closing, and `counters` is a
    // PROCESS_MEMORY_COUNTERS of the size given.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    (ok != 0).then_some(counters.working_set_size as u64)
}

#[cfg(target_os = "linux")]
fn working_set() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib: u64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .trim()
        .strip_suffix("kB")?
        .trim()
        .parse()
        .ok()?;
    Some(kib << 10)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn working_set() -> Option<u64> {
    None
}
