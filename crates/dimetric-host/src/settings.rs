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

/// How big a window a game wants to open.
///
/// The engine cannot answer this without a display, and `dimetric-host` has
/// none — so a project states its *intent* here and `dim-play` turns it into
/// pixels once it knows the monitor. [`WindowSize::resolve`] is that
/// arithmetic, which is why it is here and testable rather than in the
/// runtime's event loop.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WindowSize {
    /// The resolution the world is drawn at, scaled down to fit the monitor.
    ///
    /// The default, and the only one that is right without being told anything:
    /// a game's own resolution is the size at which its interface was laid out,
    /// so opening there means every pixel of it is a pixel on the screen.
    ///
    /// The old default was a fixed 1440×810, which for a 1920×1080 game was
    /// smaller than the game — and with the frame cropped rather than scaled,
    /// that is how a menu button came to be outside the window.
    #[default]
    Internal,
    /// The largest size at the game's shape that the monitor will hold.
    ///
    /// Up as well as down, for a game whose art is not tied to a pixel size and
    /// which would rather use the screen it is given.
    Monitor,
    /// Exactly this, in pixels.
    ///
    /// Not clamped. A project that names a number means it, and a window
    /// manager that cannot honour it will say so in its own way — whereas a
    /// number quietly changed here is a project being argued with.
    Fixed(u32, u32),
}

/// How much of a monitor the default window leaves alone.
///
/// A window opened at exactly the monitor's size has its title bar pushed off
/// the top, and there is no portable way to ask a window manager how much room
/// its decorations want — winit reports a monitor's full size and nothing else.
/// So the default leaves a tenth of it, which is enough for any title bar and a
/// taskbar, and is also what makes the arithmetic come out at the sizes a
/// person would have picked: a 480×270 game on a 1080p screen opens at 3× in a
/// 1440×810 window, which is exactly the fixed default this replaced.
///
/// A project that would rather use the whole screen says `fit = "monitor"`, and
/// one that wants a particular number says `size`. Neither is trimmed.
const COMFORTABLE: u32 = 90;

/// The largest rectangle with `shape`'s aspect ratio that fits in `within`.
///
/// Whole pixels, and at least one of each: a window of zero width is not a
/// window.
fn largest_fitting(shape: (u32, u32), within: (u32, u32)) -> (u32, u32) {
    let within = (within.0.max(1), within.1.max(1));
    // Integer arithmetic throughout. A float ratio here lands on 1079.999 for
    // an exact fit and costs a pixel row for nothing.
    let by_width = (within.0 as u64 * shape.1 as u64) / shape.0 as u64;
    match by_width <= within.1 as u64 {
        true => (within.0, (by_width as u32).max(1)),
        false => (
            ((within.1 as u64 * shape.0 as u64) / shape.1 as u64).max(1) as u32,
            within.1,
        ),
    }
}

/// How a game is placed in a window, which is nobody's business but the
/// runtime's.
///
/// Beside [`Game`] rather than among the contract's fields, and for the same
/// reason: a window size and a scaling mode reach a window manager and a
/// viewport call and nothing else. The simulation never sees either, so a
/// player who resizes a window cannot change what a recorded run replays to.
///
/// That line runs *through* `[render]`, which is worth saying out loud because
/// the section holds one key of each kind. `resolution` is the contract — a
/// script unprojects a click through it — and `integer_upscale` decides only
/// how the finished frame is laid into the output.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Presentation {
    /// `[window] size` or `[window] fit`.
    pub window: WindowSize,
    /// `[render] integer_upscale`. `None` leaves the engine's default, which is
    /// on, because a pixel-art project is the one that most needs it and the
    /// one least likely to know to ask.
    pub integer_upscale: Option<bool>,
    /// `[render] present_filter`. `None` leaves the engine's default, which is
    /// `auto`: nearest at a whole scale and linear at any other, because at any
    /// other nearest drops source rows and a dropped row in a letter is a
    /// missing stroke.
    pub present_filter: Option<dimetric_render::PresentFilter>,
    /// `[render] ambient`. `None` leaves the engine's default, opaque white,
    /// which is unlit — most projects have no lights at all.
    ///
    /// The project-wide floor under a scene's own. A `Camera2D` that names an
    /// `ambient` wins, because a region's light is a fact about the region;
    /// this is for the game whose light is the same everywhere, and for the
    /// default a scene that says nothing falls back to.
    pub ambient: Option<dimetric_scene::Color>,
}

