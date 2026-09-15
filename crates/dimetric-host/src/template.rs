//! A project to start from.
//!
//! Small on purpose. A template that shows off everything is a template nobody
//! reads, and the first thing anyone does is delete most of it. This one is a
//! room, a thing you steer around it, a wall to bump into, and the three
//! commands that prove the run reproduces.
//!
//! The art is generated rather than checked in. Two flat squares are not worth
//! binary files in a repository, and a new project's first job is replacing
//! them anyway.

use std::path::{Path, PathBuf};

use dimetric_core::{Code, Diagnostic, Diagnostics};
use dimetric_scene::Color;

/// What a new project got.
pub struct Created {
    /// Where it was written.
    pub root: PathBuf,
    /// Files written, relative to the root, in order.
    pub files: Vec<String>,
}

/// Write a starting project into `root`.
///
/// Refuses a directory that already has files in it: overwriting somebody's
/// work because they typed the wrong path is not a thing to be casual about.
pub fn create(root: impl AsRef<Path>, name: &str) -> Result<Created, Diagnostics> {
    let root = root.as_ref().to_path_buf();
    if root.exists() {
        let occupied = std::fs::read_dir(&root)
            .map(|d| d.flatten().next().is_some())
            .unwrap_or(false);
        if occupied {
            return Err(one(Diagnostic::new(
                Code::COMMAND_REJECTED,
                format!("{} is not empty", root.display()),
            )
            .with_field("path", root.display().to_string())));
        }
    }

    let mut created = Created {
        root: root.clone(),
        files: Vec::new(),
    };

    // Written through the canonicaliser rather than by hand, so a new project
    // passes `dim scene fmt --check` on the day it is made and keeps doing so
    // when the canonical form changes.
    write(&root, "main.dim", &canonical(&scene(name))?, &mut created)?;
    write(&root, "scripts/player.lua", PLAYER_LUA, &mut created)?;
    write(&root, "README.md", &readme(name), &mut created)?;
    write(&root, ".gitignore", GITIGNORE, &mut created)?;

    // Two squares, so the scene draws something the moment it opens.
    png(
        &root,
        "assets/sprites/hero.png",
        16,
        Color::parse("#e8c48a").unwrap_or(Color::WHITE),
        &mut created,
    )?;
    png(
        &root,
        "assets/sprites/wall.png",
        16,
        Color::parse("#5a5f6b").unwrap_or(Color::WHITE),
        &mut created,
    )?;

    Ok(created)
}

/// The same scene, in canonical form.
fn canonical(text: &str) -> Result<String, Diagnostics> {
    let registry = dimetric_scene::KindRegistry::with_builtins();
    let out = dimetric_scene::parse(text, "main.dim", &registry);
    if out.diagnostics.has_errors() {
        return Err(out.diagnostics);
    }
    let doc = out.doc.expect("the template parses");
    let mut formatted = doc.doc.clone();
    dimetric_scene::write::format_in_place(&mut formatted, &doc.scene, &registry, None);
    Ok(formatted.to_string())
}

fn scene(name: &str) -> String {
    format!(
        r##"format = "dimetric"
version = 1

# The room. Nodes are a flat list, each naming its parent, and the tree is
# rebuilt on load — so moving a subtree is a one-line change.

[scene]
root = "n_root0000"

[[node]]
id = "n_root0000"
kind = "Node2D"
name = "{name}"

[[node]]
id = "n_player00"
kind = "Collider"
name = "Player"
parent = "n_root0000"
script = "script:scripts/player.lua"
pos = [0.0, 0.0]
collision_layer = 1
collision_mask = 1
radius = 7.0
shape = "Circle"

[[node]]
id = "n_plsprt00"
kind = "Sprite2D"
name = "Body"
parent = "n_player00"          # /{name}/Player
z = 5
texture = "asset:sprites/hero"

# The camera follows because it is a child of the thing that moves.
[[node]]
id = "n_camera00"
kind = "Camera2D"
name = "View"
parent = "n_player00"          # /{name}/Player
current = true

[[node]]
id = "n_wall0000"
kind = "Collider"
name = "Wall"
parent = "n_root0000"
pos = [64.0, 0.0]
collision_layer = 1
collision_mask = 1
is_static = true
shape = "AABB"
size = [16.0, 96.0]

[[node]]
id = "n_wlsprt00"
kind = "Sprite2D"
name = "Face"
parent = "n_wall0000"          # /{name}/Wall
z = 4
texture = "asset:sprites/wall"
"##
    )
}

