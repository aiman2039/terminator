use super::super::harness::{Harness, artifacts, git, id};
use super::run::symlink;
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
pub fn command_counts(seconds: u64, destination: Option<PathBuf>) -> Result<()> {
    ensure!(
        (3..=60).contains(&seconds),
        "Measurement duration must be 3–60 seconds"
    );
    let mut h = Harness::new()?;
    h.setup()?;
    let shim = h.root.join("bin");
    fs::create_dir_all(&shim)?;
    for program in ["git", "ps"] {
        let real =
            terminator_core::find_executable(program).context("Measurement helper missing")?;
        symlink(&std::env::current_exe()?, &shim.join(program), false)?;
        h.env.insert(
            format!("TERMINATOR_FIXTURE_REAL_{}", program.to_ascii_uppercase()),
            real.to_string_lossy().into_owned(),
        );
    }
    let log = h.root.join("commands.jsonl");
    h.env.insert(
        "PATH".into(),
        format!(
            "{}:{}",
            shim.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    h.env.insert(
        "TERMINATOR_FIXTURE_COMMAND_LOG".into(),
        log.to_string_lossy().into_owned(),
    );
    let project = h.project("command-counts")?;
    git(
        Path::new(
            project
                .get("path")
                .ok_or_else(|| anyhow!("missing project path"))?
                .as_str()
                .unwrap(),
        ),
        &["init", "-q"],
    )?;
    let session = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&session))?;
    let opts = crate::native::Options {
        seconds,
        output: artifacts().join("commands"),
        ..Default::default()
    };
    let mut counts = serde_json::Map::new();
    for visible in [true, false] {
        fs::write(h.root.join("ui-preferences.json"),json!({"version":1,"tool":"Explorer","visible":visible,"typography_migrated":true,"attention_migrated":true}).to_string())?;
        fs::write(&log, "")?;
        crate::native::capture(
            &h,
            &opts,
            if visible { "visible" } else { "hidden" },
            json!([]),
            seconds.saturating_mul(1000),
            |_| Ok(()),
        )?;
        let commands = fs::read_to_string(&log)?
            .lines()
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        counts.insert(
            if visible { "visible_git" } else { "hidden_git" }.into(),
            json!(commands.iter().filter(|c| c["program"] == "git").count()),
        );
    }
    fs::write(&log, "")?;
    let mut hook = h.command("terminator-hook");
    hook.args(["event", "codex"])
        .env("TERMINATOR_SESSION_ID", id(&session))
        .env(
            "TERMINATOR_SESSION_TOKEN",
            fs::read_to_string(h.root.join("run/auth"))?.trim(),
        );
    terminator_core::run_command(
        hook,
        terminator_core::CommandOptions {
            input: Some(b"{}".to_vec()),
            ..Default::default()
        },
    )?;
    counts.insert(
        "ancestry_ps".into(),
        json!(
            fs::read_to_string(&log)?
                .lines()
                .filter(|l| {
                    serde_json::from_str::<Value>(l)
                        .is_ok_and(|v| v.get("program").is_some_and(|program| program == "ps"))
                })
                .count()
        ),
    );
    let report = json!({"seconds":seconds,"counts":counts,"scope":"Actual helper invocations in isolated visible/hidden GUI fixtures and one ancestry lookup; selected-context metadata remains active when Explorer is hidden"});
    if let Some(path) = destination {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    println!("{report}");
    Ok(())
}
