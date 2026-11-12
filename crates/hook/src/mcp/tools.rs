//! MCP tools: each one is a `terminator-hook ctl` command line. The tool
//! arguments become that command line, and `ctl` parses and runs it, so a
//! tool has exactly the ctl limits: no agent launch, Enter only when
//! `enter` is true, and every worktree removal refusal of the daemon.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};

/// One tool argument.
struct Arg {
    name: &'static str,
    kind: Kind,
    required: bool,
    about: &'static str,
}

#[derive(Clone, Copy)]
enum Kind {
    /// A positional value after `--`.
    Positional,
    /// `--flag=VALUE`.
    Option(&'static str),
    /// `--flag` when true.
    Flag(&'static str),
    /// `--flag=N` for a non-negative integer.
    Integer(&'static str),
    /// A positional value from a fixed list.
    Choice(&'static [&'static str]),
    /// `--flag=VALUE` from a fixed list.
    OptionChoice(&'static str, &'static [&'static str]),
}

struct Tool {
    name: &'static str,
    /// The ctl words before the arguments.
    command: &'static [&'static str],
    about: &'static str,
    args: &'static [Arg],
}

const fn req(name: &'static str, kind: Kind, about: &'static str) -> Arg {
    Arg {
        name,
        kind,
        required: true,
        about,
    }
}

const fn opt(name: &'static str, kind: Kind, about: &'static str) -> Arg {
    Arg {
        name,
        kind,
        required: false,
        about,
    }
}

use Kind::{Choice, Flag, Integer, OptionChoice, Positional};

const SESSION: Arg = req("session", Positional, "Session ID");
const PROJECT: Arg = req("project", Positional, "Project ID, path or name");

const TOOLS: &[Tool] = &[
    Tool {
        name: "list",
        command: &["list"],
        about: "Projects, sessions, worktrees, agents and terminal notices.",
        args: &[],
    },
    Tool {
        name: "agents",
        command: &["agents"],
        about: "Agents with their hook-reported state and age in seconds.",
        args: &[opt(
            "project",
            Kind::Option("--project"),
            "Only this project",
        )],
    },
    Tool {
        name: "wait",
        command: &["wait"],
        about: "Wait for a hook-reported agent state. Never reads terminal text.",
        args: &[
            SESSION,
            req(
                "state",
                OptionChoice("--state", &["waiting", "done", "failed", "any-stop"]),
                "State to wait for",
            ),
            opt("timeout", Integer("--timeout"), "Seconds to wait"),
            opt(
                "changed",
                Flag("--changed"),
                "Accept only a newer agent record (use after send with enter)",
            ),
        ],
    },
    Tool {
        name: "read",
        command: &["read"],
        about: "Terminal text of a session: saved history, or the visible screen.",
        args: &[
            SESSION,
            opt("screen", Flag("--screen"), "Read the visible screen"),
        ],
    },
    Tool {
        name: "send",
        command: &["send"],
        about: "Type literal text into a session. Enter is pressed only when `enter` is true.",
        args: &[
            SESSION,
            req("text", Positional, "Text to type (at most 1 MiB)"),
            opt("enter", Flag("--enter"), "Press Enter after the text"),
        ],
    },
    Tool {
        name: "create",
        command: &["create"],
        about: "Create a shell (or an editor for `file`) in a new tab. Starts no agent.",
        args: &[
            PROJECT,
            opt("cwd", Kind::Option("--cwd"), "Working folder"),
            opt(
                "file",
                Kind::Option("--file"),
                "Open this file in an editor",
            ),
            opt("background", Flag("--background"), "Create without a GUI"),
        ],
    },
    Tool {
        name: "split",
        command: &["split"],
        about: "Create a shell beside a session.",
        args: &[
            SESSION,
            req(
                "direction",
                Choice(&["left", "right", "up", "down"]),
                "Side",
            ),
        ],
    },
    Tool {
        name: "focus",
        command: &["focus"],
        about: "Show and focus a session in the GUI.",
        args: &[SESSION],
    },
    Tool {
        name: "open_file",
        command: &["open-file"],
        about: "Open a file in the GUI with its default viewer.",
        args: &[
            PROJECT,
            req("path", Positional, "File path"),
            opt("text", Flag("--text"), "Open as text"),
        ],
    },
    Tool {
        name: "notify",
        command: &["notify"],
        about: "Post a terminal notice for a session.",
        args: &[
            SESSION,
            req("body", Positional, "Notice text"),
            opt("title", Kind::Option("--title"), "Notice title"),
        ],
    },
    Tool {
        name: "worktree_list",
        command: &["worktree", "list"],
        about: "Git worktrees of a project.",
        args: &[PROJECT],
    },
    Tool {
        name: "worktree_add",
        command: &["worktree", "add"],
        about: "Add a Git worktree and register it as a project.",
        args: &[
            PROJECT,
            req("path", Positional, "New checkout folder"),
            opt("branch", Kind::Option("--branch"), "New branch name"),
            opt(
                "start",
                Kind::Option("--start"),
                "Start revision (default HEAD)",
            ),
        ],
    },
    Tool {
        name: "worktree_remove",
        command: &["worktree", "remove"],
        about: "Remove a worktree project. Refused for dirty, locked or live-session checkouts.",
        args: &[req(
            "project",
            Positional,
            "Worktree project ID, path or name",
        )],
    },
];

fn arg_schema(arg: &Arg) -> Value {
    let mut schema = match arg.kind {
        Kind::Positional | Kind::Option(_) => json!({"type": "string"}),
        Kind::Flag(_) => json!({"type": "boolean"}),
        Kind::Integer(_) => json!({"type": "integer", "minimum": 0}),
        Kind::Choice(values) | Kind::OptionChoice(_, values) => {
            json!({"type": "string", "enum": values})
        }
    };
    super::insert(&mut schema, "description", arg.about.into());
    schema
}

/// The `tools/list` entries, in a fixed order.
pub(super) fn list() -> Vec<Value> {
    TOOLS
        .iter()
        .map(|tool| {
            let properties: Map<String, Value> = tool
                .args
                .iter()
                .map(|arg| (arg.name.to_owned(), arg_schema(arg)))
                .collect();
            let required: Vec<&str> = tool
                .args
                .iter()
                .filter(|arg| arg.required)
                .map(|arg| arg.name)
                .collect();
            json!({
                "name": tool.name,
                "description": tool.about,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                },
            })
        })
        .collect()
}

fn text(name: &str, value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("`{name}` must be a string"))
}

fn one(arg: &Arg, value: &Value, flags: &mut Vec<String>, values: &mut Vec<String>) -> Result<()> {
    match arg.kind {
        Kind::Positional => values.push(text(arg.name, value)?),
        Kind::Option(flag) => flags.push(format!("{flag}={}", text(arg.name, value)?)),
        Kind::Flag(flag) => {
            if value
                .as_bool()
                .with_context(|| format!("`{}` must be true or false", arg.name))?
            {
                flags.push(flag.to_owned());
            }
        }
        Kind::Integer(flag) => {
            let number = value
                .as_u64()
                .with_context(|| format!("`{}` must be a non-negative integer", arg.name))?;
            flags.push(format!("{flag}={number}"));
        }
        Kind::Choice(allowed) | Kind::OptionChoice(_, allowed) => {
            let choice = text(arg.name, value)?;
            ensure!(
                allowed.contains(&choice.as_str()),
                "`{}` must be one of {}",
                arg.name,
                allowed.join(", ")
            );
            match arg.kind {
                Kind::OptionChoice(flag, _) => flags.push(format!("{flag}={choice}")),
                _ => values.push(choice),
            }
        }
    }
    Ok(())
}

/// The ctl command line for a tool call: options as `--flag=value`, then
/// `--`, then the positional values, so no value can become an option.
/// `None` for an unknown tool.
pub(crate) fn argv(name: &str, arguments: &Map<String, Value>) -> Option<Result<Vec<String>>> {
    let tool = TOOLS.iter().find(|tool| tool.name == name)?;
    Some(build(tool, arguments))
}

fn build(tool: &Tool, arguments: &Map<String, Value>) -> Result<Vec<String>> {
    if let Some(unknown) = arguments
        .keys()
        .find(|key| !tool.args.iter().any(|arg| arg.name == *key))
    {
        bail!("Unknown argument `{unknown}`");
    }
    let mut flags = Vec::new();
    let mut values = Vec::new();
    for arg in tool.args {
        match arguments.get(arg.name).filter(|value| !value.is_null()) {
            Some(value) => one(arg, value, &mut flags, &mut values)?,
            None => ensure!(!arg.required, "Missing argument `{}`", arg.name),
        }
    }
    let mut argv: Vec<String> = tool.command.iter().map(|word| (*word).to_owned()).collect();
    argv.extend(flags);
    if !values.is_empty() {
        argv.push("--".to_owned());
        argv.extend(values);
    }
    Ok(argv)
}
