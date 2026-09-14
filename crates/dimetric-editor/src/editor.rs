//! The editor, without a window.
//!
//! [`Editor`] owns a project, the view state beside it, and a console. It has
//! no drawing code and no dependency on a GUI toolkit, which is the point:
//! every rule worth checking about an editor is about what it does to the
//! project, and none of those rules need a window to check.
//!
//! # The contract
//!
//! Every scene mutation goes through the command bus. [`Editor::dispatch`]
//! returns the commands it produced, so a scripted session can assert both
//! halves: that an edit produced commands, and that anything which is not an
//! edit left the `.dim` byte-identical.

use std::path::PathBuf;

use dimetric_core::{Code, Diagnostic, Diagnostics, NodeUid, Vec2Fx};
use dimetric_host::{Command, Project};
use dimetric_scene::Value;

use crate::action::Action;
use crate::console::Console;
use crate::playback::Playback;
use crate::sidecar::Sidecar;

/// What one dispatched action did.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// Commands applied to the bus, in order.
    ///
    /// Empty for anything that only touched view state — which is the other
    /// half of the contract, and is asserted as such.
    pub commands: Vec<Command>,
    /// Anything that went wrong, or that is worth saying.
    pub diagnostics: Diagnostics,
    /// Whether the scene file on disk was rewritten.
    pub saved: bool,
}

impl Outcome {
    /// Whether the action was rejected.
    pub fn rejected(&self) -> bool {
        self.diagnostics.has_errors()
    }
}

/// An open project, its view state, and the log.
pub struct Editor {
    /// The project, its open scene and its undo stack.
    pub project: Project,
    /// View state: camera, selection, folds.
    pub sidecar: Sidecar,
    /// Structured diagnostics, newest last.
    pub console: Console,
    /// Play-in-editor and the replay scrubber.
    pub playback: Playback,
    /// Scene the editor has open, project-relative and without the extension.
    scene: String,
}

impl Editor {
    /// Open a project and a scene in it.
    pub fn open(
        root: impl Into<PathBuf>,
        scene: &str,
        id_seed: u64,
    ) -> Result<Editor, Diagnostics> {
        let mut project = Project::open(root, id_seed);
        let diagnostics = project.load_scene(scene)?;
        let mut console = Console::new();
        console.extend(diagnostics);
        let sidecar = Sidecar::load(&project.scene_path(scene));
        Ok(Editor {
            project,
            sidecar,
            console,
            playback: Playback::default(),
            scene: scene.to_string(),
        })
    }

    /// The scene file on disk.
    pub fn scene_path(&self) -> PathBuf {
        self.project.scene_path(&self.scene)
    }

    /// The scene's text as it currently stands in memory.
    pub fn scene_text(&self) -> String {
        self.project
            .open
            .as_ref()
            .map(|d| d.to_text())
            .unwrap_or_default()
    }

    /// The selected node, when exactly one is selected.
    pub fn selected(&self) -> Option<NodeUid> {
        let mut it = self.sidecar.selection.iter();
        match (it.next(), it.next()) {
            (Some(uid), None) => Some(*uid),
            _ => None,
        }
    }

