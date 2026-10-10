//! Staging a project into something you can hand to somebody.
//!
//! A packaged game is the project's own files beside a runtime that knows how
//! to open them, plus a small manifest saying which scene to boot. There is no
//! archive step and no bundling into the executable: the files a game ships are
//! the files it was developed against, byte for byte, which is what makes "it
//! worked on my machine" checkable rather than an opinion.
//!
//! The runtime is not built here. `dim` does not compile Rust; cargo does, and
//! pretending otherwise would put a toolchain dependency in the middle of the
//! engine. What this module does is decide what goes in and copy it.

use std::path::{Path, PathBuf};

use dimetric_core::{Code, Diagnostic, Diagnostics};

use crate::project::Project;

/// Name of the manifest a staged game carries.
pub const MANIFEST: &str = "dimetric.toml";

/// A platform a game can be staged for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Platform {
    /// What the CLI calls it.
    pub name: &'static str,
    /// The Rust target triple, for whoever builds the runtime.
    pub triple: &'static str,
    /// Extension an executable carries there.
    pub exe_suffix: &'static str,
}

/// The three targets v0.1 supports.
pub const PLATFORMS: [Platform; 3] = [
    Platform {
        name: "linux",
        triple: "x86_64-unknown-linux-gnu",
        exe_suffix: "",
    },
    Platform {
        name: "macos",
        triple: "aarch64-apple-darwin",
        exe_suffix: "",
    },
    Platform {
        name: "windows",
        triple: "x86_64-pc-windows-msvc",
        exe_suffix: ".exe",
    },
];

/// Find a platform by its short name or by its triple.
pub fn platform(name: &str) -> Option<Platform> {
    PLATFORMS
        .iter()
        .copied()
        .find(|p| p.name == name || p.triple == name)
}

/// What run a shipped game starts from.
///
/// A packaged game boots with the seed in its manifest, and every stream a
/// script draws from is derived from that one number — so a fixed seed means
/// the first run of every launch of the same build is identical, and the second
/// is identical to the second. For a roguelike whose title screen offers a new
/// run, that is the same three essences every time somebody quits and comes
/// back.
///
/// Nothing a script can read differs between two launches: `app.today()` is the
/// only thing from outside and it holds still for a day, which is the Daily
/// Descent and deliberately not this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BootSeed {
    /// This number, every launch. What a fixture, a capture and a bug report
    /// want, and still the default.
    Fixed(u64),
    /// A different number each launch, read from outside at startup.
    ///
    /// Safe for exactly one reason: the seed goes into the recording, the same
    /// way `--date` does. A session's log carries the seed it ran on, so a
    /// launch-seeded session replays to the run it recorded rather than to
    /// whatever the clock says at replay time.
    Launch,
}

impl Default for BootSeed {
    fn default() -> BootSeed {
        BootSeed::Fixed(0)
    }
}

impl BootSeed {
    /// The spelling a manifest uses.
    pub fn parse(text: &str) -> Option<BootSeed> {
        match text {
            "launch" => Some(BootSeed::Launch),
            number => number.parse().ok().map(BootSeed::Fixed),
        }
    }

    /// Settle on a number for this launch.
    ///
    /// Reads the clock for [`BootSeed::Launch`], which is the one place in the
    /// engine that is allowed to and the reason this returns a `u64` rather
    /// than being read twice: the answer is settled once, before the first
    /// tick, and written into the recording.
    ///
    /// Nanoseconds and the process id, through BLAKE3. The nanoseconds are what
    /// make two launches differ and the hash is what stops the *low bits*
    /// marching in lockstep between launches a moment apart — a seed is a whole
    /// number to `Rng::new`, and two seeds differing in one low bit should not
    /// look related. The process id covers the case of two launches inside one
    /// clock tick, which a coarse platform clock can produce.
    pub fn resolve(self) -> u64 {
        match self {
            BootSeed::Fixed(seed) => seed,
            BootSeed::Launch => {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                let mut bytes = Vec::with_capacity(20);
                bytes.extend_from_slice(&nanos.to_le_bytes());
                bytes.extend_from_slice(&std::process::id().to_le_bytes());
                let digest = blake3::hash(&bytes);
                u64::from_le_bytes(digest.as_bytes()[..8].try_into().expect("8 bytes"))
            }
        }
    }
}

impl std::fmt::Display for BootSeed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BootSeed::Fixed(seed) => write!(f, "{seed}"),
            BootSeed::Launch => write!(f, "launch"),
        }
    }
}