impl Presentation {
    /// The window to open, given the game's own resolution and the monitor's.
    ///
    /// `monitor` is `None` when there is no display to ask — a platform that
    /// will not say — and then the game's own resolution is the best answer
    /// available. It is never *smaller* than the game, which is the property
    /// the fixed 1440×810 default broke.
    pub fn window_size(&self, internal: (u32, u32), monitor: Option<(u32, u32)>) -> (u32, u32) {
        let internal = (internal.0.max(1), internal.1.max(1));
        match self.window {
            // Said outright. Not clamped: a project that names a number means
            // it, and a number quietly changed here is a project being argued
            // with. A window bigger than the screen is the window manager's
            // business, and the frame inside it is never cropped either way.
            WindowSize::Fixed(w, h) => (w.max(1), h.max(1)),
            // Asked for the screen, so no allowance is kept back.
            WindowSize::Monitor => match monitor {
                Some(monitor) => largest_fitting(internal, monitor),
                None => internal,
            },
            WindowSize::Internal => {
                let Some(monitor) = monitor else {
                    return internal;
                };
                let room = (
                    (monitor.0 as u64 * COMFORTABLE as u64 / 100).max(1) as u32,
                    (monitor.1 as u64 * COMFORTABLE as u64 / 100).max(1) as u32,
                );
                // A pixel-locked game opens at a whole multiple, because that
                // is the only scale at which its pixels are square — and a
                // window that forces a fractional scale on launch would make
                // the setting look broken. One game, 480×270, on a 1080p
                // screen: 3×, which is 1440×810.
                let whole = internal.0 <= room.0
                    && internal.1 <= room.1
                    && self.integer_upscale.unwrap_or(true);
                if whole {
                    let times = (room.0 / internal.0).min(room.1 / internal.1).max(1);
                    return (internal.0 * times, internal.1 * times);
                }
                // Otherwise its own size, trimmed only when the screen will not
                // hold it. The frame then scales by the exact ratio and all of
                // it is visible, which is the whole point of this round.
                match internal.0 <= room.0 && internal.1 <= room.1 {
                    true => internal,
                    false => largest_fitting(internal, room),
                }
            }
        }
    }
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
    /// The window size and the scaling mode. Presentation, not contract — see
    /// [`Presentation`].
    pub presentation: Presentation,
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
            presentation: Presentation::default(),
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
            // In `[render]` because that is where a reader looks for a
            // rendering knob, and in `Presentation` because it is not part of
            // the contract. The section holds one key of each kind; see
            // [`Presentation`].
            if let Some(value) = render.get("integer_upscale") {
                match value.as_bool() {
                    Some(on) => out.presentation.integer_upscale = Some(on),
                    None => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!("{origin}: render.integer_upscale must be true or false"),
                    )),
                }
            }
            // Also presentation. It decides how the finished frame is sampled
            // into a window and reaches nothing the simulation can see.
            // Presentation too, and the one key in here a scene can override:
            // a `Camera2D`'s own `ambient` wins, because nine regions want
            // nine lights. This is the floor under all of them.
            if let Some(value) = render.get("ambient") {
                match value.as_str().map(dimetric_scene::Color::parse) {
                    Some(Ok(color)) => out.presentation.ambient = Some(color),
                    Some(Err(e)) => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!("{origin}: render.ambient: {e}"),
                    )),
                    None => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!(
                            "{origin}: render.ambient must be a colour written as a \
                             string, \"#rrggbbaa\" — \"#ffffffff\" is unlit, and \
                             anything darker dims the world and lets a Light2D show"
                        ),
                    )),
                }
            }
            if let Some(value) = render.get("present_filter") {
                match value.as_str() {
                    Some("auto") => {
                        out.presentation.present_filter = Some(dimetric_render::PresentFilter::Auto)
                    }
                    Some("nearest") => {
                        out.presentation.present_filter =
                            Some(dimetric_render::PresentFilter::Nearest)
                    }
                    Some("linear") => {
                        out.presentation.present_filter =
                            Some(dimetric_render::PresentFilter::Linear)
                    }
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!(
                            "{origin}: render.present_filter is \"auto\" — nearest at a \
                             whole scale and linear at any other — or \"nearest\" or \
                             \"linear\" to say outright"
                        ),
                    )),
                }
            }
        }

        if let Some(window) = doc.get("window") {
            // `size` and `fit` answer the same question, so declaring both is a
            // mistake worth reporting rather than a precedence rule to
            // remember.
            let size = window.get("size");
            let fit = window.get("fit");
            if size.is_some() && fit.is_some() {
                diagnostics.push(Diagnostic::new(
                    Code::SETTINGS_INVALID,
                    format!(
                        "{origin}: window.size and window.fit both say how big the window \
                         should be; declare one"
                    ),
                ));
            }
            if let Some(value) = size {
                match pair(value) {
                    Some((w, h)) if w > 0 && h > 0 => {
                        out.presentation.window = WindowSize::Fixed(w as u32, h as u32)
                    }
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!(
                            "{origin}: window.size must be two positive whole numbers, as \
                             [width, height]"
                        ),
                    )),
                }
            }
            if let Some(value) = fit {
                match value.as_str() {
                    Some("monitor") => out.presentation.window = WindowSize::Monitor,
                    Some("internal") => out.presentation.window = WindowSize::Internal,
                    _ => diagnostics.push(Diagnostic::new(
                        Code::SETTINGS_INVALID,
                        format!(
                            "{origin}: window.fit is \"monitor\" — the largest the screen \
                             holds at the game's shape — or \"internal\", the game's own \
                             resolution"
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
