//! Keys to input.
//!
//! The simulation reads a [`PlayerInput`]: a bitfield, a movement vector and an
//! aim angle. Everything a keyboard or a gamepad does has to become one of
//! those before it crosses into a tick, because inside a tick there is no such
//! thing as a key (I8).
//!
//! Bindings are names rather than key codes, so the mapping is a table a
//! project can print, diff and eventually configure, rather than a match arm.

use std::collections::BTreeSet;

use dimetric_core::{Angle, Code, Diagnostic, Fx, Vec2Fx};
use dimetric_sim::input::{buttons, Device};
use dimetric_sim::PlayerInput;

/// Something a player can be doing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Action {
    /// Move north.
    Up,
    /// Move south.
    Down,
    /// Move west.
    Left,
    /// Move east.
    Right,
    /// Primary action.
    Fire,
    /// Secondary action.
    Alt,
    /// Dash or dodge.
    Dash,
    /// Interact.
    Use,
    /// Pause. Read outside the simulation.
    Pause,
    /// An action the project declared of its own, by its position in the list.
    ///
    /// The index rather than the name so this stays `Copy` and cheap in a set,
    /// and because the index *is* the contract: it picks the bit, and a
    /// recording stores bits. See [`dimetric_sim::input::action_button`].
    Custom(usize),
}

impl Action {
    /// The button bit this action sets, if it is a button rather than a
    /// direction.
    pub fn button(self) -> Option<u32> {
        Some(match self {
            Action::Fire => buttons::FIRE,
            Action::Alt => buttons::ALT,
            Action::Dash => buttons::DASH,
            Action::Use => buttons::USE,
            Action::Pause => buttons::PAUSE,
            // `custom` returns `None` past the cap rather than shifting a bit
            // out of the field, and `Bindings::from_declared` never builds one
            // past it — this is the second lock on the same door.
            Action::Custom(index) => return buttons::custom(index),
            _ => return None,
        })
    }

    /// Every action, in the order they are documented.
    pub const ALL: &'static [Action] = &[
        Action::Up,
        Action::Down,
        Action::Left,
        Action::Right,
        Action::Fire,
        Action::Alt,
        Action::Dash,
        Action::Use,
        Action::Pause,
    ];

    /// The built-in action a binding file names, if it is one.
    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }

    /// The action a binding file names, built-in or declared by the project.
    ///
    /// `declared` is the project's own actions in declaration order. A built-in
    /// wins, which is why declaring one of their names is refused: a key bound
    /// to `fire` must set the engine's bit whatever a project wrote.
    pub fn resolve(name: &str, declared: &[String]) -> Option<Action> {
        Action::from_name(name).or_else(|| {
            declared
                .iter()
                .position(|a| a == name)
                .filter(|index| *index < buttons::MAX_CUSTOM)
                .map(Action::Custom)
        })
    }

    /// The name used in bindings and in diagnostics.
    ///
    /// A declared action's own name is not here — it belongs to the project,
    /// not to this enum. [`Action::label`] is the one to print.
    pub fn name(self) -> &'static str {
        match self {
            Action::Up => "up",
            Action::Down => "down",
            Action::Left => "left",
            Action::Right => "right",
            Action::Fire => "fire",
            Action::Alt => "alt",
            Action::Dash => "dash",
            Action::Use => "use",
            Action::Pause => "pause",
            Action::Custom(_) => "declared",
        }
    }

    /// What to call this action in a message, given the project's own list.
    pub fn label(self, declared: &[String]) -> String {
        match self {
            Action::Custom(index) => declared
                .get(index)
                .cloned()
                .unwrap_or_else(|| format!("declared action {index}")),
            other => other.name().to_string(),
        }
    }
}

