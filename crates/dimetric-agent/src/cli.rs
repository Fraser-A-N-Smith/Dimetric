//! Command-line surface.
//!
//! Every engine command is a subcommand, every output is JSON when asked for,
//! and every failure carries a `DIM####` code. That is not decoration: an agent
//! that has to parse prose to find out whether something worked will eventually
//! get it wrong.

use clap::{Parser, Subcommand};

/// The Dimetric command-line tool.
#[derive(Parser, Debug)]
#[command(name = "dim", version, about = "Dimetric engine tooling", long_about = None)]
pub struct Cli {
    /// Project directory. Defaults to the working directory.
    #[arg(long, global = true)]
    pub project: Option<String>,

    /// Scene to operate on, relative to the project root.
    #[arg(long, global = true)]
    pub scene: Option<String>,

    /// Emit JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Seed for generated node ids, so a scripted session is reproducible.
    #[arg(long, global = true, default_value_t = 0)]
    pub id_seed: u64,

    /// What to do.
    #[command(subcommand)]
    pub command: Top,
}

/// Top-level subcommands.
#[derive(Subcommand, Debug)]
pub enum Top {
    /// Inspect and format scenes.
    #[command(subcommand)]
    Scene(SceneCmd),
    /// Read and edit nodes.
    #[command(subcommand)]
    Node(NodeCmd),
    /// Instance prefabs and manage their overrides.
    #[command(subcommand)]
    Prefab(PrefabCmd),
    /// Set and clear instance overrides.
    #[command(subcommand)]
    Override(OverrideCmd),
    /// Connect and disconnect signals.
    #[command(subcommand)]
    Signal(SignalCmd),
    /// Write scripts.
    #[command(subcommand)]
    Script(ScriptCmd),
    /// Read and paint tiles.
    #[command(subcommand)]
    Tile(TileCmd),
    /// Import and list assets.
    #[command(subcommand)]
    Asset(AssetCmd),
    /// Run the simulation headlessly.
    Run(RunArgs),
    /// Dump simulation state.
    #[command(subcommand)]
    State(StateCmd),
    /// Capture a frame.
    #[command(subcommand)]
    Frame(FrameCmd),
    /// Replay a recorded run and check it.
    Replay(ReplayArgs),
    /// Package the project for a platform.
    Build(BuildArgs),
    /// Write a new project to start from.
    New(NewArgs),
    /// Print the engine's diagnostic codes, node kinds and command set.
    #[command(subcommand)]
    Api(ApiCmd),
    /// Serve the same commands as MCP tools over stdin and stdout.
    Mcp,
}

/// Scene-level operations.
#[derive(Subcommand, Debug)]
pub enum SceneCmd {
    /// Print the node tree.
    Tree,
    /// Look up one node by path.
    Query {
        /// Scene path, such as `/Arena01/Player`.
        path: String,
    },
    /// Rewrite the scene in canonical form.
    Fmt {
        /// Report whether the file is already canonical instead of writing it.
        #[arg(long)]
        check: bool,
    },
    /// Report the scene's validation diagnostics.
    Check,
    /// Resolve every prefab instance and print the runtime tree.
    Resolve,
}

/// Node operations.
#[derive(Subcommand, Debug)]
pub enum NodeCmd {
    /// Print one node's properties.
    Get {
        /// Scene path.
        path: String,
    },
    /// Add a node.
    Create {
        /// Registered node kind.
        #[arg(long)]
        kind: String,
        /// Name, unique among its siblings.
        #[arg(long)]
        name: String,
        /// Parent's scene path.
        #[arg(long)]
        parent: String,
        /// Node id to use. Generated when absent.
        #[arg(long)]
        id: Option<String>,
        /// Property assignments, as `key=value` with TOML value syntax.
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
    },
    /// Set one property.
    Set {
        /// Scene path.
        path: String,
        /// Property key.
        key: String,
        /// Value, in TOML syntax.
        value: String,
    },
    /// Clear one property, restoring its default.
    Clear {
        /// Scene path.
        path: String,
        /// Property key.
        key: String,
    },
    /// Move a node under a new parent.
    Reparent {
        /// Scene path of the node to move.
        path: String,
        /// Scene path of the new parent.
        parent: String,
    },
    /// Rename a node.
    Rename {
        /// Scene path.
        path: String,
        /// New name.
        name: String,
    },
    /// Remove a node and its subtree.
    Delete {
        /// Scene path.
        path: String,
    },
}

/// Prefab operations.
#[derive(Subcommand, Debug)]
pub enum PrefabCmd {
    /// Add an instance of another scene.
    Instance {
        /// Source scene, such as `prefabs/skeleton`.
        #[arg(long)]
        source: String,
        /// Parent's scene path.
        #[arg(long)]
        parent: String,
        /// Name for the instance.
        #[arg(long)]
        name: String,
        /// Position, as `x,y`.
        #[arg(long)]
        pos: Option<String>,
        /// Node id to use. Generated when absent.
        #[arg(long)]
        id: Option<String>,
    },
}

