//! Landing-page reference captures from public sample projects only.
use super::{CaptureInput, Harness, Options, PathBuf, Result, Value, fs, git, id, json, plain, prefs, save_prefs};

/// One capture: which right-side tool is open, whether IDE mode is on, and the fixture actions.
struct Scene {
    name: &'static str,
    tool: &'static str,
    ide: bool,
    actions: Value,
    after: u64,
}

fn scene(name: &'static str, tool: &'static str, actions: Value, after: u64) -> Scene {
    Scene { name, tool, ide: false, actions, after }
}

/// Capture every scene with `layout`; a failed scene is reported and the rest still run.
fn capture_all(h: &Harness, o: &Options, web: &Value, layout: &[Value], scenes: Vec<Scene>) -> Result<()> {
    for s in scenes {
        h.layout(web, layout)?;
        save_prefs(
            h,
            &json!({"version":1,"typography_migrated":true,"attention_migrated":true,"visible":true,"tool":s.tool,"ide_mode":s.ide,"left_visible":true}),
        )?;
        if let Err(e) = plain(CaptureInput { h, opts: o, name: s.name, actions: s.actions, after: s.after }) {
            eprintln!("scene {} failed: {e:#}", s.name);
        }
    }
    Ok(())
}

/// Settings opens through the header overflow menu, then optionally a section.
fn settings(section: Option<&str>) -> Value {
    let mut a = vec![json!({"at_ms":1500,"target":"header-overflow"}), json!({"at_ms":1800,"target":"settings"})];
    if let Some(s) = section {
        a.push(json!({"at_ms":2600,"target":format!("settings-section:{s}")}));
    }
    Value::Array(a)
}

pub fn run(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.env
        .insert("TERMINATOR_TEST_SIZE".into(), "[1440,900]".into());
    h.setup()?;
    let web = h.project("atlas-web")?;
    let api = h.project("atlas-api")?;
    let root = PathBuf::from(
        web.get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?,
    );
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("README.md"), "# Atlas web\n\nA small **sample project**.\n\n- [x] Auth\n- [ ] Billing\n\n| Route | Status |\n|---|---|\n| /home | ok |\n")?;
    fs::write(root.join("src/app.ts"), "export const App = () => \"hello\";\n\nexport function route(path: string) {\n  return path;\n}\n")?;
    fs::write(root.join("src/routes.ts"), "export const routes = [\"/home\"];\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "-A"])?;
    git(
        &root,
        &["-c", "user.name=Demo", "-c", "user.email=demo@example.com", "commit", "-qm", "init"],
    )?;
    fs::write(root.join("src/app.ts"), "export const App = () => \"hello atlas\";\nexport const version = 2;\n\nexport function route(path: string) {\n  return path.trim();\n}\n")?;
    fs::write(root.join("README.md"), "# Atlas web\n\nA small **sample project**.\n\n- [x] Auth\n- [x] Billing\n\n| Route | Status |\n|---|---|\n| /home | ok |\n| /pay | ok |\n")?;
    fs::write(root.join("src/billing.ts"), "export const bill = () => 42;\n")?;
    let left = h.shell(&web)?;
    let right = h.shell(&web)?;
    let other = h.shell(&api)?;
    h.rpc(json!({"Rename":{"session":id(&left),"label":"Dev server"}}))?;
    h.rpc(json!({"Rename":{"session":id(&right),"label":"Tests"}}))?;
    h.layout(&web, &[left.clone(), right.clone()])?;
    h.layout(&api, std::slice::from_ref(&other))?;
    h.rpc(json!({"SelectProject":{"project":id(&web)}}))?;
    save_prefs(&h, &json!({"version":1,"typography_migrated":true,"attention_migrated":true}))?;
    h.write(&mut h.attach(&left)?, "printf '\\033[2J\\033[H'; printf '\\033[32m$\\033[0m npm run dev\\n\\n  VITE ready in 312 ms\\n\\n  Local: http://localhost:5173/\\n'\n")?;
    h.write(&mut h.attach(&right)?, "printf '\\033[2J\\033[H'; printf '\\033[32m$\\033[0m cargo test\\n\\ntest result: ok. 42 passed; 0 failed\\n'\n")?;

    let terms = [left.clone(), right.clone()];
    capture_all(&h, o, &web, &terms, vec![
        scene("workspace", "explorer", json!([]), 2500),
        scene("git", "git", json!([]), 2600),
        scene("git-diff", "git", json!([{"at_ms":1200,"target":"git-file-app.ts"}]), 7000),
        scene("history", "history", json!([]), 2600),
        scene("info", "info", json!([]), 2600),
        scene("hosts", "hosts", json!([]), 2600),
        scene("vnc", "vnc", json!([]), 2600),
        scene("settings", "explorer", settings(None), 2600),
        scene("settings-notifications", "explorer", settings(Some("Notifications")), 3200),
        scene("settings-hooks", "explorer", settings(Some("Agent Hooks")), 3200),
    ])?;

    let md = h.editor(&web, &root.join("README.md"))?;
    let code = h.editor(&web, &root.join("src/app.ts"))?;
    capture_all(&h, o, &web, &[md.clone(), left.clone()], vec![Scene { ide: true, ..scene("ide", "explorer", json!([]), 3400) }])?;
    capture_all(&h, o, &web, &[code, right.clone()], vec![
        scene("editor", "structure", json!([]), 4000),
        scene("problems", "problems", json!([]), 4000),
        scene("player", "explorer", json!([{"at_ms":900,"target":"player-chrome"}]), 3000),
    ])?;
    capture_all(&h, o, &web, &[md, right], vec![scene("markdown", "explorer", json!([{"at_ms":900,"target":"markdown-mode:Split"}]), 3000)])?;

    h.layout(&web, &terms)?;
    let mut p = prefs(&h)?;
    p.as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("prefs"))?
        .insert("left_agents".into(), json!(true));
    save_prefs(&h, &p)?;
    h.rpc(json!({"Hook":{"protocol_version":1,"event_id":"l1","terminal_session_id":id(&left),"agent_invocation_id":"a1","agent_kind":"sample","state":"waiting_input","request_id":"r1","sequence":1,"summary":"Approve running cargo test","details":"Sample event","resume":null}}))?;
    plain(CaptureInput { h: &h, opts: o, name: "agents", actions: json!([]), after: 2600 })?;
    Ok(())
}