/// Every pad button a binding may name, in a fixed order.
///
/// Here rather than in [`crate::pad`] on purpose: the names belong to the
/// binding vocabulary, and a build without the `pad` feature has to validate a
/// `project.toml` and print the same documentation as one with it. Only the
/// mapping to the gamepad library's own enum is feature-gated.
///
/// Spelt `Pad` plus the shoulder and trigger abbreviations people actually use,
/// because a Controls page shows these strings and `PadLeftTrigger2` is not
/// what anybody calls it.
pub const PAD_BUTTONS: &[&str] = &[
    "PadSouth",
    "PadEast",
    "PadNorth",
    "PadWest",
    "PadLB",
    "PadRB",
    "PadLT",
    "PadRT",
    "PadSelect",
    "PadStart",
    "PadMode",
    "PadLeftThumb",
    "PadRightThumb",
    "PadUp",
    "PadDown",
    "PadLeft",
    "PadRight",
];

/// Whether a binding name is a pad button rather than a key.
///
/// Used only for messages: the bindings table does not care which a name is,
/// which is the point — one table, keys and buttons together, so a Controls
/// page is one list and a pad is as bindable as a keyboard.
pub fn is_pad_button(name: &str) -> bool {
    PAD_BUTTONS.contains(&name)
}

/// Snap an analogue stick to something a log can hold exactly.
///
/// A stick reports floats, and a float written into simulation state is the
/// easiest way to break a replay (I3). Worse, the raw value jitters: a stick
/// at rest sends slightly different numbers every poll, and every one of them
/// would be a different line in the log.
///
/// So the magnitude is rounded to one of [`STICK_STEPS`] steps and the value
/// rebuilt from the engine's own trig tables. What comes out is exactly
/// representable, identical on every machine, and stable while the player
/// holds still. Below the dead zone it is exactly zero, which is what stops a
/// resting stick writing input for ever.
pub fn quantize_stick(x: f32, y: f32) -> Vec2Fx {
    // I3-exempt: this is the device boundary, and quantising is the whole
    // point of the function. Nothing downstream of it sees a float.
    let magnitude = (x * x + y * y).sqrt();
    if magnitude < STICK_DEAD_ZONE {
        return Vec2Fx::ZERO;
    }
    let clamped = magnitude.min(1.0);
    let step = ((clamped * STICK_STEPS as f32).round() as i32).clamp(1, STICK_STEPS);
    let scale = Fx::from_int(step) / Fx::from_int(STICK_STEPS);

    // The direction goes through the engine's own fixed-point atan2 rather
    // than the platform's, for the same reason everything else does: libm
    // implementations do not agree with each other.
    let angle = Angle::from_vector(Fx::from_f64_lossy(x as f64), Fx::from_f64_lossy(y as f64));
    let (sin, cos) = angle.sin_cos();
    Vec2Fx::new(cos * scale, sin * scale)
}

/// How many magnitude steps a stick is rounded to.
///
/// Sixteen is finer than a player can feel and coarse enough that holding a
/// stick still produces one repeated value rather than a stream of them.
pub const STICK_STEPS: i32 = 16;

/// Below this, a stick reads as centred.
pub const STICK_DEAD_ZONE: f32 = 0.2;

/// Which key does what.
#[derive(Clone, Debug)]
pub struct Bindings {
    pairs: Vec<(String, Action)>,
}

impl Default for Bindings {
    fn default() -> Bindings {
        Bindings::wasd()
    }
}

impl Bindings {
    /// WASD and the arrow keys to move, mouse and space to act.
    ///
    /// Both movement sets at once, because the first thing anyone does with a
    /// new game is try whichever they prefer, and a game that ignores one of
    /// them reads as broken rather than opinionated.
    pub fn wasd() -> Bindings {
        Bindings {
            pairs: vec![
                ("KeyW".into(), Action::Up),
                ("KeyS".into(), Action::Down),
                ("KeyA".into(), Action::Left),
                ("KeyD".into(), Action::Right),
                ("ArrowUp".into(), Action::Up),
                ("ArrowDown".into(), Action::Down),
                ("ArrowLeft".into(), Action::Left),
                ("ArrowRight".into(), Action::Right),
                ("Space".into(), Action::Fire),
                ("Mouse0".into(), Action::Fire),
                ("Mouse1".into(), Action::Alt),
                ("ShiftLeft".into(), Action::Dash),
                ("KeyE".into(), Action::Use),
                ("Escape".into(), Action::Pause),
                // The pad, in the same table as the keys.
                //
                // This used to be a constant in `pad.rs` that nothing could
                // reach: South *was* fire, and a player who wanted confirm and
                // cancel the other way round — which is the other half of the
                // world's convention — had no way to say so. One table means a
                // Controls page is one list and `input.bind` rebinds a button
                // exactly as it rebinds a key.
                ("PadSouth".into(), Action::Fire),
                ("PadRB".into(), Action::Fire),
                ("PadWest".into(), Action::Alt),
                ("PadEast".into(), Action::Dash),
                ("PadNorth".into(), Action::Use),
                ("PadStart".into(), Action::Pause),
                ("PadUp".into(), Action::Up),
                ("PadDown".into(), Action::Down),
                ("PadLeft".into(), Action::Left),
                ("PadRight".into(), Action::Right),
            ],
        }
    }

