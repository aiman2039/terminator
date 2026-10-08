use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{fs, io::Write, path::PathBuf, process::Command};
pub fn git_shim() -> Result<()> {
    let name = std::env::args_os()
        .next()
        .and_then(|p| {
            PathBuf::from(p)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .context("Shim program missing")?;
    if name == "gh" {
        let value = std::env::var("TERMINATOR_FIXTURE_GH_JSON")
            .context("The gh fixture requires an explicit JSON payload")?;
        let value: Value = serde_json::from_str(&value)?;
        println!("{value}");
        return Ok(());
    }
    let real = std::env::var_os(format!(
        "TERMINATOR_FIXTURE_REAL_{}",
        name.to_ascii_uppercase()
    ))
    .context("Command shim requires isolated fixture configuration")?;
    let log = std::env::var_os("TERMINATOR_FIXTURE_COMMAND_LOG")
        .context("Missing fixture command log")?;
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    let mut file = fs::OpenOptions::new().create(true).append(true).open(log)?;
    let record =
        json!({"program":name,"args":args.iter().map(|a|a.to_string_lossy()).collect::<Vec<_>>()});
    file.lock()?;
    file.write_all(format!("{record}\n").as_bytes())?;
    file.unlock()?;
    let status = Command::new(real).args(args).status()?;
    std::process::exit(status.code().unwrap_or(1));
}
pub fn browser_live() -> Result<()> {
    crate::browser_fixture::run()
}
