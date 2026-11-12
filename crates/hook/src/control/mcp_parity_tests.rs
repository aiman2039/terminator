//! MCP tools become ctl command lines; these check that each one parses to
//! the same ctl control, so it keeps the same limits and daemon refusals.
use super::*;
use serde_json::{Value, json};

fn control(tool: &str, arguments: Value) -> Control {
    let argv = crate::mcp::tool_argv(tool, arguments.as_object().unwrap())
        .unwrap()
        .unwrap();
    parse(&argv).unwrap().command
}

#[test]
fn mcp_send_without_enter_is_ctl_send_without_enter() {
    let Control::Send(args) = control("send", json!({"session": "s", "text": "rm -rf build"}))
    else {
        panic!("not a send");
    };
    assert!(!args.enter);
    assert!(!args.stdin);
    assert_eq!(args.text.as_deref(), Some("rm -rf build"));
    let Control::Send(args) = control(
        "send",
        json!({"session": "s", "text": "go", "enter": false}),
    ) else {
        panic!("not a send");
    };
    assert!(!args.enter);
}

#[test]
fn mcp_text_that_looks_like_the_enter_flag_stays_text() {
    let Control::Send(args) = control("send", json!({"session": "-s", "text": "--enter"})) else {
        panic!("not a send");
    };
    assert!(!args.enter);
    assert_eq!(args.session, "-s");
    assert_eq!(args.text.as_deref(), Some("--enter"));
}

#[test]
fn mcp_send_with_enter_true_is_ctl_send_enter() {
    let Control::Send(args) = control("send", json!({"session": "s", "text": "y", "enter": true}))
    else {
        panic!("not a send");
    };
    assert!(args.enter);
}

#[test]
fn mcp_worktree_remove_is_the_ctl_worktree_remove_request() {
    let Control::Worktree {
        command: Worktree::Remove { project },
    } = control("worktree_remove", json!({"project": "task"}))
    else {
        panic!("not a worktree removal");
    };
    assert_eq!(project, "task");
}

#[test]
fn mcp_worktree_add_and_wait_keep_their_ctl_options() {
    let Control::Worktree {
        command:
            Worktree::Add {
                project,
                path,
                branch,
                start,
            },
    } = control(
        "worktree_add",
        json!({"project": "p", "path": "/tmp/x", "branch": "-b"}),
    )
    else {
        panic!("not a worktree add");
    };
    assert_eq!((project.as_str(), branch.as_deref()), ("p", Some("-b")));
    assert_eq!((path, start.as_str()), (PathBuf::from("/tmp/x"), "HEAD"));
    let Control::Wait(args) = control(
        "wait",
        json!({"session": "s", "state": "done", "timeout": 5, "changed": true}),
    ) else {
        panic!("not a wait");
    };
    assert_eq!(args.state, agents::WaitFor::Done);
    assert_eq!(args.timeout, Some(5));
    assert!(args.changed);
}

#[test]
fn mcp_has_no_shutdown_window_or_browser_control() {
    for argv in [
        &["shutdown", "--stop-all"][..],
        &["window", "close"],
        &["browser", "open", "https://example.invalid"],
    ] {
        let argv: Vec<String> = argv.iter().map(|word| (*word).to_owned()).collect();
        let error = parse(&argv).err().unwrap();
        assert!(error.to_string().contains("not available"), "{error:#}");
    }
}
