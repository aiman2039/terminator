//! Review regressions use fresh daemons, PTYs and temporary files only.
mod daemon;
mod health;
mod shutdown;
#[cfg(test)]
mod tests;

use crate::harness::Harness;
use anyhow::{Result, anyhow, ensure};
use daemon::{history_burst, large_snapshots, stalled_attachment, terminal_editors};
use health::missing_helper_health;
#[cfg(test)]
use shutdown::wait_relaunch_marker;
use shutdown::{clean_shutdown, idle_shutdown_race, relaunch_shutdown};
use std::thread;
#[cfg(test)]
use std::{fs, time::Duration};

pub fn run() -> Result<()> {
    thread::scope(|scope| -> Result<()> {
        let tasks = [
            ("clean_shutdown", scope.spawn(clean_shutdown)),
            ("relaunch_shutdown", scope.spawn(relaunch_shutdown)),
            ("idle_shutdown_race", scope.spawn(idle_shutdown_race)),
            ("shutdown_reply", scope.spawn(shutdown_reply)),
            ("history_burst", scope.spawn(history_burst)),
            ("missing_helper_health", scope.spawn(missing_helper_health)),
            ("large_snapshots", scope.spawn(large_snapshots)),
            ("stalled_attachment", scope.spawn(stalled_attachment)),
            ("terminal_editors", scope.spawn(terminal_editors)),
        ];
        let mut failures = Vec::new();
        for (name, task) in tasks {
            match task.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{name}: {error:#}")),
                Err(_) => failures.push(format!("{name}: panicked")),
            }
        }
        ensure!(
            failures.is_empty(),
            "Regression failures:\n{}",
            failures.join("\n")
        );
        Ok(())
    })
}

fn shutdown_reply() -> Result<()> {
    let mut h = Harness::new()?;
    for round in 0..32 {
        h.restart()
            .map_err(|error| anyhow!("Shutdown reply round {round}: {error:#}"))?;
    }
    println!("Shutdown acknowledged before daemon exit (32 restarts)");
    Ok(())
}
