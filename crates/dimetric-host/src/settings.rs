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

/// What a game calls itself, and what it looks like in a taskbar.
///
/// A separate type, and not fields on [`Settings`] beside the tick rate,
/// because everything else in this file is the replay contract and none of this
/// is. A window's title and its icon are host presentation in the same sense
/// audio is: they reach a window manager and nothing else. No part of the
/// engine below the runtime reads them, the simulation never sees them, and
/// renaming a game cannot change what a recorded run replays to.
///
/// They live in `project.toml` anyway because that is the file a project
/// already has, and a second one holding two strings is a second one to
/// forget.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Game {
    /// What the window and the taskbar call it. `None` means the engine's own
    /// name, which is what an unnamed project used to get with no way to say
    /// otherwise.
    pub name: Option<String>,
    /// A PNG for the window and the taskbar, relative to the project root.
    ///
    /// Decoded where the window is created, from whatever the project is read
    /// through — so a game folded into one file finds its icon inside itself.
    pub icon: Option<String>,
}

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
    /// The name and the icon. Presentation, not contract — see [`Game`].
    pub game: Game,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            tick_rate: DEFAULT_TICK_RATE,
            canvas: Canvas::default(),
            resolution: (480, 270),
            bindings: Vec::new(),
            bindings_declared: false,
            game: Game::default(),
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

    /// The same, over anything the project's files can be read from.
    ///
    /// `root` is only for the path a diagnostic names; the bytes come from the
    /// source, which for a single-file game is an archive appended to the
    /// runtime. Settings are where the replay contract lives, so a shipped game
    /// reading its own is not optional.
    pub fn read_from(source: &dyn dimetric_core::Source, root: &Path) -> (Settings, Diagnostics) {
        let origin = root.join(SETTINGS_FILE).display().to_string();
        if !source.exists(SETTINGS_FILE) {
            return (Settings::default(), Diagnostics::new());
        }
        match dimetric_core::source::read_to_string(source, SETTINGS_FILE) {
            Ok(text) => Settings::parse(&text, &origin),
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

        if let Some(game) = doc.get("game") {
            // A name and an icon, each reported rather than ignored when it is
            // the wrong shape: a game that silently shows "Dimetric" because
            // `name` was given as a number is a bug nobody can see.
            match game.get("name") {
                None => {}
                Some(item) => match item.as_str() {
                    Some(name) if !name.trim().is_empty() => out.game.name = Some(name.to_string()),
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!("{origin}: game.name must be a non-empty string"),
                    )),
                },
            }
            match game.get("icon") {
                None => {}
                Some(item) => match item.as_str() {
                    Some(icon) if !icon.trim().is_empty() => out.game.icon = Some(icon.to_string()),
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!(
                            "{origin}: game.icon must be a path to a PNG, relative to the \
                             project root"
                        ),
                    )),
                },
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