    /// Bindings a project declared, or the defaults when it declared none.
    ///
    /// An unknown action name is an error rather than a warning, and rather
    /// than being quietly dropped: a player who mistyped `fier` and got no
    /// error would conclude the engine's binding system does not work, which
    /// is worse than being told about the typo.
    ///
    /// Key names are *not* validated. The set of them belongs to whatever
    /// window library is underneath, it differs by platform, and refusing a key
    /// this build has never heard of would make a binding file unportable for
    /// no benefit — an unrecognised key simply never fires.
    pub fn from_declared(
        declared: &[(String, Vec<String>)],
        declared_any: bool,
    ) -> (Bindings, Vec<Diagnostic>) {
        Bindings::from_declared_with(declared, declared_any, &[])
    }

    /// The same, with the project's own declared action names.
    ///
    /// `actions` is `[input] actions` in declaration order. A name in it is
    /// bindable like any built-in, which is the point: a game's undo should be
    /// a key and a pad button, not only a button on the screen.
    ///
    /// An unknown name is still an error, and that is why declaring has to be
    /// explicit. If an unrecognised action simply became a new one, `fier =
    /// ["Space"]` would silently be a verb nothing reads, which is the typo
    /// this diagnostic exists to catch.
    pub fn from_declared_with(
        declared: &[(String, Vec<String>)],
        declared_any: bool,
        actions: &[String],
    ) -> (Bindings, Vec<Diagnostic>) {
        if !declared_any {
            return (Bindings::wasd(), Vec::new());
        }
        let mut pairs = Vec::new();
        let mut problems = Vec::new();
        for (action, keys) in declared {
            match Action::resolve(action, actions) {
                Some(action) => {
                    for key in keys {
                        pairs.push((key.clone(), action));
                    }
                }
                None => problems.push(
                    Diagnostic::new(
                        Code::BINDING_UNKNOWN,
                        format!(
                            "no action called `{action}`. The engine has: {}{}. Declare \
                             one of your own under `[input] actions` to bind it.",
                            Action::ALL
                                .iter()
                                .map(|a| a.name())
                                .collect::<Vec<_>>()
                                .join(", "),
                            match actions.is_empty() {
                                true => String::new(),
                                false =>
                                    format!("; this project also declares {}", actions.join(", ")),
                            }
                        ),
                    )
                    .with_field("action", action.clone()),
                ),
            }
        }
        // A project that declares `[input]` replaces the defaults, and that
        // used to mean only the keys, because the pad was a separate constant
        // nothing could reach. Now the pad is in this table — so a project
        // that says nothing about it would silently have no pad at all, which
        // is a worse game than the one it had before this change.
        //
        // Four cases, and each says what it means:
        //
        //   * no `[input]` at all — the engine's defaults, keys and pad;
        //   * `[input]` with nothing under it — nothing bound, which is a
        //     choice a project is allowed to make, and is why this is guarded
        //     on the table being non-empty rather than on the pad alone;
        //   * `[input]` with keys and no pad button — those keys, and the
        //     engine's pad, so a controller does not vanish from every project
        //     that has not written a pad layout yet;
        //   * `[input]` naming any pad button — exactly what it says, because
        //     a project with an opinion about the pad owns all of it.
        // One key, two actions. `action` answers with the first match, so the
        // second binding would be a line in a project file that does nothing —
        // which is the same class of mistake as a mistyped action name, and
        // deserves the same treatment.
        for (index, (key, action)) in pairs.iter().enumerate() {
            if let Some((_, first)) = pairs[..index].iter().find(|(k, _)| k == key) {
                problems.push(
                    Diagnostic::new(
                        Code::BINDING_UNKNOWN,
                        format!(
                            "`{key}` is bound to both `{}` and `{}`; the first wins, so \
                             the second does nothing. One key does one thing.",
                            first.label(actions),
                            action.label(actions)
                        ),
                    )
                    .with_severity(dimetric_core::Severity::Warning)
                    .with_field("key", key.clone()),
                );
            }
        }
        if !pairs.is_empty() && !pairs.iter().any(|(key, _)| is_pad_button(key)) {
            pairs.extend(
                Bindings::wasd()
                    .pairs
                    .into_iter()
                    .filter(|(key, _)| is_pad_button(key)),
            );
        }
        (Bindings { pairs }, problems)
    }

