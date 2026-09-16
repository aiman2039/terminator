//! Local bundle-replacement continuity fixture. This does not claim to exercise
//! Sparkle's signature/notarization/authorization paths; see docs/UPDATES.md.
use super::*;
use terminator_core::{Paths, ui_control};

fn snapshot(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}
fn identity(snapshot: &Value) -> Value {
    fn strip_rectangles(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.remove("rect");
                map.remove("viewport");
                for value in map.values_mut() {
                    strip_rectangles(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    strip_rectangles(value);
                }
            }
            _ => {}
        }
    }
    let mut workspaces = snapshot["workspaces"].clone();
    strip_rectangles(&mut workspaces);
    json!({"selected":snapshot["selected_project"], "focus":snapshot["active_session"], "workspaces":workspaces, "markdown_modes":snapshot["markdown_modes"]})
}
fn image_layout(value: &mut Value, path: &Path) {
    match value {
        Value::Object(map) if map.contains_key("Terminal") => {
            *value = json!({"Image":{"path":path}});
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                image_layout(value, path);
            }
        }
        Value::Array(values) => {
            for value in values {
                image_layout(value, path);
            }
        }
        _ => {}
    }
}
pub fn run(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let first = h.project("update-visible")?;
    let second = h.project("update-hidden")?;
    let shell = h.shell(&first)?;
    let split = h.shell(&first)?;
    let hidden = h.shell(&second)?;
    let root = PathBuf::from(first["path"].as_str().unwrap());
    let path = root.join("unsaved.md");
    fs::write(&path, "# Saved on disk\n")?;
    let editor = h.editor(&first, &path)?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok()
        },
        8,
    )?;
    h.write(
        &mut h.attach(&editor)?,
        "gg0C# Unsaved across GUI update\u{1b}",
    )?;
    h.wait(
        |_| {
            h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                .is_ok_and(|r| r["Text"] == "1")
        },
        8,
    )?;
    let image = root.join("preview.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([30, 130, 210, 255])).save(&image)?;
    let shell_layout = h.layout(&first, &[shell.clone(), split.clone()])?;
    let editor_layout = h.layout(&first, std::slice::from_ref(&editor))?;
    let mut preview_layout = editor_layout.clone();
    image_layout(&mut preview_layout, &image);
    h.rpc(json!({"SaveLayout":{"project":id(&first), "layout":{
        "version":3,"active":"shells","tabs":[
            {"id":"shells","primary":{"Terminal":id(&shell)},"layout":shell_layout},
            {"id":"markdown","primary":{"Terminal":id(&editor)},"layout":editor_layout},
            {"id":"image","primary":{"Image":{"path":image}},"layout":preview_layout}
        ]
    }}}))?;
    h.layout(&second, std::slice::from_ref(&hidden))?;
    h.rpc(json!({"SelectProject":{"project":id(&first)}}))?;
    h.rpc(json!({"Focus":{"session":id(&shell)}}))?;
    save_prefs(
        &h,
        &json!({"version":1,"hidden_projects":[id(&second)],"markdown_modes":{id(&editor):"Split"}}),
    )?;
    let original = vec![shell.clone(), split, hidden, editor.clone()];
    let generation = h.state()?["generation"].clone();
    let daemon_pid = h.daemon.as_ref().unwrap().0.id();
    h.write(
        &mut h.attach(&shell)?,
        "(i=0; while [ $i -lt 60 ]; do echo update-continuity-$i; i=$((i+1)); sleep 1; done) &\r",
    )?;

    // Install copies, preserving the running daemon's original executable.
    // Model a user Applications installation under the isolated fixture home.
    h.env
        .insert("HOME".into(), h.root.to_string_lossy().into_owned());
    let app = h.root.join("Applications/Terminator.app/Contents/MacOS");
    fs::create_dir_all(&app)?;
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        fs::copy(bin().join(name), app.join(name))?;
    }
    if let Some(framework) = std::env::var_os("TERMINATOR_TEST_SPARKLE_FRAMEWORK") {
        ensure!(cfg!(target_os = "macos"), "Sparkle fixture requires macOS");
        let contents = app.parent().unwrap();
        fs::create_dir_all(contents.join("Frameworks"))?;
        let mut copy = std::process::Command::new("ditto");
        copy.arg(framework)
            .arg(contents.join("Frameworks/Sparkle.framework"));
        output(copy)?;
        let mut info = plist::Dictionary::new();
        for (key, value) in [
            ("CFBundleIdentifier", "dev.terminator.update-fixture"),
            ("CFBundleName", "Terminator Update Fixture"),
            ("CFBundleExecutable", "terminator"),
            ("CFBundlePackageType", "APPL"),
            ("CFBundleVersion", "1"),
            ("CFBundleShortVersionString", env!("CARGO_PKG_VERSION")),
            ("SUFeedURL", "https://127.0.0.1:9/appcast.xml"),
            (
                "SUPublicEDKey",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            ),
        ] {
            info.insert(key.into(), value.into());
        }
        for key in ["SUEnableAutomaticChecks", "SUShowReleaseNotes"] {
            info.insert(key.into(), false.into());
        }
        for key in ["SURequireSignedFeed", "SUVerifyUpdateBeforeExtraction"] {
            info.insert(key.into(), true.into());
        }
        plist::Value::Dictionary(info).to_file_xml(contents.join("Info.plist"))?;
        h.env.insert("TERMINATOR_UPDATE_FIXTURE".into(), "1".into());
        let defaults_home = h.root.join("foundation-home");
        fs::create_dir_all(&defaults_home)?;
        h.env.insert(
            "CFFIXED_USER_HOME".into(),
            defaults_home.to_string_lossy().into(),
        );
    }
    h.env.insert(
        "TERMINATOR_FIXTURE_GUI".into(),
        app.join("terminator").to_string_lossy().into(),
    );
    let mut before = None;
    capture(&h, o, "before-replacement", json!([]), 2600, |_| {
        h.wait(
            |_| snapshot(&h).is_ok_and(|s| s["selected_project"] == first["id"]),
            8,
        )?;
        let initial = snapshot(&h)?;
        if cfg!(target_os = "macos") {
            ensure!(
                initial["update_menu"] == true,
                "Check for Updates is missing from the application menu"
            );
        }
        if std::env::var_os("TERMINATOR_TEST_SPARKLE_FRAMEWORK").is_some() {
            ensure!(
                initial["updater_available"] == true,
                "Sparkle controller did not load in the isolated app bundle"
            );
        }
        before = Some(identity(&initial));
        Ok(())
    })?;
    let history = h.history(id(&shell))?;
    for name in ["terminator", "terminator-daemon", "terminator-hook"] {
        let next = app.join(format!("{name}.new"));
        fs::copy(bin().join(name), &next)?;
        fs::rename(next, app.join(name))?;
    }
    h.env
        .insert("TERMINATOR_TEST_NATIVE_QUIT".into(), "1".into());
    capture(&h, o, "after-replacement", json!([]), 2800, |_| {
        h.wait(
            |_| snapshot(&h).is_ok_and(|s| Some(identity(&s)) == before),
            8,
        )?;
        h.write(&mut h.attach(&shell)?, "echo INPUT_AFTER_GUI_REPLACEMENT\r")?;
        Ok(())
    })?;
    capture(
        &h,
        o,
        "updates-settings",
        json!([
            {"at_ms":700,"target":"settings"},
            {"at_ms":1300,"target":"settings-section:Updates"}
        ]),
        2500,
        |_| Ok(()),
    )?;
    h.assert_pids(&original)?;
    ensure!(
        h.state()?["generation"] == generation && h.daemon.as_ref().unwrap().0.id() == daemon_pid,
        "GUI update replaced daemon generation or PID"
    );
    let after = h.history(id(&shell))?;
    ensure!(
        after != history && after.contains("INPUT_AFTER_GUI_REPLACEMENT"),
        "Output/input did not continue"
    );
    ensure!(
        h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))?["Text"] == "1",
        "GUI replacement discarded the editor buffer"
    );
    ensure!(
        fs::read_to_string(&path)? == "# Saved on disk\n",
        "GUI replacement saved the buffer without permission"
    );
    let new = h.shell(&first)?;
    h.assert_pids(&[new])?;
    let event = json!({"protocol_version":1,"event_id":"after-update-hook","terminal_session_id":id(&shell),"agent_invocation_id":"update-fixture","agent_kind":"custom","provider_session_id":"fixture","state":"waiting_input","request_id":"after-update","sequence":1,"summary":"Hook after GUI replacement","details":"Isolated fixture","resume":null});
    h.write(
        &mut h.attach(&shell)?,
        &format!(
            "printf '%s' {} | {} emit\r",
            terminator_core::quote(&event.to_string()),
            terminator_core::quote(&app.join("terminator-hook").to_string_lossy())
        ),
    )?;
    h.wait(
        |state| {
            state["notifications"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item["summary"] == "Hook after GUI replacement")
            })
        },
        5,
    )?;
    fs::write(
        o.output.join("continuity.json"),
        serde_json::to_vec_pretty(&json!({
            "kind":"local GUI executable replacement; not a signed Sparkle update",
            "daemon_generation":generation,"daemon_pid":daemon_pid,
            "sessions":original.iter().map(|s|json!({"id":s["id"],"pid":s["pid"]})).collect::<Vec<_>>(),
            "restored":before,"unsaved_buffer_preserved":true,"continued_input_output":true
        }))?,
    )?;
    Ok(())
}