/// What to stage.
pub struct PackageRequest {
    /// Platform to stage for.
    pub platform: Platform,
    /// Scene the game boots into, relative to the project root.
    pub scene: String,
    /// Run seed the game starts from, or `launch` for a new one each time.
    pub seed: BootSeed,
    /// Where to put it. Defaults to `build/<platform>` under the project.
    pub out: Option<PathBuf>,
    /// A runtime executable to copy in, if one has been built.
    pub runtime: Option<PathBuf>,
    /// What the game calls itself, overriding `[game] name` in `project.toml`.
    ///
    /// For a build that ships under a different name from the one the project
    /// is developed under. `None` means the project decides, and a project
    /// that does not decide gets its directory's name, as it always has.
    pub name: Option<String>,
}

/// What staging produced.
pub struct Staged {
    /// The directory that now holds the game.
    pub out: PathBuf,
    /// Files copied, relative to `out`.
    pub files: Vec<String>,
    /// Total bytes written.
    pub bytes: u64,
    /// Where the runtime landed, if one was given.
    pub runtime: Option<PathBuf>,
    /// Anything that went wrong but did not stop the staging.
    pub diagnostics: Diagnostics,
}

/// Directories a game reads at runtime, in the order they are staged.
const DIRECTORIES: [&str; 4] = ["assets", "scripts", "prefabs", ".import"];

/// Stage a project for a platform.
///
/// Copies the scenes, the settings, the scripts, the prefabs, the assets and
/// the import cache, and writes the manifest. An allowlist rather than
/// "everything but": a project accumulates notes, recordings and half-finished
/// experiments, and a shipped game should carry what it runs on and nothing
/// else.
///
/// The cost of an allowlist is that a file nobody remembered is a file that
/// does not ship, and the game runs on a default instead of saying so.
pub fn stage(project: &mut Project, request: PackageRequest) -> Result<Staged, Diagnostics> {
    let root = project.root.clone();
    let out = request
        .out
        .clone()
        .unwrap_or_else(|| root.join("build").join(request.platform.name));

    // Import first, so a shipped game carries a cache that matches its sources
    // rather than whatever was there when someone last opened the editor.
    project.import_assets();

    let mut diagnostics = Diagnostics::new();
    if !root.join(&request.scene).is_file() {
        return Err(Diagnostics(vec![Diagnostic::new(
            Code::ASSET_MISSING,
            format!("{} is not a scene in this project", request.scene),
        )
        .with_field("scene", request.scene.clone())]));
    }

    // A fresh directory: leaving a previous build's files in place is how a
    // deleted asset keeps shipping.
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| cannot(&out, e))?;
    }
    std::fs::create_dir_all(&out).map_err(|e| cannot(&out, e))?;

    let mut staged = Staged {
        out: out.clone(),
        files: Vec::new(),
        bytes: 0,
        runtime: None,
        diagnostics: Diagnostics::new(),
    };

    for entry in read_sorted(&root)? {
        if entry.extension().and_then(|e| e.to_str()) == Some("dim") && entry.is_file() {
            copy_into(&entry, &root, &out, &mut staged)?;
        }
    }
    if root
        .join(dimetric_scene::project_kinds::KINDS_FILE)
        .is_file()
    {
        copy_into(
            &root.join(dimetric_scene::project_kinds::KINDS_FILE),
            &root,
            &out,
            &mut staged,
        )?;
    }

    // `project.toml`, which decides what a run *means*: the tick rate, the
    // canvas, the render resolution and the input bindings.
    //
    // It was not staged, and every one of those silently fell back to an
    // engine default in the shipped game. Three of the four usually match by
    // luck; the bindings do not. A game that declared `dash = ["Tab"]` shipped
    // with dash on Left Shift, so the key its own instructions named did
    // nothing -- and the keys that happened to agree with the defaults kept
    // working, which is what made it look like one broken feature rather than
    // a missing file.
    //
    // The same file also carries the replay contract. A recorded run replayed
    // against a staged build at a different tick rate is not the same run.
    if root.join(crate::settings::SETTINGS_FILE).is_file() {
        copy_into(
            &root.join(crate::settings::SETTINGS_FILE),
            &root,
            &out,
            &mut staged,
        )?;
    }
    for directory in DIRECTORIES {
        let from = root.join(directory);
        if from.is_dir() {
            copy_tree(&from, &root, &out, &mut staged)?;
        }
    }

    if let Some(runtime) = &request.runtime {
        let name = format!("{}{}", project_name(&root), request.platform.exe_suffix);
        let to = out.join(&name);
        std::fs::copy(runtime, &to).map_err(|e| cannot(&to, e))?;
        copy_permissions(runtime, &to);
        staged.bytes += std::fs::metadata(&to).map(|m| m.len()).unwrap_or(0);
        staged.files.push(name);
        staged.runtime = Some(to);
    } else {
        diagnostics.push(
            Diagnostic::new(
                Code::NOT_IMPLEMENTED,
                "staged without a runtime; pass --runtime with a dim-play built for this target",
            )
            .with_field("target", request.platform.triple.to_string())
            .with_severity(dimetric_core::Severity::Warning),
        );
    }

    // The window's title and icon: host presentation, carried in the manifest
    // beside the boot scene because the window is created before any of the
    // project is read and no script can reach it.
    //
    // Nothing further down the engine sees either of them. They cannot reach
    // the simulation and so cannot reach the state hash, which is the same
    // line audio is on.
    let name = request
        .name
        .clone()
        .or_else(|| project.settings.game.name.clone())
        .unwrap_or_else(|| project_name(&root));
    let icon = match &project.settings.game.icon {
        None => None,
        Some(declared) => match stage_icon(declared, &root, &out, &mut staged) {
            Ok(()) => Some(declared.clone()),
            Err(d) => {
                // A warning, and the game ships without it. Refusing the whole
                // build over a window decoration would be the wrong trade, and
                // discovering it at launch instead of here would be worse than
                // either.
                diagnostics.push(d);
                None
            }
        },
    };

    let manifest = manifest_text(&request, &name, icon.as_deref());
    let path = out.join(MANIFEST);
    std::fs::write(&path, &manifest).map_err(|e| cannot(&path, e))?;
    staged.bytes += manifest.len() as u64;
    staged.files.push(MANIFEST.to_string());
    staged.files.sort();

    staged.diagnostics = diagnostics;
    Ok(staged)
}

