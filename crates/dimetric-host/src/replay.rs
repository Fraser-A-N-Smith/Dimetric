//! The replay harness.
//!
//! Replay compares a state hash per tick and reports the **first** divergent
//! tick with a structured diff of what differs. "Replay failed" on its own is
//! useless for debugging; "state diverged at tick 4117, in
//! `/Arena01/Skeleton_03.pos`" is a bug report.

use std::fmt;

use dimetric_core::{Code, Diagnostic, Fx, NodeUid, StateHash, Tick};
use dimetric_scene::Value;
use dimetric_sim::{InputLog, ScriptHost, Sim, SimConfig, SimState};
use serde::{Deserialize, Serialize};

/// The outcome of a replay.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayReport {
    /// Ticks simulated.
    pub ticks: u64,
    /// Seed the run used.
    pub seed: u64,
    /// State hash after each tick.
    pub hashes: Vec<StateHash>,
    /// The first tick whose hash did not match, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub divergence: Option<Divergence>,
    /// Probe results, in file order.
    pub probes: Vec<ProbeResult>,
    /// Diagnostics raised during the run.
    pub diagnostics: dimetric_core::Diagnostics,
}

impl ReplayReport {
    /// True when nothing diverged and every probe held.
    pub fn passed(&self) -> bool {
        self.divergence.is_none()
            && self.probes.iter().all(|p| p.passed)
            && !self.diagnostics.has_errors()
    }

    /// The final hash, when there is one.
    pub fn final_hash(&self) -> Option<StateHash> {
        self.hashes.last().copied()
    }
}

/// Where and how a replay stopped matching.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Divergence {
    /// The first tick that did not match.
    pub tick: u64,
    /// Hash that was recorded.
    pub expected: StateHash,
    /// Hash this run produced.
    pub actual: StateHash,
    /// What differs, when both states are available to compare.
    pub differences: Vec<Difference>,
}

/// One differing field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Difference {
    /// Scene path, when the node still exists.
    pub path: String,
    /// Field name.
    pub field: String,
    /// What was expected.
    pub expected: String,
    /// What was found.
    pub actual: String,
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}: expected {}, found {}",
            self.path, self.field, self.expected, self.actual
        )
    }
}

/// Compare two states field by field.
///
/// Used to turn a hash mismatch into something a person can act on.
pub fn diff(expected: &SimState, actual: &SimState) -> Vec<Difference> {
    let mut out = Vec::new();
    if expected.tick != actual.tick {
        out.push(Difference {
            path: "/".into(),
            field: "tick".into(),
            expected: expected.tick.to_string(),
            actual: actual.tick.to_string(),
        });
    }

    for id in expected.scene.walk() {
        let Some(node) = expected.scene.get(id) else { continue };
        let path = expected.scene.path_of(id).unwrap_or_default();
        let Some(other_id) = actual.scene.by_uid(node.uid) else {
            out.push(Difference {
                path,
                field: "exists".into(),
                expected: "true".into(),
                actual: "false".into(),
            });
            continue;
        };
        let other = actual.scene.get(other_id).expect("resolved id exists");

        if node.transform.pos != other.transform.pos {
            out.push(Difference {
                path: path.clone(),
                field: "pos".into(),
                expected: node.transform.pos.to_string(),
                actual: other.transform.pos.to_string(),
            });
        }
        if node.transform.rot != other.transform.rot {
            out.push(Difference {
                path: path.clone(),
                field: "rot".into(),
                expected: node.transform.rot.to_degrees_string(),
                actual: other.transform.rot.to_degrees_string(),
            });
        }
        if node.name != other.name {
            out.push(Difference {
                path: path.clone(),
                field: "name".into(),
                expected: node.name.clone(),
                actual: other.name.clone(),
            });
        }
        for (key, value) in &node.props {
            match other.props.get(key) {
                Some(v) if v == value => {}
                Some(v) => out.push(Difference {
                    path: path.clone(),
                    field: key.clone(),
                    expected: value.to_string(),
                    actual: v.to_string(),
                }),
                None => out.push(Difference {
                    path: path.clone(),
                    field: key.clone(),
                    expected: value.to_string(),
                    actual: "<absent>".into(),
                }),
            }
        }
        let empty = indexmap::IndexMap::new();
        let vars = expected.vars.get(&node.uid).unwrap_or(&empty);
        let other_vars = actual.vars.get(&node.uid).unwrap_or(&empty);
        for (key, value) in vars {
            if other_vars.get(key) != Some(value) {
                out.push(Difference {
                    path: path.clone(),
                    field: format!("var:{key}"),
                    expected: value.to_string(),
                    actual: other_vars
                        .get(key)
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "<absent>".into()),
                });
            }
        }
        // Nodes the replay produced but the recording did not.
        if out.len() > 64 {
            break;
        }
    }
    out
}

