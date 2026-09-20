//! Output formatting.
//!
//! Two renderings of the same result: a JSON object for programs, and terse
//! text for people. The JSON is the authoritative one — errors carry codes and
//! machine-readable fields, so a caller never has to parse prose to find out
//! what went wrong (I9).

use dimetric_core::{Diagnostic, Diagnostics};
use serde_json::json;

/// A successful result.
pub struct Output {
    /// Machine-readable body.
    pub body: serde_json::Value,
    /// Human-readable rendering.
    pub text: String,
    /// Non-fatal diagnostics raised along the way.
    pub warnings: Vec<Diagnostic>,
    /// Whether the command did everything it was asked to.
    ///
    /// A command can finish, exit zero and still not have done all of it —
    /// an import where one asset failed is the standing example. Saying `ok:
    /// true` there tells a caller that reads the envelope the opposite of what
    /// happened, and a caller that reads envelopes is the only kind this
    /// output exists for.
    pub ok: bool,
}

impl Output {
    /// A result with no warnings.
    pub fn new(body: serde_json::Value, text: impl Into<String>) -> Output {
        Output {
            body,
            text: text.into(),
            warnings: Vec::new(),
            ok: true,
        }
    }

    /// The command ran, and did not do all of it.
    ///
    /// The exit status stays zero: partial failure exits zero throughout this
    /// CLI, and changing that is a separate decision. What changes is that the
    /// envelope stops claiming success.
    pub fn partial(mut self) -> Output {
        self.ok = false;
        self
    }

    /// Print it.
    pub fn emit(&self, as_json: bool) {
        if as_json {
            let envelope = json!({
                "ok": self.ok,
                "result": self.body,
                "warnings": self.warnings,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&envelope).unwrap_or_default()
            );
        } else {
            if !self.text.is_empty() {
                println!("{}", self.text.trim_end());
            }
            for warning in &self.warnings {
                eprintln!("{warning}");
            }
        }
    }
}

/// Print a failure and its diagnostics.
pub fn emit_error(diagnostics: &Diagnostics, as_json: bool) {
    if as_json {
        let envelope = json!({
            "ok": false,
            "diagnostics": diagnostics,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&envelope).unwrap_or_default()
        );
    } else {
        for d in diagnostics.iter() {
            eprintln!("{d}");
        }
    }
}