/// Override operations.
#[derive(Subcommand, Debug)]
pub enum OverrideCmd {
    /// Set one override.
    Set {
        /// Scene path of the instance node.
        #[arg(long)]
        instance: String,
        /// Node id in the source scene.
        #[arg(long)]
        target: String,
        /// Property key.
        #[arg(long)]
        key: String,
        /// Value, in TOML syntax.
        #[arg(long)]
        value: String,
    },
    /// Clear one override.
    Clear {
        /// Scene path of the instance node.
        #[arg(long)]
        instance: String,
        /// Node id in the source scene.
        #[arg(long)]
        target: String,
        /// Property key.
        #[arg(long)]
        key: String,
    },
    /// List the overrides on an instance.
    List {
        /// Scene path of the instance node.
        instance: String,
    },
}

/// Signal operations.
#[derive(Subcommand, Debug)]
pub enum SignalCmd {
    /// Connect a signal to a method.
    Connect {
        /// Emitter's scene path.
        #[arg(long)]
        from: String,
        /// Signal name.
        #[arg(long)]
        signal: String,
        /// Receiver's scene path.
        #[arg(long)]
        to: String,
        /// Method on the receiver's script.
        #[arg(long)]
        method: String,
    },
    /// Remove a connection.
    Disconnect {
        /// Emitter's scene path.
        #[arg(long)]
        from: String,
        /// Signal name.
        #[arg(long)]
        signal: String,
        /// Receiver's scene path.
        #[arg(long)]
        to: String,
        /// Method.
        #[arg(long)]
        method: String,
    },
    /// List the scene's connections.
    List,
}

/// Script operations.
#[derive(Subcommand, Debug)]
pub enum ScriptCmd {
    /// Write a script file, reporting syntax errors structurally.
    Write {
        /// Project-relative path, such as `scripts/enemy.lua`.
        path: String,
        /// Source text. Read from standard input when absent.
        #[arg(long)]
        source: Option<String>,
    },
    /// Check a script without writing it, or every script in the project when
    /// no path is given.
    Check {
        /// Also scan for things that may not reproduce on another machine.
        #[arg(long)]
        determinism: bool,
        /// Project-relative path. Absent means every script under `scripts/`.
        path: Option<String>,
    },
    /// List the project's scripts.
    List,
}

/// Tile operations.
#[derive(Subcommand, Debug)]
pub enum TileCmd {
    /// Fill a rectangle.
    Fill {
        /// Tile layer's scene path.
        #[arg(long)]
        layer: String,
        /// Region, as `x,y,width,height`.
        #[arg(long)]
        rect: String,
        /// Tile index.
        #[arg(long)]
        tile: u16,
    },
    /// Set one tile.
    Set {
        /// Tile layer's scene path.
        #[arg(long)]
        layer: String,
        /// Coordinate, as `x,y`.
        #[arg(long)]
        at: String,
        /// Tile index.
        #[arg(long)]
        tile: u16,
    },
    /// Read one tile.
    Get {
        /// Tile layer's scene path.
        #[arg(long)]
        layer: String,
        /// Coordinate, as `x,y`.
        #[arg(long)]
        at: String,
    },
    /// Import an LDtk level, baking it to native chunks.
    ///
    /// One way: the scene never references the `.ldtk` again. Reimporting the
    /// same level updates the layers it made rather than stacking new ones.
    ImportLdtk {
        /// Path to the `.ldtk` file, project-relative.
        path: String,
        /// Level to bake. Defaults to the first in the project.
        #[arg(long)]
        level: Option<String>,
        /// Node the layers hang off. Defaults to the scene root.
        #[arg(long)]
        into: Option<String>,
        /// Tileset asset for every layer, overriding what LDtk named.
        #[arg(long)]
        tileset: Option<String>,
        /// Report what would happen without changing anything.
        #[arg(long)]
        dry_run: bool,
    },
}

/// Asset operations.
#[derive(Subcommand, Debug)]
pub enum AssetCmd {
    /// List the project's source assets and their import state.
    List {
        /// Only list assets whose cache is behind their source.
        #[arg(long)]
        stale: bool,
    },
    /// Import one asset.
    Import {
        /// Project-relative path.
        path: String,
    },
    /// Import everything whose cache is behind its source.
    Reimport {
        /// Import every asset, not only the stale ones.
        #[arg(long)]
        all: bool,
    },
    /// Describe one asset: its id, its hash, and what it imported to.
    Info {
        /// Asset name, as a scene refers to it: `sprites/hero`.
        name: String,
    },
}