const PLAYER_LUA: &str = r#"-- Something to steer.
--
-- `input` is the simulation's own input, not a device: inside a tick there is
-- no way to tell a keyboard from a replay log, which is what makes a recorded
-- run reproduce.

local SPEED = 90

function on_ready(self)
  self.steps = 0
end

function on_tick(self)
  local move = input.move()
  if move:length() > fx.new(0) then
    self:set_velocity(move * fx.new(SPEED))
    self.steps = (self.steps or 0) + 1
  else
    self:set_velocity(vec2(fx.new(0), fx.new(0)))
  end
end

function on_collision(self, other)
  -- Walls stop you; this is here to show where a reaction would go.
  self.bumped = (self.bumped or 0) + 1
end
"#;

const GITIGNORE: &str = "# Imported assets are derived from `assets/` and rebuilt on demand.\n.import/\n\n# Packaged builds.\nbuild/\n";

fn readme(name: &str) -> String {
    format!(
        r#"# {name}

A Dimetric project.

## Run it

```sh
dim-play .
```

Or without a window, which is how anything gets checked:

```sh
dim run --headless --ticks 120 --seed 1 --record run.hashes
dim replay --input <a log> --hashes run.hashes
```

`dim-play . --record run.input` writes what you did as an input log, so a run
you played back reproduces exactly — the same seed and the same log give the
same state, on any machine.

## What is here

- `main.dim` — the room. TOML, a flat node list, each node naming its parent.
- `scripts/player.lua` — what the player node does each tick.
- `assets/sprites/` — two placeholder squares. Replace them.

`dim api kinds --project .` lists every node kind and property you can use.
Declare your own in a `kinds.toml` here — `Enemy extends Collider` with
whatever numbers your game needs — and they validate and override like the
built-in ones.

## Ship it

```sh
cargo build -p dimetric-player --features gui --release
dim build --target linux --runtime <path to dim-play>
```
"#
    )
}

fn write(
    root: &Path,
    relative: &str,
    contents: &str,
    created: &mut Created,
) -> Result<(), Diagnostics> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| cannot(parent, e))?;
    }
    std::fs::write(&path, contents).map_err(|e| cannot(&path, e))?;
    created.files.push(relative.to_string());
    Ok(())
}

/// A flat square, so a new project draws something before anyone has made art.
fn png(
    root: &Path,
    relative: &str,
    size: u32,
    color: Color,
    created: &mut Created,
) -> Result<(), Diagnostics> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| cannot(parent, e))?;
    }
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            // A one-pixel darker border, so two of them side by side are
            // distinguishable and a sprite's extent is visible.
            let edge = x == 0 || y == 0 || x == size - 1 || y == size - 1;
            let shade = |c: u8| if edge { c / 2 } else { c };
            pixels.extend_from_slice(&[shade(color.r), shade(color.g), shade(color.b), 255]);
        }
    }
    dimetric_assets::encode_png(&path, &pixels, size, size)
        .map_err(|e| one(Diagnostic::new(Code::COMMAND_REJECTED, e.to_string())))?;
    created.files.push(relative.to_string());
    Ok(())
}

fn one(d: Diagnostic) -> Diagnostics {
    Diagnostics(vec![d])
}

fn cannot(path: &Path, e: std::io::Error) -> Diagnostics {
    one(Diagnostic::new(
        Code::COMMAND_REJECTED,
        format!("cannot write {}: {e}", path.display()),
    ))
}