    /// Do what a user did.
    pub fn dispatch(&mut self, action: Action) -> Outcome {
        let mut outcome = Outcome::default();
        match action {
            // -- view state ------------------------------------------------
            Action::Select(nodes) => {
                self.sidecar.selection = nodes.into_iter().collect();
            }
            Action::ToggleSelect(node) => {
                if !self.sidecar.selection.remove(&node) {
                    self.sidecar.selection.insert(node);
                }
            }
            Action::SelectNone => self.sidecar.selection.clear(),
            Action::ToggleFold(node) => {
                if !self.sidecar.folded.remove(&node) {
                    self.sidecar.folded.insert(node);
                }
            }
            Action::LookAt(at) => {
                self.sidecar.camera = [at.x.to_exact_string(), at.y.to_exact_string()];
            }
            Action::Zoom(zoom) => self.sidecar.zoom = zoom,

            // -- scene edits -----------------------------------------------
            Action::Create { kind, name, parent } => {
                let id = self.project.new_node_id();
                self.apply(
                    &mut outcome,
                    Command::CreateNode {
                        id,
                        kind,
                        name,
                        parent,
                        props: Default::default(),
                    },
                );
                // Selecting what you just made is what every editor does, and
                // it is view state, so it costs no command.
                if !outcome.rejected() {
                    self.sidecar.selection = [id].into_iter().collect();
                }
            }
            Action::Delete(node) => {
                self.apply(&mut outcome, Command::DeleteNode { id: node });
                self.sidecar.selection.remove(&node);
                self.sidecar.folded.remove(&node);
            }
            Action::Rename { node, name } => {
                self.apply(&mut outcome, Command::RenameNode { id: node, name });
            }
            Action::SetProperty { node, key, literal } => {
                match self.parse_property(node, &key, literal.as_deref()) {
                    // Writing a value a node already has would make an explicit
                    // key out of a default, so a client that fires on every
                    // field blur would slowly write every default into the file.
                    Ok(value) if self.already_is(node, &key, value.as_ref()) => {}
                    Ok(value) => self.apply(
                        &mut outcome,
                        Command::SetProperty {
                            id: node,
                            key,
                            value,
                        },
                    ),
                    // A bad literal is a line in the console, not a panic in a
                    // text field. The field keeps what the user typed.
                    Err(d) => outcome.diagnostics.push(d),
                }
            }
            Action::Reparent { node, parent } => {
                self.apply(
                    &mut outcome,
                    Command::Reparent {
                        id: node,
                        new_parent: parent,
                    },
                );
            }
            Action::Move { node, to } => {
                self.apply(
                    &mut outcome,
                    Command::SetProperty {
                        id: node,
                        key: "pos".to_string(),
                        value: Some(Value::Vec2(to)),
                    },
                );
            }
            Action::Instance {
                source,
                name,
                parent,
            } => {
                let id = self.project.new_node_id();
                self.apply(
                    &mut outcome,
                    Command::InstancePrefab {
                        id,
                        scene: source,
                        parent,
                        name,
                        pos: None,
                    },
                );
                if !outcome.rejected() {
                    self.sidecar.selection = [id].into_iter().collect();
                }
            }

            // -- history and files -----------------------------------------
            Action::Undo => match self.project.undo() {
                Ok(command) => outcome.commands.push(command),
                Err(d) => outcome.diagnostics.push(d),
            },
            Action::Redo => match self.project.redo() {
                Ok(command) => outcome.commands.push(command),
                Err(d) => outcome.diagnostics.push(d),
            },
            Action::Save => {
                if let Err(d) = self.project.save_scene(None) {
                    outcome.diagnostics.push(d);
                } else {
                    outcome.saved = true;
                }
                if let Err(e) = self.sidecar.save(&self.scene_path()) {
                    outcome.diagnostics.push(Diagnostic::new(
                        Code::COMMAND_REJECTED,
                        format!("cannot write the editor sidecar: {e}"),
                    ));
                }
            }

            // -- playback ---------------------------------------------------
            Action::Play => {
                if let Err(d) = self.playback.play(&mut self.project) {
                    outcome.diagnostics.extend(d);
                }
            }
            Action::Stop => self.playback.stop(),
            Action::Pause => self.playback.pause(),
            Action::StepTick => self.playback.step(),
            Action::ScrubTo(tick) => self.playback.scrub_to(tick),
        }

        self.console.extend(outcome.diagnostics.clone());
        outcome
    }

    /// Apply a command, recording it or its refusal.
    fn apply(&mut self, outcome: &mut Outcome, command: Command) {
        match self.project.apply(command.clone()) {
            Ok(()) => outcome.commands.push(command),
            Err(d) => outcome.diagnostics.extend(d),
        }
    }

