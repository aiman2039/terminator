use super::*;

fn preset(name: &str, kind: &str, program: &str) -> AgentPreset {
    AgentPreset {
        name: name.into(),
        kind: kind.into(),
        program: program.into(),
        args: vec!["--model".into(), "big one".into()],
    }
}

fn installed(kind: &str, program: &str) -> InstalledAgent {
    InstalledAgent {
        kind: kind.into(),
        program: program.into(),
    }
}

#[test]
fn launchers_list_presets_then_installed_agents_with_brand_icons() {
    let list = launchers(
        &[
            preset("Fast", "claude", "claude"),
            preset("Mine", "x", "my-ai"),
        ],
        &[installed("codex", "codex")],
        &[],
    );
    let keys: Vec<_> = list.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(keys, ["preset:Fast", "preset:Mine", "agent:codex"]);
    assert_eq!(list[0].icon, "AgentClaude");
    assert_eq!(list[1].icon, agents::GENERIC_ICON);
    assert_eq!(list[0].command, "claude --model 'big one'");
    assert_eq!(list[2].title(), "New terminal with Codex");
}

#[test]
fn launchers_put_the_last_used_first() {
    let list = launchers(
        &[preset("Fast", "claude", "claude")],
        &[installed("codex", "codex"), installed("pi", "pi")],
        &["agent:pi".into(), "preset:Fast".into()],
    );
    let keys: Vec<_> = list.iter().map(|l| l.key.as_str()).collect();
    assert_eq!(keys, ["agent:pi", "preset:Fast", "agent:codex"]);
}

#[test]
fn unnamed_or_empty_presets_are_not_launchers() {
    let list = launchers(
        &[
            preset(" ", "claude", "claude"),
            preset("Blank", "claude", ""),
            preset("Twice", "", "a"),
            preset("Twice", "", "b"),
        ],
        &[],
        &[],
    );
    assert_eq!(list.len(), 1);
    assert!(list[0].command.starts_with("a "));
}

#[test]
fn launcher_never_presses_enter() {
    for launcher in launchers(
        &[preset("Fast", "claude", "claude")],
        &[installed("cursor", "cursor-agent")],
        &[],
    ) {
        let post = launcher.post_create();
        assert!(!post.enter);
        assert_eq!(post.input, launcher.command);
    }
}