/// One assertion evaluated during a replay.
///
/// Probes are how an agent verifies its own work: write one, run the replay,
/// read a structured pass or fail. They are deliberately a plain text file
/// rather than a Lua hook, so writing one cannot itself perturb the run.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Probe {
    /// Tick to evaluate at, after that tick has completed.
    pub tick: u64,
    /// Scene path of the node.
    pub path: String,
    /// Field: a property name, `pos.x`, `pos.y`, `rot`, `name`, `exists`, or
    /// `var:<name>` for a script variable.
    pub field: String,
    /// Comparison.
    pub op: Comparison,
    /// Right-hand side, as written.
    pub value: String,
}

/// A probe's comparison operator.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Comparison {
    /// `==`
    Equal,
    /// `!=`
    NotEqual,
    /// `<`
    Less,
    /// `<=`
    LessOrEqual,
    /// `>`
    Greater,
    /// `>=`
    GreaterOrEqual,
}

impl Comparison {
    fn parse(s: &str) -> Option<Comparison> {
        Some(match s {
            "==" => Comparison::Equal,
            "!=" => Comparison::NotEqual,
            "<" => Comparison::Less,
            "<=" => Comparison::LessOrEqual,
            ">" => Comparison::Greater,
            ">=" => Comparison::GreaterOrEqual,
            _ => return None,
        })
    }

    fn symbol(self) -> &'static str {
        match self {
            Comparison::Equal => "==",
            Comparison::NotEqual => "!=",
            Comparison::Less => "<",
            Comparison::LessOrEqual => "<=",
            Comparison::Greater => ">",
            Comparison::GreaterOrEqual => ">=",
        }
    }

    fn holds(self, ordering: std::cmp::Ordering) -> bool {
        use std::cmp::Ordering::*;
        match self {
            Comparison::Equal => ordering == Equal,
            Comparison::NotEqual => ordering != Equal,
            Comparison::Less => ordering == Less,
            Comparison::LessOrEqual => ordering != Greater,
            Comparison::Greater => ordering == Greater,
            Comparison::GreaterOrEqual => ordering != Less,
        }
    }
}

/// Whether a probe held.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProbeResult {
    /// The probe.
    pub probe: Probe,
    /// Whether it held.
    pub passed: bool,
    /// What was actually found.
    pub found: String,
}

impl fmt::Display for ProbeResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "tick {} {} {} {} {} -> {} ({})",
            self.probe.tick,
            self.probe.path,
            self.probe.field,
            self.probe.op.symbol(),
            self.probe.value,
            self.found,
            if self.passed { "pass" } else { "FAIL" }
        )
    }
}

/// Parse a probe file.
///
/// One probe per line: `tick <n> <path> <field> <op> <value>`. Blank lines and
/// `#` comments are ignored.
pub fn parse_probes(text: &str) -> Result<Vec<Probe>, Diagnostic> {
    let mut out = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        let bad = |what: &str| {
            Diagnostic::new(
                Code::PROBE_FAILED,
                format!("line {}: {what}", number + 1),
            )
            .with_field("line", (number + 1) as i64)
            .with_field("text", raw.to_string())
        };
        if parts.len() < 6 || parts[0] != "tick" {
            return Err(bad(
                "expected `tick <n> <path> <field> <op> <value>`",
            ));
        }
        let tick: u64 = parts[1].parse().map_err(|_| bad("tick is not a number"))?;
        let op = Comparison::parse(parts[4])
            .ok_or_else(|| bad("operator must be one of == != < <= > >="))?;
        out.push(Probe {
            tick,
            path: parts[2].to_string(),
            field: parts[3].to_string(),
            op,
            value: parts[5..].join(" "),
        });
    }
    Ok(out)
}

/// Read a field from a state, as text.
fn read_field(state: &SimState, path: &str, field: &str) -> Option<String> {
    let id = state.scene.resolve_path(path);
    if field == "exists" {
        return Some(id.is_some().to_string());
    }
    let id = id?;
    let node = state.scene.get(id)?;
    Some(match field {
        "pos.x" => node.transform.pos.x.to_exact_string(),
        "pos.y" => node.transform.pos.y.to_exact_string(),
        "rot" => node.transform.rot.to_degrees_string(),
        "name" => node.name.clone(),
        "kind" => node.kind.clone(),
        "visible" => node.visible.to_string(),
        "z" => node.z.to_string(),
        other => {
            let value = match other.strip_prefix("var:") {
                Some(key) => state.var(node.uid, key)?,
                None => node.get(other)?,
            };
            render_value(value)
        }
    })
}

