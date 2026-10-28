#![forbid(unsafe_code)]
mod attach;
mod browser;
mod control;
mod hook_event;
mod output;
use output::Output;
mod shutdown;
use anyhow::{Context, Result};
use terminator_core::*;
fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) != Some("--health-check")
        && let Ok(paths) = Paths::discover()
    {
        crash::install(crash::CrashInstall {
            binary: "terminator-hook",
            version: env!("CARGO_PKG_VERSION"),
            data_dir: paths.data,
            notify: None,
        });
    }
    match args.first().map(String::as_str) {
        Some("--health-check") => {
            Output::line(format_args!(
                "{}:{PROTOCOL_VERSION}",
                env!("CARGO_PKG_VERSION")
            ));
            Ok(())
        }
        Some("ctl") => control::run(args.get(1..).unwrap_or_default()),
        Some("attach") => attach::Attach::run(
            args.get(1).context("Missing session ID")?,
            args.get(2..).unwrap_or_default(),
        ),
        Some("event" | "emit" | "cwd" | "prompt") => {
            // Observational hooks never block or alter an agent's decision.
            let _ = hook_event::HookCommand::run(&args);
            Ok(())
        }
        Some("rpc") => {
            let req: Request = serde_json::from_str(args.get(1).context("Expected JSON request")?)?;
            let response = rpc(&generations::workspace_paths(&Paths::discover()?)?, req)?;
            Output::line(serde_json::to_string(&response)?);
            Ok(())
        }
        Some("install" | "remove" | "inspect") => {
            let home = std::env::var_os("TERMINATOR_CONFIG_HOME")
                .or(std::env::var_os("HOME"))
                .context("No user directory")?;
            let kind = args.get(1).context("Expected agent kind")?;
            if args.first().is_some_and(|arg| arg == "inspect") {
                Output::line(terminator_integrations::installed(
                    std::path::Path::new(&home),
                    kind,
                ));
            } else {
                let path =
                    terminator_integrations::install(terminator_integrations::InstallInput {
                        home: std::path::Path::new(&home),
                        kind,
                        helper: &std::env::current_exe()?,
                        remove: args.first().is_some_and(|arg| arg == "remove"),
                    })?;
                Output::line(path.display());
            }
            Ok(())
        }
        _ => {
            Output::line(
                "terminator-hook ctl --help\nterminator-hook attach SESSION\nterminator-hook event AGENT < payload.json\nterminator-hook emit < normalized-event.json\nterminator-hook cwd PATH\nterminator-hook install|remove|inspect AGENT\nterminator-hook rpc JSON\n\nHooks are inert outside app-created terminals. They never approve or deny agent actions.",
            );
            Ok(())
        }
    }
}
