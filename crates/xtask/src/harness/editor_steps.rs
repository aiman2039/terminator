use super::{Harness, id, sessions};
use anyhow::Result;
use serde_json::{Value, json};

impl Harness {
    /// Wait for the editor to answer status requests, type `keys`, then wait
    /// until it reports exactly one modified buffer.
    #[track_caller]
    pub fn dirty_editor(&self, editor: &Value, keys: &str, seconds: u64) -> Result<()> {
        self.wait(
            |_| {
                self.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                    .is_ok()
            },
            seconds,
        )?;
        self.write(&mut self.attach(editor)?, keys)?;
        self.wait(
            |_| {
                self.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                    .is_ok_and(|status| status.get("Text").is_some_and(|text| text == "1"))
            },
            seconds,
        )?;
        Ok(())
    }
    /// Wait for a running editor (labelled `label` when given), type `keys`,
    /// then wait up to 3 seconds for at least one modified buffer.
    #[track_caller]
    pub fn type_into_running_editor(&self, label: Option<&str>, keys: &str) -> Result<()> {
        let running = |s: &Value| {
            s["kind"] == "editor"
                && s["lifecycle"] == "running"
                && label.is_none_or(|label| s["label"] == label)
        };
        let state = self.wait(|st| sessions(st).iter().any(running), 5)?;
        let editor = sessions(&state).iter().find(|s| running(s)).unwrap();
        self.write(&mut self.attach(editor)?, keys)?;
        self.wait(
            |_| {
                self.rpc(json!({"EditorStatus":{"session":id(editor)}}))
                    .is_ok_and(|r| {
                        r.get("Text")
                            .and_then(Value::as_str)
                            .and_then(|s| s.trim().parse::<u32>().ok())
                            .is_some_and(|n| n > 0)
                    })
            },
            3,
        )?;
        Ok(())
    }
}
