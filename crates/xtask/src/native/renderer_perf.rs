//! GUI-only CPU/RSS measurements; fixture repaint polling stops after warmup.
use super::{Options, capture};
use crate::harness::{Harness, output};
use crate::notification_perf::parse_cpu_time;
use anyhow::{Result, anyhow, ensure};
use serde_json::json;
use std::{
    fs,
    process::Command,
    thread,
    time::{Duration, Instant},
};

fn resources(pid: u32) -> Result<(f64, u64)> {
    let mut command = Command::new("ps");
    command.args(["-o", "time=", "-o", "rss=", "-p", &pid.to_string()]);
    let raw = String::from_utf8(output(command)?)?;
    let mut fields = raw.split_whitespace();
    let cpu = fields
        .next()
        .and_then(parse_cpu_time)
        .ok_or_else(|| anyhow!("missing GUI CPU time: {raw}"))?;
    let rss = fields
        .next()
        .ok_or_else(|| anyhow!("missing GUI RSS"))?
        .parse()?;
    Ok((cpu, rss))
}

pub fn run(opts: &Options) -> Result<()> {
    ensure!(
        (5..=60).contains(&opts.seconds),
        "Use 5–60 measurement seconds"
    );
    fs::create_dir_all(&opts.output)?;
    let mut results = Vec::new();
    for workload in ["idle", "output"] {
        for trial in 1..=3 {
            let mut h = Harness::new()?;
            h.setup()?;
            h.env
                .insert("TERMINATOR_TEST_PASSIVE_CAPTURE".into(), "1".into());
            let project = h.project("renderer-perf")?;
            let mut sessions = Vec::new();
            for _ in 0..6 {
                let session = h.shell(&project)?;
                if workload == "output" {
                    h.write(&mut h.attach(&session)?,
                        "i=0; while :; do printf 'renderer line %s: abcdefghijklmnopqrstuvwxyz 0123456789\\n' \"$i\"; i=$((i+1)); sleep 0.033; done\n")?;
                }
                sessions.push(session);
            }
            h.layout(&project, &sessions)?;
            let name = format!("{workload}-{trial}");
            let mut measurement = None;
            let logs = capture(
                &h,
                opts,
                &name,
                json!([]),
                opts.seconds.saturating_add(7).saturating_mul(1000),
                |child| {
                    thread::sleep(Duration::from_secs(5));
                    ensure!(child.try_wait()?.is_none(), "GUI exited during warmup");
                    let (cpu_start, mut rss_peak) = resources(child.id())?;
                    let start = Instant::now();
                    while start.elapsed() < Duration::from_secs(opts.seconds) {
                        thread::sleep(Duration::from_millis(500));
                        rss_peak = rss_peak.max(resources(child.id())?.1);
                    }
                    let (cpu_end, rss_end) = resources(child.id())?;
                    let seconds = start.elapsed().as_secs_f64();
                    let cpu_ms = cpu_end - cpu_start;
                    measurement = Some(json!({
                        "workload":workload,"trial":trial,"seconds":seconds,
                        "gui_cpu_ms":cpu_ms,"gui_cpu_percent":cpu_ms / seconds / 10.0,
                        "gui_peak_rss_kib":rss_peak.max(rss_end)
                    }));
                    Ok(())
                },
            )?;
            let renderer = logs
                .lines()
                .find_map(|line| line.strip_prefix("Native renderer: "))
                .ok_or_else(|| anyhow!("renderer identity missing from fixture log"))?;
            fs::write(opts.output.join(format!("{name}.log")), &logs)?;
            let mut row = measurement.ok_or_else(|| anyhow!("measurement missing"))?;
            row.as_object_mut()
                .ok_or_else(|| anyhow!("measurement is not an object"))?
                .insert("renderer".into(), json!(renderer));
            println!("{row}");
            results.push(row);
            fs::write(
                opts.output.join("renderer-perf.json"),
                serde_json::to_vec_pretty(&json!({
                    "platform":std::env::consts::OS,"arch":std::env::consts::ARCH,
                    "visible_terminals":6,"warmup_seconds":5,"fixture_forced_repaints":false,
                    "window":"visible, inactive, mouse passthrough",
                    "scale":opts.scale,"narrow":opts.narrow,
                    "scope":"GUI process CPU time and RSS; no GPU, input latency or startup measurement",
                    "trials":results
                }))?,
            )?;
        }
    }
    Ok(())
}
