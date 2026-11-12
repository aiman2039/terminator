use super::*;
use serde_json::json;

fn ask(server: &Server, message: &Value) -> Value {
    server.handle(&message.to_string()).unwrap()
}

fn modern(method: &str, params: Value) -> Value {
    let mut params = params;
    params["_meta"] = json!({
        VERSION_KEY: MODERN,
        "io.modelcontextprotocol/clientInfo": {"name": "fixture", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})
}

#[test]
fn mcp_legacy_initialize_negotiates_a_handshake_version() {
    let server = Server::new(None, None);
    let reply = ask(
        &server,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}}}),
    );
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert!(reply["result"]["capabilities"]["tools"].is_object());
    let newer = ask(
        &server,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": "2099-01-01"}}),
    );
    assert_eq!(newer["result"]["protocolVersion"], "2025-11-25");
    assert!(
        server
            .handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string())
            .is_none()
    );
}

#[test]
fn mcp_modern_discover_and_results_carry_the_modern_shape() {
    let server = Server::new(None, None);
    let discover = ask(&server, &modern("server/discover", json!({})));
    let result = &discover["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(result["supportedVersions"][0], MODERN);
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "terminator-hook"
    );
    let list = ask(&server, &modern("tools/list", json!({})));
    assert_eq!(list["result"]["resultType"], "complete");
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    for name in [
        "list",
        "agents",
        "wait",
        "read",
        "send",
        "create",
        "split",
        "focus",
        "open_file",
        "notify",
        "worktree_list",
        "worktree_add",
        "worktree_remove",
    ] {
        assert!(names.contains(&name), "{name} missing");
    }
}

#[test]
fn mcp_unsupported_version_lists_the_supported_ones() {
    let server = Server::new(None, None);
    let mut message = modern("tools/list", json!({}));
    message["params"]["_meta"][VERSION_KEY] = "1900-01-01".into();
    let reply = ask(&server, &message);
    assert_eq!(reply["error"]["code"], -32022);
    assert_eq!(reply["error"]["data"]["requested"], "1900-01-01");
    assert_eq!(reply["error"]["data"]["supported"][0], MODERN);
}

#[test]
fn mcp_unknown_tool_and_method_are_protocol_errors() {
    let server = Server::new(None, None);
    let tool = ask(
        &server,
        &modern(
            "tools/call",
            json!({"name": "launch_agent", "arguments": {}}),
        ),
    );
    assert_eq!(tool["error"]["code"], -32602);
    let method = ask(&server, &modern("resources/list", json!({})));
    assert_eq!(method["error"]["code"], -32601);
    assert_eq!(server.handle("{not json").unwrap()["error"]["code"], -32700);
}

#[test]
fn mcp_bad_arguments_are_tool_errors_before_any_daemon_call() {
    let server = Server::new(Some("/nonexistent-terminator-fixture".into()), None);
    for arguments in [
        json!({"session": "s", "text": "x", "enter": "yes"}),
        json!({"session": "s", "text": "x", "press": true}),
        json!({"session": "s"}),
    ] {
        let reply = ask(
            &server,
            &modern(
                "tools/call",
                json!({"name": "send", "arguments": arguments}),
            ),
        );
        assert_eq!(reply["result"]["isError"], true, "{arguments}");
    }
}

#[test]
fn mcp_send_presses_enter_only_when_enter_is_true() {
    let call = |arguments: Value| {
        tools::argv("send", arguments.as_object().unwrap())
            .unwrap()
            .unwrap()
    };
    assert_eq!(
        call(json!({"session": "s", "text": "--enter"})),
        ["send", "--", "s", "--enter"]
    );
    assert_eq!(
        call(json!({"session": "s", "text": "go", "enter": false})),
        ["send", "--", "s", "go"]
    );
    assert_eq!(
        call(json!({"session": "s", "text": "go", "enter": true})),
        ["send", "--enter", "--", "s", "go"]
    );
}
