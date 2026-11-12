//! `terminator-hook mcp`: a stdio Model Context Protocol server for tools.
//! Dual-era: it serves `2026-07-28` requests (version in `_meta`, plus
//! `server/discover`) and the `initialize` handshake of earlier revisions.
//! Every tool runs a `ctl` command line, with the same limits and refusals.
#[cfg(test)]
mod tests;
mod tools;
#[cfg(test)]
pub(crate) use tools::argv as tool_argv;

use anyhow::Result;
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};
use std::path::PathBuf;

/// The current revision: version per request, no handshake.
pub(crate) const MODERN: &str = "2026-07-28";
/// Revisions that open with `initialize`, newest first.
const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";
/// `UnsupportedProtocolVersionError`.
const UNSUPPORTED_VERSION: i64 = -32022;
const INSTRUCTIONS: &str = "Controls Terminator terminals with the limits of `terminator-hook ctl`. \
Tools never start an agent. `send` presses Enter only when `enter` is true. \
Agent state comes from hooks, never from terminal text.";

/// Global `ctl` options given to `terminator-hook mcp`.
pub(crate) struct Server {
    globals: Vec<String>,
}

fn supported() -> Vec<&'static str> {
    std::iter::once(MODERN)
        .chain(LEGACY.iter().copied())
        .collect()
}

/// Adds `key` to a JSON object (no-op for any other value).
pub(crate) fn insert(object: &mut Value, key: &str, item: Value) {
    if let Some(map) = object.as_object_mut() {
        map.insert(key.to_owned(), item);
    }
}

fn error(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let Some(data) = data {
        insert(&mut error, "data", data);
    }
    json!({"jsonrpc": "2.0", "id": id, "error": error})
}

fn server_info() -> Value {
    json!({"name": "terminator-hook", "version": env!("CARGO_PKG_VERSION")})
}

impl Server {
    pub(crate) fn new(data_dir: Option<PathBuf>, runtime_dir: Option<PathBuf>) -> Self {
        let mut globals = Vec::new();
        if let Some(path) = data_dir {
            globals.push(format!("--data-dir={}", path.display()));
        }
        if let Some(path) = runtime_dir {
            globals.push(format!("--runtime-dir={}", path.display()));
        }
        Self { globals }
    }

    /// Answers one JSON-RPC message; `None` for a notification.
    pub(crate) fn handle(&self, line: &str) -> Option<Value> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return Some(error(&Value::Null, -32700, "Parse error", None));
        };
        let id = message.get("id")?.clone();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Some(error(&id, -32600, "Invalid request", None));
        };
        let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
        let version = params
            .get("_meta")
            .and_then(|meta| meta.get(VERSION_KEY))
            .and_then(Value::as_str);
        if let Some(requested) = version
            && !supported().contains(&requested)
        {
            let data = json!({"supported": supported(), "requested": requested});
            return Some(error(
                &id,
                UNSUPPORTED_VERSION,
                "Unsupported protocol version",
                Some(data),
            ));
        }
        let mut result = match self.dispatch(method, &params) {
            Ok(result) => result,
            Err((code, message)) => return Some(error(&id, code, &message, None)),
        };
        if version == Some(MODERN) {
            insert(&mut result, "resultType", "complete".into());
        }
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    fn dispatch(&self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|asked| LEGACY.contains(asked))
                    .unwrap_or(LEGACY.first().copied().unwrap_or(MODERN));
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": server_info(),
                    "instructions": INSTRUCTIONS,
                }))
            }
            "server/discover" => Ok(json!({
                "supportedVersions": supported(),
                "capabilities": {"tools": {"listChanged": false}},
                "_meta": {"io.modelcontextprotocol/serverInfo": server_info()},
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools::list()})),
            "tools/call" => self.call(params),
            _ => Err((-32601, format!("Method not found: {method}"))),
        }
    }

    fn call(&self, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "Missing tool name".to_owned()))?;
        let empty = Map::new();
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(arguments)) => arguments,
            Some(_) => return Err((-32602, "Tool arguments must be an object".to_owned())),
        };
        let argv = tools::argv(name, arguments).ok_or((-32602, format!("Unknown tool: {name}")))?;
        Ok(match argv.and_then(|argv| self.run(argv)) {
            Ok(value) => value,
            Err(failure) => tool_result(format!("{failure:#}"), None, true),
        })
    }

    fn run(&self, argv: Vec<String>) -> Result<Value> {
        let args: Vec<String> = self.globals.iter().cloned().chain(argv).collect();
        Ok(match crate::control::execute_args(&args)? {
            crate::control::Outcome::Json(value) => {
                tool_result(serde_json::to_string_pretty(&value)?, Some(value), false)
            }
            crate::control::Outcome::Text(text) => tool_result(text, None, false),
            crate::control::Outcome::Exit(code) => {
                let why = match code {
                    2 => "Timed out before the state was reached",
                    3 => "The session ended before the state was reached",
                    _ => "The control failed",
                };
                tool_result(format!("{why} (ctl exit code {code})"), None, true)
            }
        })
    }
}

fn tool_result(text: String, structured: Option<Value>, is_error: bool) -> Value {
    let mut result = json!({"content": [{"type": "text", "text": text}], "isError": is_error});
    if let Some(value) = structured.filter(Value::is_object) {
        insert(&mut result, "structuredContent", value);
    }
    result
}

/// Reads newline-delimited JSON-RPC from stdin and answers on stdout until
/// stdin closes. Diagnostics never go to stdout.
pub fn run(args: &[String]) -> Result<()> {
    #[derive(clap::Parser)]
    #[command(about = "Stdio MCP server for the terminator-hook ctl controls")]
    struct McpCli {
        #[arg(long)]
        data_dir: Option<PathBuf>,
        #[arg(long)]
        runtime_dir: Option<PathBuf>,
    }
    let cli = <McpCli as clap::Parser>::parse_from(
        std::iter::once("terminator-hook mcp".to_owned()).chain(args.iter().cloned()),
    );
    let server = Server::new(cli.data_dir, cli.runtime_dir);
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = server.handle(&line) {
            serde_json::to_writer(&mut stdout, &reply)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
    }
    Ok(())
}