/// Fold a staged directory into one executable.
///
/// Reads back what [`stage`] just wrote and appends it to a copy of the runtime,
/// so the bytes in the single file are the staged bytes — the property the
/// module note defends, kept by reading rather than by rewriting. The staged
/// directory is left alone: it is the thing that was verified, and a build worth
/// shipping is a build worth keeping beside the one-file copy of it.
///
/// The runtime itself is not in the archive; it *is* the file.
pub fn fold(staged: &Staged) -> Result<Folded, Diagnostics> {
    let Some(runtime) = &staged.runtime else {
        return Err(Diagnostics(vec![Diagnostic::new(
            Code::NOT_IMPLEMENTED,
            "a single-file build is a runtime with the game appended to it, so it needs \
             --runtime with a dim-play built for this target",
        )]));
    };
    let runtime_name = runtime
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut files = Vec::new();
    for relative in &staged.files {
        // Everything but the runtime, which the archive is appended to.
        if relative == &runtime_name {
            continue;
        }
        let from = staged.out.join(relative);
        let bytes = std::fs::read(&from).map_err(|e| cannot(&from, e))?;
        files.push((relative.replace('\\', "/"), bytes));
    }

    let out = staged.out.join(format!("{runtime_name}-single"));
    let manifest = crate::archive::write(runtime, &out, &files).map_err(|e| cannot(&out, e))?;
    copy_permissions(runtime, &out);
    // Named after the runtime once it exists, so what ships is `sorcerer`
    // rather than `sorcerer-single`.
    let final_path = staged.out.join(&runtime_name);
    std::fs::rename(&out, &final_path).map_err(|e| cannot(&final_path, e))?;

    let bytes = std::fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    Ok(Folded {
        path: final_path,
        files: files.len(),
        bytes,
        hash: manifest.hash,
    })
}

/// What folding a staged game produced.
pub struct Folded {
    /// The one file.
    pub path: PathBuf,
    /// How many files are inside it.
    pub files: usize,
    /// How big it is.
    pub bytes: u64,
    /// BLAKE3 of the appended payload, which `dim inspect` checks.
    pub hash: String,
}

/// The manifest a staged game boots from.
fn manifest_text(request: &PackageRequest, name: &str, icon: Option<&str>) -> String {
    let mut text = format!(
        "# What this game is and how it starts. Written by `dim build`.\n\
         format = \"dimetric-game\"\n\
         version = 1\n\
         name = {name:?}\n\
         engine = {:?}\n\
         target = {:?}\n\
         scene = {:?}\n\
         seed = {}\n",
        env!("CARGO_PKG_VERSION"),
        request.platform.triple,
        request.scene,
        // A number bare and `launch` quoted, which is what `BootSeed::parse`
        // reads back and what a person opening the manifest would expect.
        match request.seed {
            BootSeed::Fixed(seed) => seed.to_string(),
            BootSeed::Launch => "\"launch\"".to_string(),
        },
    );
    // Omitted rather than written empty, so a game with no icon and a game
    // whose icon could not be read produce the same manifest — there is one
    // runtime behaviour for both, and the build already said which happened.
    if let Some(icon) = icon {
        text.push_str(&format!("icon = {icon:?}\n"));
    }
    text
}

