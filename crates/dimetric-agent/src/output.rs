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
}

impl Output {
    /// A result with no warnings.
    pub fn new(body: serde_json::Value, text: impl Into<String>) -> Output {
        Output {
            body,
            text: text.into(),
            warnings: Vec::new(),
        }
    }

    /// Print it.
    pub fn emit(&self, as_json: bool) {
        if as_json {
            let envelope = json!({
                "ok": true,
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
