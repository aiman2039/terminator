use super::super::harness::{Harness, output};
use anyhow::{Result, anyhow, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use terminator_core::{read_frame, transport};

/// Counters gathered by the snapshot polling loop.
#[derive(Default)]
struct Samples {
    hint: Option<Value>,
    wire_bytes: usize,
    unchanged: usize,
    seen: std::collections::HashMap<PathBuf, (u64, Option<std::time::SystemTime>)>,
    history_creations: usize,
    history_changes: usize,
    times: Vec<f64>,
    rss: Vec<u64>,
}

pub fn load(duration: Duration, destination: Option<PathBuf>, conditional: bool) -> Result<()> {
    ensure!(
        (1..=3600).contains(&duration.as_secs()),
        "Load duration must be 1–3600 seconds"
    );
    let h = Harness::new()?;
    h.setup()?;
    let projects = (0..5)
        .map(|i| h.project(&format!("load-{i}")))
        .collect::<Result<Vec<_>>>()?;
    let sessions = (0usize..50)
        .map(|i| {
            let index = i.checked_rem(5).unwrap_or(0);
            h.shell(
                projects
                    .get(index)
                    .ok_or_else(|| anyhow!("missing load project"))?,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let stop = Arc::new(AtomicBool::new(false));
    let bytes = Arc::new(AtomicU64::new(0));
    let drains = start_output(&h, &sessions, (&stop, &bytes))?;
    let start = Instant::now();
    let mut samples = Samples::default();
    let mut last_state = h.state()?;
    while start.elapsed() < duration {
        let t = Instant::now();
        samples.snapshot(&h, conditional, &mut last_state)?;
        samples.scan_history(&h)?;
        ensure!(
            crate::harness::sessions(&last_state)
                .iter()
                .filter(|s| s["lifecycle"] == "running")
                .count()
                >= 50,
            "Load session ended"
        );
        samples.record_rss(&h);
        thread::sleep(Duration::from_millis(100).saturating_sub(t.elapsed()));
    }
    stop.store(true, Ordering::Relaxed);
    for task in drains {
        let _ = task.join();
    }
    let Samples {
        wire_bytes,
        unchanged,
        history_creations,
        history_changes,
        mut times,
        rss,
        ..
    } = samples;
    times.sort_by(f64::total_cmp);
    let sample_count = times.len();
    let p50_index = sample_count.checked_div(2).unwrap_or(0);
    let p95_index = sample_count
        .saturating_mul(95)
        .checked_div(100)
        .unwrap_or(0)
        .min(sample_count.saturating_sub(1));
    let p50 = times
        .get(p50_index)
        .copied()
        .ok_or_else(|| anyhow!("missing snapshot p50"))?;
    let p95 = times
        .get(p95_index)
        .copied()
        .ok_or_else(|| anyhow!("missing snapshot p95"))?;
    let report = json!({"duration_seconds":start.elapsed().as_secs_f64(),"conditional":conditional,"snapshot_requests":sample_count,"snapshot_wire_bytes":wire_bytes,"unchanged_responses":unchanged,"snapshot_p50_ms":p50,"history_files_observed":history_creations,"history_metadata_changes_observed":history_changes,"history_activity_scope":"100ms metadata sampling, not syscall counts","sessions":50,"subscribed_streams":6,"gui_rendering_measured":false,"snapshot_p95_ms":p95,"daemon_peak_rss_kib":rss.iter().max(),"bytes_received":bytes.load(Ordering::Relaxed),"platform":std::env::consts::OS});
    if let Some(path) = destination {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}

/// Start output loops in 12 shells and drain six of them; returns the drain threads.
fn start_output(
    h: &Harness,
    sessions: &[Value],
    (stop, bytes): (&Arc<AtomicBool>, &Arc<AtomicU64>),
) -> Result<Vec<thread::JoinHandle<()>>> {
    let mut drains = Vec::new();
    for (i, s) in sessions.iter().take(12).enumerate() {
        let mut stream = h.attach(s)?;
        h.write(&mut stream,"i=0; while [ $i -lt 10000 ]; do printf 'load %s: sample terminal output\\n' \"$i\"; i=$((i+1)); sleep 0.1; done\n")?;
        if i < 6 {
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            drains.push(drain(stream, Arc::clone(stop), Arc::clone(bytes)));
        }
    }
    Ok(drains)
}

/// Count output bytes on one attached stream until `stop`.
fn drain(
    mut stream: transport::Stream,
    stop: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let Ok(v) = read_frame::<Value>(&mut stream) else {
                break;
            };
            if let Some(data) = v.get("Data").and_then(Value::as_str)
                && let Ok(data) = B64.decode(data)
            {
                bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
            }
        }
    })
}

impl Samples {
    /// Request one (optionally conditional) snapshot and time it.
    fn snapshot(&mut self, h: &Harness, conditional: bool, last_state: &mut Value) -> Result<()> {
        let t = Instant::now();
        let response: Value = read_frame(&mut h.connect(
            json!("Snapshot"),
            None,
            if conditional { self.hint.clone() } else { None },
        )?)?;
        self.times.push(t.elapsed().as_secs_f64() * 1000.0);
        self.wire_bytes = self
            .wire_bytes
            .saturating_add(serde_json::to_vec(&response)?.len().saturating_add(4));
        if let Some(state) = response.get("State") {
            *last_state = state.clone();
            self.hint =
                Some(json!({"revision":state["revision"],"generation":state["generation"]}));
        } else {
            ensure!(response == "Unchanged", "Invalid load snapshot");
            self.unchanged = self.unchanged.saturating_add(1);
        }
        Ok(())
    }
    /// Track scrollback file creation and metadata changes.
    fn scan_history(&mut self, h: &Harness) -> Result<()> {
        for entry in fs::read_dir(h.root.join("history"))? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|e| e != "pty") {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let value = (meta.len(), meta.modified().ok());
            match self.seen.insert(entry.path(), value) {
                None => self.history_creations = self.history_creations.saturating_add(1),
                Some(old) if old != value => {
                    self.history_changes = self.history_changes.saturating_add(1);
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn record_rss(&mut self, h: &Harness) {
        let mut cmd = Command::new("ps");
        cmd.args([
            "-o",
            "rss=",
            "-p",
            &h.daemon.as_ref().unwrap().0.id().to_string(),
        ]);
        if let Ok(raw) = output(cmd)
            && let Ok(value) = String::from_utf8_lossy(&raw).trim().parse::<u64>()
        {
            self.rss.push(value);
        }
    }
}