    /// What a key does, if anything.
    pub fn action(&self, key: &str) -> Option<Action> {
        self.pairs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, action)| *action)
    }

    /// Every binding, in declaration order.
    pub fn pairs(&self) -> impl Iterator<Item = (&str, Action)> {
        self.pairs.iter().map(|(k, a)| (k.as_str(), *a))
    }

    /// Every key and button bound to an action, in order.
    ///
    /// What a Controls page shows for a row: the bindings a player would see
    /// beside "Confirm".
    pub fn keys_for(&self, action: Action) -> Vec<&str> {
        self.pairs
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Replace everything bound to an action.
    ///
    /// The whole set rather than one key, because that is what a Controls page
    /// knows: a player picking keys for "Confirm" has decided the list, and
    /// adding to it would make a page that showed two keys and meant three.
    ///
    /// Keys are not validated, for the reason `from_declared` gives: the set of
    /// names belongs to whatever window library is underneath and differs by
    /// platform, so refusing one this build has never heard of would make a
    /// saved Controls page unportable for no benefit. A name nothing produces
    /// simply never fires.
    ///
    /// **Nothing here reaches the simulation.** A recording stores actions, so
    /// a session played under any bindings replays identically under any other
    /// — which is exactly why remapping cannot be done in script. A script
    /// reading `fire` and deciding it meant `alt` would put the player's
    /// preference into the simulation's reading of the input, and the same
    /// recording would replay differently under another profile.
    /// A key assigned here is **taken** from whatever else held it. That is
    /// what a Controls page does and what a player expects: putting F on
    /// Confirm means F confirms, not that F confirms unless something earlier
    /// in a table claims it. [`Bindings::action`] answers with the first
    /// match, so leaving a duplicate behind would be a row that showed F and
    /// did nothing.
    pub fn rebind(&mut self, action: Action, keys: Vec<String>) {
        self.pairs
            .retain(|(key, a)| *a != action && !keys.contains(key));
        for key in keys {
            self.pairs.push((key, action));
        }
    }
}

/// The actions currently held.
#[derive(Clone, Debug, Default)]
pub struct Held {
    actions: BTreeSet<Action>,
    aim: Angle,
    pointer: Vec2Fx,
    /// A gamepad stick, when one is being pushed.
    ///
    /// Separate from the key-derived direction rather than folded into it: a
    /// stick is analogue and keys are not, and adding them would let a player
    /// holding both walk at twice the speed.
    stick: Option<Vec2Fx>,
    /// Which kind of device last did something.
    ///
    /// For prompts that match the hand on the device: "Space" for a keyboard,
    /// the South button for a pad. It goes into the input frame, so what a
    /// game draws from it is reproduced by a recording — see
    /// [`dimetric_sim::input::Device`].
    device: Device,
}

impl Held {
    /// Nothing held, aiming east.
    pub fn new() -> Held {
        Held::default()
    }

    /// Record a press or a release, from a device.
    ///
    /// A release counts: letting go of a key is the keyboard doing something,
    /// and a player who taps a key should see keyboard prompts while the key
    /// is up as well as down.
    pub fn set_from(&mut self, action: Action, down: bool, device: Device) {
        self.device = device;
        self.set(action, down);
    }