/// Copy a declared icon into the staged game, checking it first.
///
/// Decoded here and not merely copied: an icon that is not a readable PNG is a
/// mistake worth hearing about at build time, when the file is in front of the
/// person who chose it, rather than at launch on somebody else's machine where
/// nothing can be done about it.
fn stage_icon(
    declared: &str,
    root: &Path,
    out: &Path,
    staged: &mut Staged,
) -> Result<(), Diagnostic> {
    let unusable = |why: String| {
        Diagnostic::new(
            Code::ICON_UNUSABLE,
            format!(
                "game.icon {declared:?}: {}; the game will ship without an icon",
                why.trim_end_matches('.')
            ),
        )
        .with_field("icon", declared.to_string())
        .with_severity(dimetric_core::Severity::Warning)
    };

    // Inside the project. An icon from somewhere else would stage a file the
    // project does not contain, which is the one thing a staged build is for
    // not doing.
    let relative = Path::new(declared);
    if relative.is_absolute() || relative.components().any(|c| c.as_os_str() == "..") {
        return Err(unusable(
            "an icon is a path inside the project, relative to its root".into(),
        ));
    }
    let from = root.join(relative);
    let bytes = std::fs::read(&from).map_err(|e| unusable(e.to_string()))?;
    dimetric_assets::image::decode_png_bytes(&bytes, &from).map_err(|e| unusable(e.to_string()))?;

    // Already staged when it lives under `assets/`, which is where an icon
    // usually goes. Copying it twice would double-count the bytes and list the
    // file twice.
    let listed = relative.to_string_lossy().replace('\\', "/");
    if staged.files.contains(&listed) {
        return Ok(());
    }
    copy_into(&from, root, out, staged).map_err(|d| {
        d.0.into_iter()
            .next()
            .unwrap_or_else(|| unusable("could not be copied".into()))
    })
}

/// The name a staged game's manifest gives it.
pub fn game_name(manifest: &str) -> Option<String> {
    field(manifest, "name")
}

/// The icon a staged game's manifest names, relative to the game's root.
pub fn game_icon(manifest: &str) -> Option<String> {
    field(manifest, "icon")
}

/// What a staged game calls itself: the project directory's name.
fn project_name(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("game")
        .to_string()
}

/// The boot scene a staged game's manifest names.
pub fn boot_scene(manifest: &str) -> Option<String> {
    field(manifest, "scene")
}

/// The seed a staged game's manifest names.
///
/// `None` when the manifest says nothing, or says something this build cannot
/// read — the caller decides what to do about that, and `dim-play` reports it
/// rather than inventing a run.
pub fn boot_seed(manifest: &str) -> Option<BootSeed> {
    BootSeed::parse(&field(manifest, "seed")?)
}

fn field(manifest: &str, key: &str) -> Option<String> {
    let doc: toml_edit::DocumentMut = manifest.parse().ok()?;
    let item = doc.get(key)?;
    item.as_str()
        .map(str::to_string)
        .or_else(|| item.as_integer().map(|i| i.to_string()))
}

fn read_sorted(dir: &Path) -> Result<Vec<PathBuf>, Diagnostics> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| cannot(dir, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    // Sorted, so a staged directory is the same on every machine — which is
    // what lets two builds of one commit be compared.
    entries.sort();
    Ok(entries)
}

fn copy_tree(from: &Path, root: &Path, out: &Path, staged: &mut Staged) -> Result<(), Diagnostics> {
    for entry in read_sorted(from)? {
        if entry.is_dir() {
            copy_tree(&entry, root, out, staged)?;
        } else {
            copy_into(&entry, root, out, staged)?;
        }
    }
    Ok(())
}

fn copy_into(file: &Path, root: &Path, out: &Path, staged: &mut Staged) -> Result<(), Diagnostics> {
    let relative = file.strip_prefix(root).unwrap_or(file);
    let to = out.join(relative);
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| cannot(parent, e))?;
    }
    let bytes = std::fs::copy(file, &to).map_err(|e| cannot(&to, e))?;
    staged.bytes += bytes;
    staged
        .files
        .push(relative.to_string_lossy().replace('\\', "/"));
    Ok(())
}

/// Keep the executable bit, which is the difference between a game and a file.
#[cfg(unix)]
fn copy_permissions(from: &Path, to: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(from) {
        let _ = std::fs::set_permissions(
            to,
            std::fs::Permissions::from_mode(meta.permissions().mode()),
        );
    }
}

#[cfg(not(unix))]
fn copy_permissions(_from: &Path, _to: &Path) {}

fn cannot(path: &Path, e: std::io::Error) -> Diagnostics {
    Diagnostics(vec![Diagnostic::new(
        Code::COMMAND_REJECTED,
        format!("cannot write {}: {e}", path.display()),
    )])
}