    /// Whether a node already holds this value for this key.
    ///
    /// Compared against the node's *effective* value, so setting a property to
    /// its default when the file leaves it out is a no-op rather than a write —
    /// and setting it to the same value the file spells out leaves the file
    /// spelling it out, which is what the author asked for.
    fn already_is(&self, node: NodeUid, key: &str, value: Option<&Value>) -> bool {
        let Some(doc) = self.project.open.as_ref() else {
            return false;
        };
        let Some(n) = doc.scene.by_uid(node).and_then(|id| doc.scene.get(id)) else {
            return false;
        };
        let current = match key {
            "pos" => Some(Value::Vec2(n.transform.pos)),
            "scale" => Some(Value::Vec2(n.transform.scale)),
            "rot" => Some(Value::Angle(n.transform.rot)),
            "visible" => Some(Value::Bool(n.visible)),
            "z" => Some(Value::Int(n.z as i64)),
            "layer" => Some(Value::Int(n.layer as i64)),
            other => n.get(other).cloned(),
        };
        match (value, current) {
            (Some(new), Some(old)) => *new == old,
            // Clearing a key the node does not have is also nothing to do.
            (None, None) => true,
            _ => false,
        }
    }

    /// Turn an inspector field's text into a value the schema accepts.
    fn parse_property(
        &self,
        node: NodeUid,
        key: &str,
        literal: Option<&str>,
    ) -> Result<Option<Value>, Diagnostic> {
        let Some(literal) = literal else {
            return Ok(None);
        };
        let doc = self
            .project
            .open
            .as_ref()
            .ok_or_else(|| Diagnostic::new(Code::COMMAND_REJECTED, "no scene is open"))?;
        let kind = doc
            .scene
            .by_uid(node)
            .and_then(|id| doc.scene.get(id))
            .map(|n| n.kind.clone())
            .ok_or_else(|| {
                Diagnostic::new(Code::NO_SUCH_NODE, format!("no node {}", node.to_text()))
            })?;
        let literal = quote_if_bare(&self.project.registry, &kind, key, literal);
        dimetric_scene::parse_property_literal(&self.project.registry, &kind, key, &literal)
            .map(Some)
    }
}

/// Wrap a bare field value in quotes when its property is a string-ish one.
///
/// The parser takes TOML literals, which is right for the CLI and for an agent
/// writing structured data: `"Alpha"` is a string and `Alpha` is not. It is not
/// right for a text field. Somebody typing a texture path types
/// `asset:sprites/hero`, and an inspector that demanded the quotes would be
/// making the user do the file format's job.
///
/// Only the quoting differs. The value still goes through the same parser and
/// the same schema checks, so a field is not a looser way into the scene — it
/// is the same way, with the quotes supplied.
fn quote_if_bare(
    registry: &dimetric_scene::KindRegistry,
    kind: &str,
    key: &str,
    literal: &str,
) -> String {
    use dimetric_scene::PropertyType;
    let trimmed = literal.trim();
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return literal.to_string();
    }
    let quotable = registry
        .get(kind)
        .and_then(|schema| schema.property(key))
        .map(|property| {
            matches!(
                property.ty,
                PropertyType::Str
                    | PropertyType::Enum(_)
                    | PropertyType::AssetRef
                    | PropertyType::SceneRef
                    | PropertyType::NodeRef
                    | PropertyType::ScriptRef
                    | PropertyType::Color
            )
        })
        .unwrap_or(false);
    if quotable {
        // Escaped, so a name with a quote in it does not produce broken TOML.
        format!("{:?}", trimmed)
    } else {
        literal.to_string()
    }
}

/// Where a node sits in world space, for a gizmo to be drawn at.
pub fn world_position(project: &Project, node: NodeUid) -> Option<Vec2Fx> {
    let doc = project.open.as_ref()?;
    let id = doc.scene.by_uid(node)?;
    doc.scene.world_of(id).map(|t| t.pos)
}
