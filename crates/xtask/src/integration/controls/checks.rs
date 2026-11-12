use super::super::super::harness::{id, output, session_present};
use super::super::run::symlink;
use super::{CONTROL_SIGNAL, Checkout, MCP_SIGNAL, SPAWN_SIGNAL};
use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use std::fs;
use terminator_core::transport;

impl Checkout {
    pub(super) fn cli_send_and_read(&self, s: &Value) -> Result<()> {
        let h = &self.h;
        let mut send = h.command("terminator-hook");
        send.args(["ctl", "send", id(s), CONTROL_SIGNAL, "--enter"]);
        output(send)?;
        h.wait(
            |_| {
                h.history(id(s))
                    .is_ok_and(|s| s.contains("CONTROL_SEND_PROOF"))
            },
            5,
        )?;
        let mut read = h.command("terminator-hook");
        read.args(["ctl", "read", id(s), "--screen"]);
        ensure!(
            String::from_utf8_lossy(&output(read)?).contains("CONTROL_SEND_PROOF"),
            "Screen read did not return live content"
        );
        self.mcp_send_enter_only_on_request(s)?;
        self.spawn_types_without_enter(s)
    }
    /// `ctl spawn` links a child to its parent and types `--agent` without
    /// Enter; the child is stopped again so removal checks keep their state.
    fn spawn_types_without_enter(&self, s: &Value) -> Result<()> {
        let h = &self.h;
        let mut spawn = h.command("terminator-hook");
        spawn.args(["ctl", "spawn", "--parent", id(s), "--background", "--agent"]);
        spawn.arg(SPAWN_SIGNAL);
        let reply: Value = serde_json::from_slice(&output(spawn)?)?;
        let child = reply
            .get("session")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("spawn printed no session: {reply}"))?
            .to_owned();
        ensure!(
            reply.get("enter") == Some(&json!(false))
                && reply.get("parent_linked") == Some(&json!(true)),
            "Unexpected spawn reply: {reply}"
        );
        ensure!(
            h.state()?.pointer(&format!("/session_parents/{child}")) == Some(&json!(id(s))),
            "Parent link missing from the snapshot"
        );
        std::thread::sleep(std::time::Duration::from_secs(1));
        ensure!(
            !h.history(&child)?.contains("SPAWN_TYPED_PROOF"),
            "ctl spawn pressed Enter"
        );
        self.stop(&json!({ "id": child }))
    }
    /// MCP `send` types without Enter unless `enter` is true.
    fn mcp_send_enter_only_on_request(&self, s: &Value) -> Result<()> {
        let h = &self.h;
        let typed = self.mcp_call("send", &json!({"session": id(s), "text": MCP_SIGNAL}))?;
        ensure!(
            typed.pointer("/result/isError") == Some(&json!(false)),
            "MCP send failed: {typed}"
        );
        std::thread::sleep(std::time::Duration::from_secs(1));
        ensure!(
            !h.history(id(s))?.contains("MCP_TYPED_PROOF"),
            "MCP send pressed Enter without `enter`"
        );
        let enter = self.mcp_call(
            "send",
            &json!({"session": id(s), "text": "", "enter": true}),
        )?;
        ensure!(
            enter.pointer("/result/isError") == Some(&json!(false)),
            "MCP Enter failed: {enter}"
        );
        h.wait(
            |_| {
                h.history(id(s))
                    .is_ok_and(|text| text.contains("MCP_TYPED_PROOF"))
            },
            5,
        )?;
        Ok(())
    }
    /// CLI and OSC notices stay separate from agent state; returns the attached stream.
    pub(super) fn notices(&self, s: &Value) -> Result<transport::Stream> {
        let h = &self.h;
        let mut notify = h.command("terminator-hook");
        notify.args([
            "ctl",
            "notify",
            id(s),
            "CLI notification",
            "--title",
            "Fixture",
        ]);
        output(notify)?;
        let mut stream = h.attach(s)?;
        h.write(&mut stream, "printf '\\033]9;OSC fixture\\007'\n")?;
        let state = h.wait(|s| s["terminal_notices"].as_array().unwrap().len() >= 2, 5)?;
        ensure!(
            state
                .get("agents")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("missing agents"))?
                .is_empty()
                && state
                    .get("notifications")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow!("missing notifications"))?
                    .is_empty(),
            "Terminal notices changed agent lifecycle"
        );
        let notice = state
            .get("terminal_notices")
            .and_then(|notices| notices.get(0))
            .and_then(|notice| notice.get("id"))
            .ok_or_else(|| anyhow!("missing terminal notice"))?
            .as_str()
            .unwrap();
        let mut dismiss = h.command("terminator-hook");
        dismiss.args(["ctl", "dismiss-notice", notice]);
        output(dismiss)?;
        ensure!(
            h.state()?
                .get("terminal_notices")
                .and_then(|notices| notices.get(0))
                .and_then(|notice| notice.get("dismissed"))
                .is_some_and(|dismissed| dismissed == &json!(true)),
            "Notice dismissal failed"
        );
        Ok(stream)
    }
    pub(super) fn metadata(&self, s: &Value) -> Result<()> {
        let h = &self.h;
        let stub = h.root.join("gh-fixture");
        fs::create_dir(&stub)?;
        symlink(&std::env::current_exe()?, &stub.join("gh"), false)?;
        let mut metadata = h.command("terminator-hook");
        metadata.args(["ctl", "metadata", id(s)]);
        metadata.env("PATH",format!("{}:{}",stub.display(),std::env::var("PATH").unwrap_or_default())).env("TERMINATOR_FIXTURE_GH_JSON",json!({"number":7,"title":"Fixture PR","url":"https://github.com/example/fixture/pull/7","state":"OPEN"}).to_string()).arg("--pr");
        let metadata: Value = serde_json::from_slice(&output(metadata)?)?;
        ensure!(
            metadata
                .get("branch")
                .is_some_and(|branch| branch == "fixture-task")
                && metadata
                    .get("worktree")
                    .is_some_and(|worktree| worktree == &json!(true)),
            "Worktree metadata incorrect: {metadata}"
        );
        ensure!(
            metadata
                .get("pull_request")
                .and_then(|pull_request| pull_request.get("number"))
                .is_some_and(|number| number == &json!(7)),
            "PR metadata fixture failed: {metadata}"
        );
        Ok(())
    }
    pub(super) fn stop(&self, s: &Value) -> Result<()> {
        let h = &self.h;
        h.rpc(json!({"Stop":{"session":id(s)}}))?;
        h.wait(|st| !session_present(st, id(s)), 5)?;
        Ok(())
    }
}
