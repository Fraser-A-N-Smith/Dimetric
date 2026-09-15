//! The MCP server: the same commands, as tools.
//!
//! Speaks JSON-RPC 2.0 over stdin and stdout, one message per line, which is
//! what the Model Context Protocol's stdio transport is.
//!
//! The tool list is not written out by hand. It is read from the CLI's own
//! [`clap::Command`] tree at startup, and a call is carried out by building the
//! argument vector that same CLI would have been given and handing it to
//! [`run`](crate::run). There is one command surface, and this is a second way
//! of spelling it rather than a second copy of it — which matters, because the
//! copy is the thing that goes stale.

use std::io::{BufRead, Write};

use clap::{ArgAction, Command, CommandFactory, Parser};
use dimetric_core::{Code, Diagnostic, Diagnostics};
use serde_json::{json, Value};

use crate::cli::Cli;

/// The protocol revision this server implements.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// Serve until stdin closes.
///
/// Every reply goes to stdout and nothing else does, so the command layer is
/// only ever called for its return value. A command that printed as it went
/// would put its output in the middle of a JSON-RPC frame.
pub fn serve() -> Result<(), Diagnostics> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let tools = tools();

    for line in stdin.lock().lines() {
        let line = line.map_err(|e| {
            Diagnostics(vec![Diagnostic::new(
                Code::COMMAND_REJECTED,
                format!("cannot read stdin: {e}"),
            )])
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(reply) = respond(&line, &tools) else {
            continue;
        };
        writeln!(stdout, "{reply}")
            .and_then(|()| stdout.flush())
            .map_err(|e| {
                Diagnostics(vec![Diagnostic::new(
                    Code::COMMAND_REJECTED,
                    format!("cannot write stdout: {e}"),
                )])
            })?;
    }
    Ok(())
}

/// Answer one message, or `None` when it was a notification.
pub fn respond(line: &str, tools: &[Tool]) -> Option<String> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        // No id to answer with, so this is as far as the protocol goes.
        Err(e) => {
            return Some(error_frame(
                Value::Null,
                -32700,
                &format!("invalid JSON: {e}"),
            ))
        }
    };
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let id = request.get("id").cloned();

    // A request carries an id and expects an answer; a notification does not.
    let id = id?;

    match method {
        "initialize" => Some(result_frame(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": { "listChanged": false } },
                "serverInfo": { "name": "dimetric", "version": env!("CARGO_PKG_VERSION") },
            }),
        )),
        "ping" => Some(result_frame(id, json!({}))),
        "tools/list" => {
            let list: Vec<Value> = tools.iter().map(Tool::describe).collect();
            Some(result_frame(id, json!({ "tools": list })))
        }
        "tools/call" => {
            let params = request.get("params").cloned().unwrap_or(Value::Null);
            Some(call(id, &params, tools))
        }
        other => Some(error_frame(
            id,
            -32601,
            &format!("no method called {other:?}"),
        )),
    }
}

/// Carry out `tools/call`.
fn call(id: Value, params: &Value, tools: &[Tool]) -> String {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let Some(tool) = tools.iter().find(|t| t.name == name) else {
        return error_frame(id, -32602, &format!("no tool called {name:?}"));
    };
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

    let argv = match tool.argv(&arguments) {
        Ok(argv) => argv,
        Err(d) => return tool_failure(id, &Diagnostics(vec![d])),
    };
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(e) => {
            return tool_failure(
                id,
                &Diagnostics(vec![Diagnostic::new(
                    Code::COMMAND_REJECTED,
                    e.to_string().trim().to_string(),
                )]),
            )
        }
    };

    match crate::run(cli) {
        Ok(out) => result_frame(
            id,
            json!({
                "content": [{ "type": "text", "text": out.text }],
                "structuredContent": out.body,
                "isError": false,
            }),
        ),
        Err(diagnostics) => tool_failure(id, &diagnostics),
    }
}

/// A tool call that failed.
///
/// A protocol-level error would tell the model its request was malformed. This
/// is a command that ran and said no, so it comes back as a result carrying
/// `isError` and the diagnostics — codes included, which is the whole point of
/// I9.
fn tool_failure(id: Value, diagnostics: &Diagnostics) -> String {
    let text = diagnostics
        .iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    result_frame(
        id,
        json!({
            "content": [{ "type": "text", "text": text }],
            "structuredContent": { "diagnostics": diagnostics },
            "isError": true,
        }),
    )
}

