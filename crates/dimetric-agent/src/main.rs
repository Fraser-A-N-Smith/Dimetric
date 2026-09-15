//! `dim` — the Dimetric command-line tool.
//!
//! One of two front-ends over [`dimetric_agent::run`]; the other is the MCP
//! server, which the `mcp` subcommand starts.

use std::process::ExitCode;

use clap::Parser;
use dimetric_agent::cli::Cli;
use dimetric_agent::{output, run};

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(out) => {
            out.emit(json);
            ExitCode::SUCCESS
        }
        Err(diags) => {
            output::emit_error(&diags, json);
            ExitCode::FAILURE
        }
    }
}