/// Headless run arguments.
#[derive(Parser, Debug)]
pub struct RunArgs {
    /// Run without a window. Always on, and accepted so that written-down
    /// commands keep working: `dim run` is the headless one, and `dim-play`
    /// is the window.
    #[arg(long, default_value_t = true)]
    pub headless: bool,
    /// Ticks to simulate. With an input log, defaults to the log's length.
    #[arg(long)]
    pub ticks: Option<u64>,
    /// Run seed.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Input log to drive the run.
    #[arg(long)]
    pub input: Option<String>,
    /// Write the per-tick state hashes here.
    #[arg(long)]
    pub record: Option<String>,
    /// Reload changed scripts and assets between ticks.
    ///
    /// Off by default, and refused while replaying: a run that picks up an
    /// edited script does not reproduce the recording it came from.
    #[arg(long)]
    pub watch: bool,
    /// Read and write the project's `profile.toml`.
    ///
    /// Off by default, because a headless run is usually a test and a test
    /// that spent somebody's Crowns would be a bad test. With it on, the run
    /// is a real session: it starts from the saved profile and writes back
    /// whatever it changed.
    ///
    /// `dim replay` has no such flag on purpose. A replay that read a profile
    /// would reproduce its recording only on the machine that made it, and
    /// one that wrote a profile could spend the Crowns of somebody who was
    /// only checking a bug.
    #[arg(long)]
    pub profile: bool,
}

/// State inspection.
#[derive(Subcommand, Debug)]
pub enum StateCmd {
    /// Run to a tick and dump the state there.
    Dump {
        /// Tick to stop at.
        #[arg(long, default_value_t = 0)]
        tick: u64,
        /// Run seed.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Input log to drive the run.
        #[arg(long)]
        input: Option<String>,
    },
    /// Print the state hash at a tick.
    Hash {
        /// Tick to stop at.
        #[arg(long, default_value_t = 0)]
        tick: u64,
        /// Run seed.
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
    /// Run to a tick and write the state out as a resumable save.
    Save {
        /// Directory to write into.
        #[arg(long)]
        out: String,
        /// Tick to stop at.
        #[arg(long, default_value_t = 0)]
        tick: u64,
        /// Run seed.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Input log to drive the run.
        #[arg(long)]
        input: Option<String>,
    },
    /// Read a save back and report what is in it.
    ///
    /// The loaded state becomes tick 0 of a fresh session, so this is also how
    /// a resumed run starts.
    Load {
        /// Directory to read.
        #[arg(long)]
        from: String,
    },
}

/// Frame capture.
#[derive(Subcommand, Debug)]
pub enum FrameCmd {
    /// Render one frame to a PNG.
    Capture {
        /// Tick to render.
        #[arg(long, default_value_t = 0)]
        tick: u64,
        /// Output path.
        #[arg(long)]
        png: String,
        /// Run seed.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// Input log to drive the run up to the tick.
        #[arg(long)]
        input: Option<String>,
        /// Output width in pixels.
        #[arg(long, default_value_t = 480)]
        width: u32,
        /// Output height in pixels.
        #[arg(long, default_value_t = 270)]
        height: u32,
        /// Override `[render] resolution` for this capture, as `WxH`. The
        /// project's own is used when this is absent, and overriding it
        /// warns, because picking still uses the project's.
        #[arg(long)]
        internal: Option<String>,
        /// Scale by whole numbers only. On by default, for pixel art.
        #[arg(long)]
        no_integer_upscale: bool,
        /// Ambient light as `#rrggbbaa`. Anything but white enables the light
        /// pass; white means lights would add nothing and it is skipped.
        #[arg(long)]
        ambient: Option<String>,
    },
}

/// Replay arguments.
#[derive(Parser, Debug)]
pub struct ReplayArgs {
    /// Input log to replay.
    #[arg(long)]
    pub input: String,
    /// Recorded hashes to compare against.
    #[arg(long)]
    pub hashes: Option<String>,
    /// Probe file to evaluate.
    #[arg(long = "assert")]
    pub assert: Option<String>,
    /// Ticks to run. Defaults to the log's length.
    #[arg(long)]
    pub ticks: Option<u64>,
}

/// Build arguments.
#[derive(Parser, Debug)]
pub struct BuildArgs {
    /// Target platform: `linux`, `macos`, `windows`, or a Rust target triple.
    #[arg(long)]
    pub target: String,
    /// Where to stage it. Defaults to `build/<target>` under the project.
    #[arg(long)]
    pub out: Option<String>,
    /// A `dim-play` built for this target, to ship beside the game.
    ///
    /// `dim` does not compile Rust; cargo does. Build one with
    /// `cargo build -p dimetric-player --features gui --release --target <triple>`
    /// and pass it here.
    #[arg(long)]
    pub runtime: Option<String>,
    /// Seed the packaged game starts from.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
}

/// Arguments for `dim new`.
#[derive(clap::Args, Debug)]
pub struct NewArgs {
    /// Directory to create. Must be empty or absent.
    pub path: String,
    /// Name for the root node and the README. Defaults to the directory's name.
    #[arg(long)]
    pub name: Option<String>,
}

/// Generated-documentation commands.
#[derive(Subcommand, Debug)]
pub enum ApiCmd {
    /// Print every diagnostic code.
    Codes,
    /// Print every registered node kind and its properties.
    Kinds,
    /// Print the command set.
    Commands,
    /// Print the JSON schema for the command set.
    Schema,
    /// Print the MCP tool list, which is this CLI seen from the other side.
    Tools,
    /// Print the Lua sandbox's globals.
    Globals,
    /// Print the keys every node has, which a kind may not reuse.
    Reserved,
    /// Print the shape of a node id and an asset id.
    Ids,
}
