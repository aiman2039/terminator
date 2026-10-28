mod file_reuse;
mod hunks;
mod legacy;
mod neovim;

use super::{
    Context, Duration, Harness, Path, Result, Value, ensure, git, id, json, output, thread,
};
pub use file_reuse::file_reuse;
pub use hunks::hunks;
pub use legacy::legacy;
pub(super) use legacy::proxy;
pub use neovim::run;
use terminator_core::transport;

/// The two read-only snapshot buffers a review must show.
struct Expected {
    left: Value,
    right: Value,
}
fn expression(h: &Harness, s: &Value, expression: &str) -> Result<String> {
    let mut cmd = std::process::Command::new("nvim");
    cmd.arg("--server")
        .arg(transport::nvim_listen_arg(&h.root.join("run").join(
            format!(
                "{}.nvim",
                id(s).get(..8).ok_or_else(|| anyhow::anyhow!(
                    "review socket id is shorter than 8 characters"
                ))?
            ),
        )))
        .args(["--remote-expr", expression]);
    Ok(String::from_utf8(output(cmd)?)?.trim().into())
}
fn inspect(h: &Harness, s: &Value) -> Result<Value> {
    serde_json::from_str(&expression(h,s,"json_encode({'user_init':get(g:,'terminator_user_init',0),'messages':execute('messages'),'buffers':map(filter(getbufinfo(), 'v:val.loaded && !empty(v:val.name)'),{_,b -> {'name':b.name,'modifiable':getbufvar(b.bufnr,'&modifiable'),'readonly':getbufvar(b.bufnr,'&readonly'),'lines':getbufline(b.bufnr,1,'$')}})})")?).map_err(Into::into)
}
fn verify(h: &Harness, s: &Value, expected: Expected) -> Result<()> {
    let Expected { left, right } = expected;
    h.wait(
        |_| {
            expression(h, s, "get(g:, 'terminator_review_ready', 0)")
                .is_ok_and(|s| matches!(s.as_str(), "v:true" | "true" | "1"))
        },
        8,
    )?;
    thread::sleep(Duration::from_millis(150));
    let data = inspect(h, s)?;
    ensure!(
        data.get("user_init")
            .is_some_and(|value| *value == json!(0))
            && data.get("messages").is_some_and(|messages| messages == ""),
        "Isolated review loaded user config or reported error: {data}"
    );
    let buffers = data
        .get("buffers")
        .unwrap_or(&Value::Null)
        .as_array()
        .context("Missing review buffers")?;
    ensure!(
        buffers.len() == 2
            && buffers
                .iter()
                .all(|b| b["modifiable"] == 0 && b["readonly"] == 1),
        "Review buffers not read-only: {data}"
    );
    ensure!(
        buffers
            .first()
            .and_then(|buffer| buffer.get("lines"))
            .is_some_and(|lines| lines == &left)
            && buffers
                .get(1)
                .and_then(|buffer| buffer.get("lines"))
                .is_some_and(|lines| lines == &right),
        "Review compared wrong snapshots: {data}"
    );
    ensure!(
        h.rpc(json!({"EditorStatus":{"session":id(s)}}))?
            .get("Text")
            .is_some_and(|text| text == "0"),
        "Review marked modified"
    );
    Ok(())
}
/// `git init` with a fixture identity.
fn init_repo(root: &Path) -> Result<()> {
    git(root, &["init", "-q"])?;
    git(root, &["config", "user.name", "Fixture"])?;
    git(root, &["config", "user.email", "fixture@example.invalid"])?;
    Ok(())
}
/// Keep fixture commits independent of user signing and hook configuration.
fn disable_signing_and_hooks(root: &Path) -> Result<()> {
    git(root, &["config", "commit.gpgsign", "false"])?;
    git(root, &["config", "core.hooksPath", ".no-fixture-hooks"])?;
    Ok(())
}