    /// Record a press or a release, leaving the device as it was.
    pub fn set(&mut self, action: Action, down: bool) {
        if down {
            self.actions.insert(action);
        } else {
            self.actions.remove(&action);
        }
    }

    /// Which kind of device last did something.
    pub fn device(&self) -> Device {
        self.device
    }

    /// Note that a device did something no action came of.
    ///
    /// A mouse move, a wheel, a stick going back to centre: nothing an action
    /// is bound to, and still the player's hand moving from one device to
    /// another. Without this a player who puts the keyboard down and picks up
    /// a pad would keep seeing key prompts until they pressed a bound button.
    pub fn touched(&mut self, device: Device) {
        self.device = device;
    }

    /// True when an action is held.
    pub fn holds(&self, action: Action) -> bool {
        self.actions.contains(&action)
    }

    /// Point the aim somewhere.
    pub fn aim_at(&mut self, aim: Angle) {
        self.aim = aim;
    }

    /// Put the pointer at a canvas pixel.
    ///
    /// Whole pixels, and the caller does the conversion from window
    /// coordinates: the window is the one thing that must not reach a tick,
    /// so the boundary is where its size gets divided out.
    ///
    /// Moving the pointer is the mouse doing something, so it counts — but
    /// only when it actually moves. A window that reports the same position
    /// every frame would otherwise hold the device on `mouse` for ever and a
    /// pad player would never see a pad prompt.
    pub fn point_at(&mut self, canvas_pixel: Vec2Fx) {
        if canvas_pixel != self.pointer {
            self.device = Device::Mouse;
        }
        self.pointer = canvas_pixel;
    }

    /// Where the pointer is.
    pub fn pointer(&self) -> Vec2Fx {
        self.pointer
    }

    /// Push the movement stick, or let it go with `None`.
    ///
    /// The value is quantised by [`quantize_stick`] before it gets here.
    ///
    /// A stick past its dead zone is a pad in somebody's hands. Letting it go
    /// is too, so long as it was pushed: a pad polled every frame reports a
    /// centred stick for ever, and treating that as activity would pin the
    /// device on `pad` and no keyboard prompt would ever come back.
    pub fn push_stick(&mut self, stick: Option<Vec2Fx>) {
        let pushed = stick.is_some_and(|s| !s.is_zero());
        let was_pushed = self.stick.is_some_and(|s| !s.is_zero());
        if pushed || was_pushed {
            self.device = Device::Pad;
        }
        self.stick = stick;
    }

    /// Release everything. Used when the window loses focus, so a key held at
    /// the moment someone alt-tabs does not stay held forever.
    pub fn release_all(&mut self) {
        self.actions.clear();
    }

    /// The input a tick reads.
    ///
    /// Diagonals are normalised, so walking north-east is not faster than
    /// walking north. `Vec2Fx::normalized` is fixed point, so the value is the
    /// same on every machine and writes exactly into an input log.
    pub fn player_input(&self) -> PlayerInput {
        let mut buttons = 0;
        for action in &self.actions {
            if let Some(bit) = action.button() {
                buttons |= bit;
            }
        }
        let x = i32::from(self.holds(Action::Right)) - i32::from(self.holds(Action::Left));
        // North is negative y, as it is everywhere else in the engine.
        let y = i32::from(self.holds(Action::Down)) - i32::from(self.holds(Action::Up));
        let raw = Vec2Fx::new(Fx::from_int(x), Fx::from_int(y));
        let keys = if raw.is_zero() { raw } else { raw.normalized() };

        // A pushed stick wins over the keys rather than adding to them: a
        // player resting a hand on both should not move at twice the speed,
        // and whichever device they are actually using is the one that is
        // moving.
        let move_dir = match self.stick {
            Some(stick) if !stick.is_zero() => stick,
            _ => keys,
        };

        PlayerInput {
            buttons,
            move_dir,
            aim: self.aim,
            pointer: self.pointer,
            device: self.device,
        }
    }
}

/// The event kind a game rebinds an action with.
pub const BIND: &str = "input.bind";

