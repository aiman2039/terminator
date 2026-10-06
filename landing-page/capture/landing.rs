use super::{Harness, Options, PathBuf, Result, fs, git, id, json, plain, prefs, save_prefs};

pub fn run(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.env
        .insert("TERMINATOR_TEST_SIZE".into(), "[1760,1000]".into());
    h.setup()?;
    let web = h.project("atlas-web")?;
    let api = h.project("atlas-api")?;
    let root = PathBuf::from(
        web.get("path")
            .and_then(super::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?,
    );
    fs::create_dir_all(root.join("src"))?;
    fs::write(root.join("README.md"), "# Atlas web\n\nA small **sample project**.\n\n- [x] Auth\n- [ ] Billing\n\n| Route | Status |\n|---|---|\n| /home | ok |\n")?;
    fs::write(root.join("src/app.ts"), "export const App = () => \"hello\";\n")?;
    fs::write(root.join("src/routes.ts"), "export const routes = [\"/home\"];\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["add", "-A"])?;
    git(
        &root,
        &["-c", "user.name=Demo", "-c", "user.email=demo@example.com", "commit", "-qm", "init"],
    )?;
    fs::write(root.join("src/app.ts"), "export const App = () => \"hello atlas\";\nexport const version = 2;\n")?;
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
    save_prefs(
        &h,
        &json!({"version":1,"typography_migrated":true,"attention_migrated":true}),
    )?;
    h.write(&mut h.attach(&left)?, "printf '\\033[2J\\033[H'; printf '\\033[32m$\\033[0m npm run dev\\n\\n  VITE ready in 312 ms\\n\\n  Local: http://localhost:5173/\\n'\n")?;
    h.write(&mut h.attach(&right)?, "printf '\\033[2J\\033[H'; printf '\\033[32m$\\033[0m cargo test\\n\\ntest result: ok. 42 passed; 0 failed\\n'\n")?;
    let scene = |name: &str, tool: &str, ide: bool, actions: serde_json::Value, ms: u64| -> Result<()> {
        h.layout(&web, &[left.clone(), right.clone()])?;
        save_prefs(
            &h,
            &json!({"version":1,"typography_migrated":true,"attention_migrated":true,"visible":true,"tool":tool,"ide_mode":ide,"left_visible":true}),
        )?;
        plain(&h, o, name, actions, ms)?;
        Ok(())
    };
    scene("workspace", "explorer", false, json!([]), 2500)?;
    scene("git", "git", false, json!([]), 2600)?;
    scene("git-diff", "git", false, json!([{"at_ms":1200,"target":"git-file-app.ts"}]), 3800)?;
    scene("history", "history", false, json!([]), 2600)?;
    scene("info", "info", false, json!([]), 2600)?;
    let md = h.editor(&web, &root.join("README.md"))?;
    let code = h.editor(&web, &root.join("src/app.ts"))?;
    let scene_ed = |name: &str, tool: &str, pair: &[serde_json::Value], actions: serde_json::Value, ms: u64| -> Result<()> {
        h.layout(&web, pair)?;
        save_prefs(&h, &json!({"version":1,"typography_migrated":true,"attention_migrated":true,"visible":true,"tool":tool,"ide_mode":false,"left_visible":true}))?;
        plain(&h, o, name, actions, ms)?;
        Ok(())
    };
    scene_ed("ide", "explorer", &[md.clone(), left.clone()], json!([{"at_ms":900,"target":"header-overflow"},{"at_ms":1700,"target":"tool-ide-mode"}]), 3400)?;
    scene_ed("nvim", "explorer", &[code.clone(), right.clone()], json!([]), 3000)?;
    scene_ed("markdown", "explorer", &[md.clone(), right.clone()], json!([{"at_ms":900,"target":"markdown-mode:Split"}]), 3000)?;
    let _ = scene_ed("player", "explorer", &[code.clone(), right.clone()], json!([{"at_ms":900,"target":"player-chrome"}]), 3000);
    h.layout(&web, &[left.clone(), right.clone()])?;
    let _ = scene;
    let scene = |name: &str, tool: &str, ide: bool, actions: serde_json::Value, ms: u64| -> Result<()> {
        h.layout(&web, &[left.clone(), right.clone()])?;
        save_prefs(&h, &json!({"version":1,"typography_migrated":true,"attention_migrated":true,"visible":true,"tool":tool,"ide_mode":ide,"left_visible":true}))?;
        plain(&h, o, name, actions, ms)?;
        Ok(())
    };
    scene("settings", "explorer", false, json!([{"at_ms":900,"target":"header-overflow"},{"at_ms":1700,"target":"settings"}]), 2600)?;
    scene("settings-notifications", "explorer", false, json!([{"at_ms":900,"target":"header-overflow"},{"at_ms":1700,"target":"settings"},{"at_ms":2600,"target":"settings-section:Notifications"}]), 3200)?;
    scene("settings-hooks", "explorer", false, json!([{"at_ms":900,"target":"header-overflow"},{"at_ms":1700,"target":"settings"},{"at_ms":2600,"target":"settings-section:Agent Hooks"}]), 3200)?;
    let mut p = prefs(&h)?;
    p.as_object_mut().unwrap().insert("left_agents".into(), json!(true));
    save_prefs(&h, &p)?;
    h.rpc(json!({"Hook":{"protocol_version":1,"event_id":"l1","terminal_session_id":id(&left),"agent_invocation_id":"a1","agent_kind":"sample","state":"waiting_input","request_id":"r1","sequence":1,"summary":"Approve running cargo test","details":"Sample event","resume":null}}))?;
    plain(&h, o, "agents", json!([]), 2600)?;
    Ok(())
}
