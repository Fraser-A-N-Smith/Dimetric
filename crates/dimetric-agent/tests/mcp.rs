//! The MCP server.
//!
//! The thing worth testing is not the JSON-RPC framing but the claim the server
//! makes: that it is the CLI, spelled differently. So the tests check that the
//! tool list covers what §5.10 asks for, and that a call arrives at the same
//! command a shell would have run.

use dimetric_agent::mcp;
use serde_json::{json, Value};

fn reply(request: Value) -> Value {
    let tools = mcp::tools();
    let line = mcp::respond(&request.to_string(), &tools).expect("a request gets an answer");
    serde_json::from_str(&line).expect("the answer is JSON")
}

fn call(name: &str, arguments: Value) -> Value {
    reply(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    }))["result"]
        .clone()
}

fn project() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/sorcerer").to_string()
}

#[test]
fn initialize_names_the_protocol_and_the_server() {
    let answer = reply(json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }));
    assert_eq!(answer["result"]["serverInfo"]["name"], "dimetric");
    assert!(answer["result"]["capabilities"]["tools"].is_object());
}

#[test]
fn a_notification_gets_no_answer() {
    // It has no id, so there is nothing to answer to.
    let tools = mcp::tools();
    let line = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
    assert!(mcp::respond(&line, &tools).is_none());
}

#[test]
fn the_tools_cover_what_the_design_asks_for() {
    // §5.10's capability table, stage by stage. A command dropped from the CLI
    // would fail here rather than quietly leaving an agent without it.
    let tools = mcp::tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    for required in [
        "scene_tree",
        "scene_query",
        "scene_fmt",
        "node_get",
        "node_create",
        "node_set",
        "node_reparent",
        "node_delete",
        "prefab_instance",
        "override_set",
        "override_clear",
        "script_write",
        "tile_fill",
        "tile_set",
        "tile_get",
        "tile_import_ldtk",
        "asset_import",
        "asset_list",
        "run",
        "state_dump",
        "frame_capture",
        "replay",
        "build",
    ] {
        assert!(names.contains(&required), "no tool called {required}");
    }
}

#[test]
fn the_server_is_not_one_of_its_own_tools() {
    let tools = mcp::tools();
    assert!(!tools.iter().any(|t| t.name == "mcp"));
}

#[test]
fn every_tool_takes_the_global_arguments() {
    // `--project` is global on the CLI, and a tool that could not say which
    // project it meant would be useless for most of the surface.
    let answer = reply(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }));
    let tools = answer["result"]["tools"].as_array().expect("a list");
    assert!(!tools.is_empty());
    for tool in tools {
        let properties = &tool["inputSchema"]["properties"];
        assert!(
            properties["project"].is_object(),
            "{} cannot be told where the project is",
            tool["name"]
        );
    }
}

#[test]
fn a_call_reaches_the_command_and_brings_its_json_back() {
    let result = call(
        "scene_tree",
        json!({ "project": project(), "scene": "arena01.dim" }),
    );
    assert_eq!(result["isError"], false);
    let nodes = result["structuredContent"]["nodes"]
        .as_array()
        .expect("the tree came back structured");
    assert!(nodes.iter().any(|n| n["name"] == "Player"));
}

#[test]
fn a_number_is_as_good_as_its_text() {
    // A model that sends 30 means what one that sends "30" means.
    let result = call(
        "run",
        json!({
            "project": project(),
            "scene": "arena01.dim",
            "headless": true,
            "ticks": 30,
            "seed": "7",
        }),
    );
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["ticks"], 30);
}

#[test]
fn a_command_that_says_no_comes_back_as_a_result_with_its_code() {
    // Not a protocol error: the request was fine and the answer was no.
    let result = call(
        "node_get",
        json!({ "project": project(), "scene": "arena01.dim", "path": "/Arena01/Nope" }),
    );
    assert_eq!(result["isError"], true);
    let code = result["structuredContent"]["diagnostics"][0]["code"].clone();
    assert_eq!(code, "DIM0401");
}

#[test]
fn an_argument_the_tool_does_not_have_is_refused_by_name() {
    let result = call(
        "scene_tree",
        json!({ "project": project(), "scene": "arena01.dim", "depth": 2 }),
    );
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("depth"), "{text}");
}

#[test]
fn a_missing_required_argument_is_refused_before_anything_runs() {
    let result = call("scene_query", json!({ "project": project() }));
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    assert!(text.contains("path"), "{text}");
}

#[test]
fn an_unknown_tool_is_a_protocol_error() {
    let answer = reply(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "scene_teleport", "arguments": {} },
    }));
    assert_eq!(answer["error"]["code"], -32602);
}

#[test]
fn a_line_that_is_not_json_is_answered_rather_than_swallowed() {
    let tools = mcp::tools();
    let line = mcp::respond("{not json", &tools).expect("an answer");
    let answer: Value = serde_json::from_str(&line).expect("the answer is JSON");
    assert_eq!(answer["error"]["code"], -32700);
}