/// Apply an `input.bind` event, if that is what it is.
///
/// Returns whether the event was this kind — read rather than consumed, so
/// another consumer of the same drained list still finds its own kinds. The
/// same shape `Speaker::apply_event` has for `audio.bus_volume`, and for the
/// same reason.
///
/// ```text
/// event.emit("input.bind", { action = "fire", keys = { "Space", "Enter" } })
/// ```
///
/// # Why this is the host's and not the script's
///
/// Remapping in script — reading `fire` and deciding it meant `alt` — would put
/// the player's preference into the simulation's *reading* of the input, so the
/// same recording would replay differently under another profile. Bindings
/// belong here, before the input frame is built: a recording stores actions, so
/// a session played under any bindings replays identically under any other.
///
/// A game emits its saved bindings on launch and on every change, which is why
/// this replaces an action's whole set rather than adding to it — a Controls
/// page knows the list, and adding would make a page that showed two keys and
/// meant three.
pub fn apply_event(
    bindings: &mut Bindings,
    actions: &[String],
    event: &dimetric_sim::event::GameEvent,
) -> Option<Diagnostic> {
    if event.kind != BIND {
        return None;
    }
    let refused = |why: String| {
        Some(
            Diagnostic::new(
                Code::BINDING_UNKNOWN,
                format!("{BIND}: {why}; the bindings are unchanged"),
            )
            .with_severity(dimetric_core::Severity::Warning)
            .with_field("kind", BIND.to_string()),
        )
    };
    let dimetric_scene::Value::Map(payload) = &event.payload else {
        return refused("the payload is not a table".to_string());
    };
    let Some(name) = payload
        .get("action")
        .and_then(dimetric_scene::Value::as_str)
    else {
        return refused("no `action` name in the payload".to_string());
    };
    let Some(action) = Action::resolve(name, actions) else {
        return refused(format!(
            "no action called `{name}`. The engine has: {}{}",
            Action::ALL
                .iter()
                .map(|a| a.name())
                .collect::<Vec<_>>()
                .join(", "),
            match actions.is_empty() {
                true => String::new(),
                false => format!("; this project also declares {}", actions.join(", ")),
            }
        ));
    };
    let Some(dimetric_scene::Value::List(keys)) = payload.get("keys") else {
        return refused(
            "no `keys` list in the payload; pass every key for this action,              because this replaces the set rather than adding to it"
                .to_string(),
        );
    };
    let mut names = Vec::with_capacity(keys.len());
    for key in keys {
        match key.as_str() {
            Some(name) => names.push(name.to_string()),
            None => return refused("a `keys` entry is not a string".to_string()),
        }
    }
    bindings.rebind(action, names);
    None
}

/// The key the runtime freezes its own simulation on.
///
/// Not a bindable action, deliberately. Freezing is a debugging affordance of
/// whatever is running the game, and `pause` is the key a *game* wants for a
/// menu of its own — so the two cannot be the same thing. They were: the
/// runtime toggled its freeze on the `pause` action and returned before the
/// press reached the input frame, so `input.pressed("pause")` was never true in
/// Lua and no game could build a pause screen. And a project cannot take this
/// key away, because it is not in the bindings table.
pub const FREEZE_KEY: &str = "Pause";

/// What a key press means to the runtime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyEffect {
    /// Delivered to the input frame, where a script can read it.
    Action(Action),
    /// The runtime's own freeze.
    ToggleFreeze,
    /// Bound to nothing. Worth saying out loud when it came from `--keys`.
    Unbound,
}

/// Decide what a key press does, before anything is done about it.
///
/// Here rather than in the window's event handler so that every path which
/// takes a key — the window, and a scripted `--capture` run — agrees on what a
/// key means. The capture path used to resolve keys itself under a comment
/// claiming it went "through the same `press` the window uses"; it did not, so
/// a key the window handled differently would have photographed correctly and
/// played wrongly, which is exactly what `pause` did.
pub fn key_effect(bindings: &Bindings, key: &str) -> KeyEffect {
    if key == FREEZE_KEY {
        return KeyEffect::ToggleFreeze;
    }
    match bindings.action(key) {
        Some(action) => KeyEffect::Action(action),
        None => KeyEffect::Unbound,
    }
}