fn render_value(value: &Value) -> String {
    match value {
        Value::Scalar(v) => v.to_exact_string(),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Str(s) | Value::Enum(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Compare a found value against a probe's expectation.
fn evaluate(probe: &Probe, found: &str) -> bool {
    // Numeric when both sides parse as scalars, textual otherwise. Comparing
    // "10.0" against "9.0" as strings would say 10 is smaller.
    if let (Ok(a), Ok(b)) = (Fx::parse_exact(found), Fx::parse_exact(&probe.value)) {
        return probe.op.holds(a.cmp(&b));
    }
    probe.op.holds(found.cmp(probe.value.as_str()))
}

/// Run a simulation for `ticks`, optionally checking against recorded hashes.
pub struct Replay<'a> {
    /// Input to drive the run.
    pub log: &'a InputLog,
    /// How many ticks to run. Defaults to the log's length.
    pub ticks: Option<u64>,
    /// Hashes to compare against, one per tick.
    pub expected: Option<&'a [StateHash]>,
    /// Assertions to evaluate.
    pub probes: &'a [Probe],
}

impl Replay<'_> {
    /// Run it.
    pub fn run(
        &self,
        scene: dimetric_scene::Scene,
        scripts: Box<dyn ScriptHost>,
        config: SimConfig,
    ) -> ReplayReport {
        let mut sim = Sim::new(scene, self.log.seed, scripts, config);
        let ticks = self.ticks.unwrap_or(self.log.frames.len() as u64);
        let mut hashes = Vec::with_capacity(ticks as usize);
        let mut divergence = None;
        let mut probes: Vec<ProbeResult> = Vec::new();

        for tick in 0..ticks {
            sim.step(self.log.frame(tick));
            let hash = sim.hash();
            hashes.push(hash);

            if let Some(expected) = self.expected {
                if divergence.is_none() {
                    if let Some(want) = expected.get(tick as usize) {
                        if *want != hash {
                            // Stop comparing after the first mismatch. Every
                            // later tick is downstream of this one, and listing
                            // them all buries the tick that actually matters.
                            divergence = Some(Divergence {
                                tick,
                                expected: *want,
                                actual: hash,
                                differences: Vec::new(),
                            });
                        }
                    }
                }
            }

            let state = sim.state();
            for probe in self.probes.iter().filter(|p| p.tick == tick) {
                let found = read_field(&state, &probe.path, &probe.field)
                    .unwrap_or_else(|| "<not found>".into());
                probes.push(ProbeResult {
                    passed: evaluate(probe, &found),
                    found,
                    probe: probe.clone(),
                });
            }
        }

        // Probes aimed past the end of the run never ran, which is a failure
        // rather than a silent pass.
        for probe in self.probes.iter().filter(|p| p.tick >= ticks) {
            probes.push(ProbeResult {
                probe: probe.clone(),
                passed: false,
                found: format!("<run ended at tick {ticks}>"),
            });
        }

        ReplayReport {
            ticks,
            seed: self.log.seed,
            hashes,
            divergence,
            probes,
            diagnostics: sim.take_diagnostics(),
        }
    }
}

/// A recorded run: the hashes a replay must reproduce.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HashLog {
    /// Seed the run used.
    pub seed: u64,
    /// One hash per tick.
    pub hashes: Vec<StateHash>,
}

impl HashLog {
    /// Render as text, one tick per line.
    pub fn to_text(&self) -> String {
        let mut out = format!("dimetric-hashes 1\nseed {}\n", self.seed);
        for (tick, hash) in self.hashes.iter().enumerate() {
            out.push_str(&format!("{tick} {hash}\n"));
        }
        out
    }

    /// Parse from text.
    pub fn parse(text: &str) -> Result<HashLog, Diagnostic> {
        let mut seed = 0u64;
        let mut hashes = Vec::new();
        for (number, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with("dimetric-hashes") {
                continue;
            }
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("seed") => {
                    seed = parts
                        .next()
                        .and_then(|s| s.parse().ok())
                        .ok_or_else(|| malformed(number, raw))?;
                }
                Some(tick) => {
                    let index: usize = tick.parse().map_err(|_| malformed(number, raw))?;
                    if index != hashes.len() {
                        return Err(malformed(number, raw));
                    }
                    let hash = parts
                        .next()
                        .and_then(StateHash::from_hex)
                        .ok_or_else(|| malformed(number, raw))?;
                    hashes.push(hash);
                }
                None => {}
            }
        }
        Ok(HashLog { seed, hashes })
    }
}

fn malformed(line: usize, text: &str) -> Diagnostic {
    Diagnostic::new(
        Code::LOG_MISMATCH,
        format!("hash log line {} is malformed: {text:?}", line + 1),
    )
}

/// Pretty-print a divergence for a terminal.
pub fn describe(divergence: &Divergence) -> String {
    let mut out = format!(
        "state diverged at tick {}\n  recorded {}\n  produced {}",
        divergence.tick, divergence.expected, divergence.actual
    );
    for d in &divergence.differences {
        out.push_str(&format!("\n  {d}"));
    }
    out
}

/// Convenience: the node ids present in a state, for tooling.
pub fn node_ids(state: &SimState) -> Vec<NodeUid> {
    state
        .scene
        .walk()
        .into_iter()
        .filter_map(|id| state.scene.get(id).map(|n| n.uid))
        .collect()
}

/// The tick a report reached.
pub fn last_tick(report: &ReplayReport) -> Tick {
    Tick(report.ticks)
}
