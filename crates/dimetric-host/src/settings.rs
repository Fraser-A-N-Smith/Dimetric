//! `project.toml`: the settings that change what a run *means*.
//!
//! Most configuration is a preference — a window size, a volume — and belongs
//! wherever the player last left it. What is in this file is the other kind:
//! the tick rate and the UI canvas are both part of the replay contract, so
//! two people running the same input log have to agree on them or they are not
//! playing the same game. Putting them in a file the project commits, rather
//! than in a preference somewhere on a machine, is what makes that agreement
//! automatic.
//!
//! Every setting has a default that works, so a project with no `project.toml`
//! runs. A file that exists but is wrong is a different matter: that is
//! reported, because a typo in a tick rate silently falling back to 60 is how
//! a project spends a week wondering why its recordings drift.

use std::path::Path;

use dimetric_core::{Code, Diagnostic, Diagnostics};
use dimetric_scene::ui::Canvas;

/// The file's name inside a project.
pub const SETTINGS_FILE: &str = "project.toml";

/// Ticks per second when a project does not say.
pub const DEFAULT_TICK_RATE: u32 = 60;

/// What a project declares about how it runs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Settings {
    /// Ticks per second. Part of the replay contract.
    pub tick_rate: u32,
    /// The virtual resolution the UI is laid out against.
    ///
    /// Also part of the contract, and less obviously so: layout decides what a
    /// click hits, so two players whose canvases differ disagree about which
    /// button was pressed. See `dimetric_scene::ui`.
    pub canvas: Canvas,
    /// The resolution the world is drawn at, before upscaling.
    ///
    /// Part of the replay contract, and not obviously so. A wider viewport
    /// shows *more world* at the same zoom, so unprojecting a canvas pixel to
    /// a world position depends on it — and since a script can pick a cell
    /// that way, two players whose resolutions differed would click different
    /// cells from the same pointer. See `dimetric_core::Projection`.
    pub resolution: (u32, u32),
    /// What each action is bound to, as `(action, keys)` in declaration order.
    ///
    /// Raw strings rather than a validated table, because the names of both
    /// the actions and the keys belong to crates this one sits below. What is
    /// checked here is the shape — a table of string arrays — and what the
    /// names mean is checked where they are understood.
    ///
    /// Empty means the project did not say, which is different from a project
    /// that bound nothing: the first gets the defaults and the second gets
    /// silence, and a player who deliberately unbinds everything should not
    /// have the engine argue.
    pub bindings: Vec<(String, Vec<String>)>,
    /// True when the project declared an `[input]` section at all.
    pub bindings_declared: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            tick_rate: DEFAULT_TICK_RATE,
            canvas: Canvas::default(),
            resolution: (480, 270),
            bindings: Vec::new(),
            bindings_declared: false,
        }
    }
}

impl Settings {
    /// Read `project.toml` from a project root.
    ///
    /// A missing file is not an error — it means every default — but an
    /// unreadable or malformed one is reported rather than silently ignored.
    pub fn load(root: &Path) -> (Settings, Diagnostics) {
        let path = root.join(SETTINGS_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Settings::parse(&text, &path.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Settings::default(), Diagnostics::new())
            }
            Err(e) => {
                let mut diagnostics = Diagnostics::new();
                diagnostics.push(Diagnostic::new(
                    Code::SETTINGS_UNREADABLE,
                    format!("{SETTINGS_FILE} could not be read: {e}"),
                ));
                (Settings::default(), diagnostics)
            }
        }
    }

    /// Parse settings from TOML text.
    pub fn parse(text: &str, origin: &str) -> (Settings, Diagnostics) {
        let mut out = Settings::default();
        let mut diagnostics = Diagnostics::new();

        let doc: toml_edit::DocumentMut = match text.parse() {
            Ok(doc) => doc,
            Err(e) => {
                diagnostics.push(Diagnostic::new(
                    Code::SETTINGS_UNREADABLE,
                    format!("{origin}: not valid TOML: {e}"),
                ));
                return (out, diagnostics);
            }
        };

        if let Some(sim) = doc.get("sim") {
            if let Some(rate) = sim.get("tick_rate") {
                match rate.as_integer() {
                    // An upper bound because a tick rate is a divisor and the
                    // whole engine measures time in ticks; a project that asks
                    // for a million of them per second has made a mistake it
                    // would rather hear about now.
                    Some(v) if (1..=1000).contains(&v) => out.tick_rate = v as u32,
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!("{origin}: sim.tick_rate must be a whole number from 1 to 1000"),
                    )),
                }
            }
        }

        if let Some(ui) = doc.get("ui") {
            if let Some(canvas) = ui.get("canvas") {
                match pair(canvas) {
                    Some((w, h)) if w > 0 && h > 0 => {
                        out.canvas = Canvas {
                            width: w,
                            height: h,
                        }
                    }
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!("{origin}: ui.canvas must be two positive whole numbers, as [width, height]"),
                    )),
                }
            }
        }

        if let Some(render) = doc.get("render") {
            if let Some(value) = render.get("resolution") {
                match pair(value) {
                    Some((w, h)) if w > 0 && h > 0 => out.resolution = (w as u32, h as u32),
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!(
                            "{origin}: render.resolution must be two positive whole numbers, \
                             as [width, height]"
                        ),
                    )),
                }
            }
        }

        if let Some(input) = doc.get("input") {
            match input.as_table_like() {
                Some(table) => {
                    out.bindings_declared = true;
                    for (action, item) in table.iter() {
                        match keys(item) {
                            Some(k) => out.bindings.push((action.to_string(), k)),
                            None => diagnostics.push(Diagnostic::new(
                                Code::SETTINGS_INVALID,
                                format!(
                                    "{origin}: input.{action} must be a list of key names, \
                                     such as [\"KeyW\", \"ArrowUp\"]"
                                ),
                            )),
                        }
                    }
                }
                None => diagnostics.push(Diagnostic::new(
                    Code::SETTINGS_INVALID,
                    format!("{origin}: [input] must be a table of action names"),
                )),
            }
        }

        (out, diagnostics)
    }
}

/// A list of key names.
fn keys(item: &toml_edit::Item) -> Option<Vec<String>> {
    let array = item.as_array()?;
    array
        .iter()
        .map(|v| v.as_str().map(str::to_string))
        .collect()
}

/// A `[w, h]` array of whole numbers.
fn pair(v: &toml_edit::Item) -> Option<(i32, i32)> {
    let array = v.as_array()?;
    let mut it = array.iter();
    let w = i32::try_from(it.next()?.as_integer()?).ok()?;
    let h = i32::try_from(it.next()?.as_integer()?).ok()?;
    // Exactly two: a three-element canvas is a mistake, not a depth.
    if it.next().is_some() {
        return None;
    }
    Some((w, h))
}
