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
use terminator_core::read_frame;
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
    let mut drains = Vec::new();
    for (i, s) in sessions.iter().take(12).enumerate() {
        let mut stream = h.attach(s)?;
        h.write(&mut stream,"i=0; while [ $i -lt 10000 ]; do printf 'load %s: sample terminal output\\n' \"$i\"; i=$((i+1)); sleep 0.1; done\n")?;
        if i < 6 {
            let stop = Arc::clone(&stop);
            let bytes = Arc::clone(&bytes);
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            drains.push(thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match read_frame::<Value>(&mut stream) {
                        Ok(v) => {
                            if let Some(data) = v.get("Data").and_then(Value::as_str)
                                && let Ok(data) = B64.decode(data)
                            {
                                bytes.fetch_add(data.len() as u64, Ordering::Relaxed);
                            }
                        }
                        Err(_) => break,
                    }
                }
            }));
        }
    }
    let start = Instant::now();
    let mut hint = None::<Value>;
    let mut last_state = h.state()?;
    let mut wire_bytes = 0usize;
    let mut unchanged = 0usize;
    let mut seen = std::collections::HashMap::new();
    let mut history_creations = 0usize;
    let mut history_changes = 0usize;
    let mut times = Vec::new();
    let mut rss = Vec::new();
    while start.elapsed() < duration {
        let t = Instant::now();
        let response: Value = read_frame(&mut h.connect(
            json!("Snapshot"),
            None,
            if conditional { hint.clone() } else { None },
        )?)?;
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        wire_bytes =
            wire_bytes.saturating_add(serde_json::to_vec(&response)?.len().saturating_add(4));
        if let Some(state) = response.get("State") {
            last_state = state.clone();
            hint = Some(json!({"revision":state["revision"],"generation":state["generation"]}));
        } else {
            ensure!(response == "Unchanged", "Invalid load snapshot");
            unchanged = unchanged.saturating_add(1);
        }
        let state = &last_state;
        for entry in fs::read_dir(h.root.join("history"))? {
            let entry = entry?;
            if entry.path().extension().is_none_or(|e| e != "pty") {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                let value = (meta.len(), meta.modified().ok());
                match seen.insert(entry.path(), value) {
                    None => history_creations = history_creations.saturating_add(1),
                    Some(old) if old != value => {
                        history_changes = history_changes.saturating_add(1);
                    }
                    _ => {}
                }
            }
        }
        ensure!(
            crate::harness::sessions(state)
                .iter()
                .filter(|s| s["lifecycle"] == "running")
                .count()
                >= 50,
            "Load session ended"
        );
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
            rss.push(value);
        }
        thread::sleep(Duration::from_millis(100).saturating_sub(t.elapsed()));
    }
    stop.store(true, Ordering::Relaxed);
    for task in drains {
        let _ = task.join();
    }
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
