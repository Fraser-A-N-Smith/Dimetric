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

/// What to stage.
pub struct PackageRequest {
    /// Platform to stage for.
    pub platform: Platform,
    /// Scene the game boots into, relative to the project root.
    pub scene: String,
    /// Run seed the game starts from.
    pub seed: u64,
    /// Where to put it. Defaults to `build/<platform>` under the project.
    pub out: Option<PathBuf>,
    /// A runtime executable to copy in, if one has been built.
    pub runtime: Option<PathBuf>,
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
/// Copies the scenes, the scripts, the prefabs, the assets and the import
/// cache, and writes the manifest. An allowlist rather than "everything but":
/// a project accumulates notes, recordings and half-finished experiments, and a
/// shipped game should carry what it runs on and nothing else.
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

    let manifest = manifest_text(&request, project_name(&root));
    let path = out.join(MANIFEST);
    std::fs::write(&path, &manifest).map_err(|e| cannot(&path, e))?;
    staged.bytes += manifest.len() as u64;
    staged.files.push(MANIFEST.to_string());
    staged.files.sort();

    staged.diagnostics = diagnostics;
    Ok(staged)
}

/// The manifest a staged game boots from.
fn manifest_text(request: &PackageRequest, name: String) -> String {
    format!(
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
        request.seed,
    )
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
pub fn boot_seed(manifest: &str) -> Option<u64> {
    field(manifest, "seed")?.parse().ok()
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