fn result_frame(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_frame(id: Value, code: i32, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

// -- the tool list ------------------------------------------------------

/// One callable command.
pub struct Tool {
    /// Tool name, the subcommand path joined with underscores.
    pub name: String,
    /// The subcommand path itself, as words.
    path: Vec<String>,
    summary: String,
    params: Vec<Param>,
}

/// One argument of a tool.
struct Param {
    name: String,
    /// The flag to pass it under, or `None` for a positional.
    flag: Option<String>,
    /// True for a switch, which takes no value.
    switch: bool,
    /// True when it may be repeated.
    many: bool,
    required: bool,
    doc: String,
}

/// Every command the CLI has, as tools.
///
/// Read from clap rather than listed here, so a subcommand added to the CLI is
/// a tool without anyone remembering to do anything.
pub fn tools() -> Vec<Tool> {
    let mut command = Cli::command();
    // Globals are only pushed down into subcommands once the command is built,
    // and `--project` reaching every tool is the difference between a usable
    // surface and one where half the calls cannot say what they operate on.
    command.build();
    let mut out = Vec::new();
    collect(&command, &[], &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn collect(command: &Command, path: &[String], out: &mut Vec<Tool>) {
    for sub in command.get_subcommands() {
        let name = sub.get_name().to_string();
        // The server is not one of its own tools.
        if path.is_empty() && name == "mcp" {
            continue;
        }
        if name == "help" {
            continue;
        }
        let mut here = path.to_vec();
        here.push(name);
        if sub.get_subcommands().next().is_some() {
            collect(sub, &here, out);
        } else {
            out.push(Tool {
                // `tile import-ldtk` becomes `tile_import_ldtk`: one separator
                // rather than two, so a model does not have to remember which
                // word takes which.
                name: here.join("_").replace('-', "_"),
                summary: summary(sub, &here),
                params: params(sub),
                path: here,
            });
        }
    }
}

fn summary(command: &Command, path: &[String]) -> String {
    command
        .get_about()
        .map(|a| a.to_string())
        .unwrap_or_else(|| format!("dim {}", path.join(" ")))
}

fn params(command: &Command) -> Vec<Param> {
    let mut params = Vec::new();
    for arg in command.get_arguments() {
        if arg.get_id() == "help" || arg.get_id() == "version" {
            continue;
        }
        let switch = matches!(
            arg.get_action(),
            ArgAction::SetTrue | ArgAction::SetFalse | ArgAction::Count
        );
        params.push(Param {
            name: arg.get_id().to_string(),
            flag: arg.get_long().map(|l| format!("--{l}")),
            switch,
            many: !switch && matches!(arg.get_num_args(), Some(n) if n.max_values() > 1),
            required: arg.is_required_set(),
            doc: arg
                .get_help()
                .map(|h| h.to_string())
                .unwrap_or_else(|| arg.get_id().to_string()),
        });
    }
    params
}

impl Tool {
    /// The tool as `tools/list` reports it.
    pub fn describe(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();
        for param in &self.params {
            let schema = if param.switch {
                json!({ "type": "boolean", "description": param.doc })
            } else if param.many {
                json!({
                    "type": "array",
                    "items": { "type": "string" },
                    "description": param.doc,
                })
            } else {
                json!({ "type": "string", "description": param.doc })
            };
            properties.insert(param.name.clone(), schema);
            if param.required {
                required.push(param.name.clone());
            }
        }
        json!({
            "name": self.name,
            "description": self.summary,
            "inputSchema": {
                "type": "object",
                "properties": properties,
                "required": required,
            },
        })
    }

    /// The argument vector the CLI would have been given.
    fn argv(&self, arguments: &Value) -> Result<Vec<String>, Diagnostic> {
        let mut argv = vec!["dim".to_string()];
        argv.extend(self.path.iter().cloned());
        let empty = serde_json::Map::new();
        let given = arguments.as_object().unwrap_or(&empty);

        for (key, _) in given {
            if !self.params.iter().any(|p| &p.name == key) {
                return Err(Diagnostic::new(
                    Code::COMMAND_REJECTED,
                    format!("{} has no argument {key:?}", self.name),
                )
                .with_field("tool", self.name.clone()));
            }
        }

        for param in &self.params {
            let Some(value) = given.get(&param.name) else {
                if param.required {
                    return Err(Diagnostic::new(
                        Code::COMMAND_REJECTED,
                        format!("{} needs {:?}", self.name, param.name),
                    )
                    .with_field("tool", self.name.clone()));
                }
                continue;
            };
            if param.switch {
                // A switch that is false is simply absent, which is what the
                // CLI means by it.
                if value.as_bool() == Some(true) {
                    argv.push(param.flag.clone().unwrap_or_default());
                }
                continue;
            }
            for text in scalars(value) {
                if let Some(flag) = &param.flag {
                    argv.push(flag.clone());
                }
                argv.push(text);
            }
        }
        Ok(argv)
    }
}

/// A JSON value as the strings a command line would carry.
///
/// Numbers and booleans come through as their text, because a model that sends
/// `{"ticks": 600}` means the same thing as one that sends `"600"` and should
/// not be told otherwise.
fn scalars(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items.iter().flat_map(scalars).collect(),
        Value::String(s) => vec![s.clone()],
        Value::Null => Vec::new(),
        other => vec![other.to_string()],
    }
}
