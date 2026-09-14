//! The editor window.
//!
//! A client and nothing more. Every widget in here reads a model from
//! `dimetric_editor` and, when the user does something, dispatches an
//! [`Action`]. There is no scene logic in this file, and there should never be
//! any: §16 names editor scope as the highest risk in the project, and a client
//! that holds no logic cannot grow any.
//!
//! Behind the `gui` feature, so `cargo build --workspace` does not pull `egui`
//! and `winit` into every build of the engine.

use dimetric_core::{Severity, Vec2Fx};
use dimetric_editor::panels::{
    asset_rows, gizmos, inspector_rows, tree_rows, viewport::pick, Viewport,
};
use dimetric_editor::{Action, Editor, Mode};
use eframe::egui;

/// Size the viewport renders at, before it is scaled into the panel.
const VIEWPORT: (u32, u32) = (640, 360);

fn main() -> eframe::Result<()> {
    let mut args = std::env::args().skip(1);
    let root = args.next().unwrap_or_else(|| ".".to_string());
    let scene = args.next().unwrap_or_else(|| {
        first_scene(&root).unwrap_or_else(|| {
            eprintln!("no scene given and none found in {root}");
            std::process::exit(2);
        })
    });

    let editor = match Editor::open(&root, &scene, 0) {
        Ok(editor) => editor,
        Err(diagnostics) => {
            eprintln!("{diagnostics}");
            std::process::exit(1);
        }
    };

    eframe::run_native(
        "Dimetric",
        eframe::NativeOptions::default(),
        Box::new(|_| Ok(Box::new(Studio::new(editor)))),
    )
}

/// The first `.dim` in a directory, so a one-scene project needs no argument.
fn first_scene(root: &str) -> Option<String> {
    let mut found: Vec<String> = std::fs::read_dir(root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".dim"))
        .map(|n| n.trim_end_matches(".dim").to_string())
        .collect();
    found.sort();
    found.into_iter().next()
}

/// Which panel the bottom dock is showing.
#[derive(PartialEq, Eq)]
enum Dock {
    Console,
    Assets,
}

struct Studio {
    editor: Editor,
    dock: Dock,
    /// The last frame drawn, uploaded as a texture.
    viewport: Option<egui::TextureHandle>,
    /// What the renderer said, if it could not draw.
    viewport_error: Option<String>,
    /// Name being typed into the rename field.
    rename: String,
    /// Kind to create when the button is pressed.
    new_kind: String,
    /// Whether the viewport needs redrawing.
    dirty: bool,
}

impl Studio {
    fn new(editor: Editor) -> Studio {
        Studio {
            editor,
            dock: Dock::Console,
            viewport: None,
            viewport_error: None,
            rename: String::new(),
            new_kind: "Sprite2D".to_string(),
            dirty: true,
        }
    }

    /// Dispatch, and redraw the viewport if anything visible changed.
    fn act(&mut self, action: Action) {
        let visible = action.edits_scene()
            || matches!(
                action,
                Action::Play
                    | Action::Stop
                    | Action::StepTick
                    | Action::ScrubTo(_)
                    | Action::Select(_)
                    | Action::ToggleSelect(_)
                    | Action::SelectNone
                    | Action::LookAt(_)
                    | Action::Zoom(_)
            );
        self.editor.dispatch(action);
        self.dirty |= visible;
    }

    /// Redraw the viewport into a texture.
    ///
    /// Through the host's offscreen path rather than a shared device: `egui`
    /// is on one `wgpu` release and the renderer on the next, so they cannot
    /// share a device yet. The pixels make a round trip that they would not
    /// need to if the versions lined up, which is a cost to remove when they do
    /// rather than a design.
    fn redraw(&mut self, ctx: &egui::Context) {
        if !self.dirty {
            return;
        }
        self.dirty = false;

        let settings = dimetric_render::RenderSettings {
            internal_resolution: (VIEWPORT.0 / 2, VIEWPORT.1 / 2),
            integer_upscale: true,
            pixel_snap: true,
            ambient: dimetric_scene::Color::WHITE,
        };

        // In play mode the running simulation's own scene, so the viewport
        // shows the state the editor is actually running.
        let drawn = match self.editor.playback.sim() {
            Some(sim) => {
                let scene = sim.state().scene.clone();
                dimetric_host::draw_scene(&self.editor.project, &scene, VIEWPORT, settings)
            }
            None => match self.editor.project.runtime_scene() {
                Ok((scene, _)) => {
                    dimetric_host::draw_scene(&self.editor.project, &scene, VIEWPORT, settings)
                }
                Err(d) => Err(d),
            },
        };

        match drawn {
            Ok(frame) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [frame.width as usize, frame.height as usize],
                    &frame.pixels,
                );
                self.viewport = Some(ctx.load_texture("viewport", image, Default::default()));
                self.viewport_error = None;
                self.editor.console.extend(frame.diagnostics);
            }
            Err(diagnostics) => {
                self.viewport = None;
                self.viewport_error = Some(diagnostics.to_string());
            }
        }
    }
}

impl eframe::App for Studio {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Play mode drives itself. egui only repaints when something happens,
        // so a running simulation has to ask for the next frame.
        if self.editor.playback.mode() == Mode::Playing {
            self.editor.playback.advance(Default::default());
            self.dirty = true;
            ctx.request_repaint();
        }
        self.redraw(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        // Order matters: the outermost panel is added first and the central one
        // last, which is what leaves the viewport whatever is left over.
        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::left("tree")
            .default_size(240.0)
            .show(ui, |ui| self.tree(ui));
        egui::Panel::right("inspector")
            .default_size(300.0)
            .show(ui, |ui| self.inspector(ui));
        egui::Panel::bottom("dock")
            .default_size(180.0)
            .show(ui, |ui| self.dock(ui));
        egui::CentralPanel::default_margins().show(ui, |ui| self.viewport(ui));
    }
}

impl Studio {
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                self.act(Action::Save);
            }
            if ui.button("Undo").clicked() {
                self.act(Action::Undo);
            }
            if ui.button("Redo").clicked() {
                self.act(Action::Redo);
            }
            ui.separator();

            match self.editor.playback.mode() {
                Mode::Editing => {
                    if ui.button("Play").clicked() {
                        self.act(Action::Play);
                    }
                }
                Mode::Playing => {
                    if ui.button("Pause").clicked() {
                        self.act(Action::Pause);
                    }
                }
                Mode::Paused => {
                    if ui.button("Resume").clicked() {
                        self.act(Action::Play);
                    }
                }
            }
            if self.editor.playback.is_running() {
                if ui.button("Step").clicked() {
                    self.act(Action::StepTick);
                }
                if ui.button("Stop").clicked() {
                    self.act(Action::Stop);
                }

                // The replay scrubber. Its range is what has been played, since
                // a tick that has not happened cannot be scrubbed to.
                let furthest = self.editor.playback.furthest();
                let mut tick = self.editor.playback.tick();
                ui.separator();
                let slider = ui.add_enabled(
                    furthest > 0,
                    egui::Slider::new(&mut tick, 0..=furthest.max(1)).text("tick"),
                );
                if slider.changed() {
                    self.act(Action::ScrubTo(tick));
                }
                ui.label(format!("{} snapshots", self.editor.playback.keyframes()));
            }
        });
    }

    fn tree(&mut self, ui: &mut egui::Ui) {
        ui.heading("Scene");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_kind).desired_width(120.0));
            if ui.button("Add").clicked() {
                let parent = self.editor.selected();
                self.act(Action::Create {
                    kind: self.new_kind.clone(),
                    name: format!("New{}", self.new_kind),
                    parent,
                });
            }
        });
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in tree_rows(&self.editor) {
                ui.horizontal(|ui| {
                    ui.add_space(row.depth as f32 * 12.0);
                    if row.has_children {
                        let glyph = if row.folded { "▸" } else { "▾" };
                        if ui.small_button(glyph).clicked() {
                            self.act(Action::ToggleFold(row.uid));
                        }
                    } else {
                        ui.add_space(20.0);
                    }
                    let label = if row.instance {
                        format!("{} [{}] ↪", row.name, row.kind)
                    } else {
                        format!("{} [{}]", row.name, row.kind)
                    };
                    if ui.selectable_label(row.selected, label).clicked() {
                        if ui.input(|i| i.modifiers.command) {
                            self.act(Action::ToggleSelect(row.uid));
                        } else {
                            self.act(Action::Select(vec![row.uid]));
                            self.rename = row.name.clone();
                        }
                    }
                });
            }
        });
    }

    fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.heading("Inspector");
        let Some(node) = self.editor.selected() else {
            ui.label("Nothing selected.");
            return;
        };

        ui.horizontal(|ui| {
            ui.label("name");
            if ui.text_edit_singleline(&mut self.rename).lost_focus() && !self.rename.is_empty() {
                self.act(Action::Rename {
                    node,
                    name: self.rename.clone(),
                });
            }
        });
        if ui.button("Delete").clicked() {
            self.act(Action::Delete(node));
            return;
        }
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for row in inspector_rows(&self.editor, node) {
                let mut text = row.literal.clone();
                ui.horizontal(|ui| {
                    let label = ui.label(&row.key);
                    if !row.doc.is_empty() {
                        label.on_hover_text(&row.doc);
                    }
                    // Greyed when it is the schema's default, so what the file
                    // actually says is visible at a glance.
                    let field = egui::TextEdit::singleline(&mut text)
                        .text_color_opt(row.is_default.then_some(egui::Color32::GRAY))
                        .desired_width(150.0);
                    if ui.add(field).lost_focus() && text != row.literal {
                        self.act(Action::SetProperty {
                            node,
                            key: row.key.clone(),
                            literal: Some(text.clone()),
                        });
                    }
                    if row.explicit && ui.small_button("×").clicked() {
                        self.act(Action::SetProperty {
                            node,
                            key: row.key.clone(),
                            literal: None,
                        });
                    }
                });
            }
        });
    }

    fn dock(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.dock, Dock::Console, "Console");
            ui.selectable_value(&mut self.dock, Dock::Assets, "Assets");
            if self.dock == Dock::Console {
                if ui.button("Clear").clicked() {
                    self.editor.console.clear();
                }
            } else if ui.button("Rescan").clicked() {
                self.editor.project.scan_assets();
            }
        });
        ui.separator();

        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .show(ui, |ui| match self.dock {
                Dock::Console => {
                    for entry in self.editor.console.entries() {
                        // The code, not just the prose: a designer who can read
                        // DIM0301 and the property it names can find the typo.
                        let colour = match entry.severity {
                            Severity::Error => egui::Color32::LIGHT_RED,
                            Severity::Warning => egui::Color32::YELLOW,
                            Severity::Note => egui::Color32::GRAY,
                        };
                        ui.colored_label(colour, format!("{} {}", entry.code, entry.message));
                    }
                }
                Dock::Assets => {
                    for row in asset_rows(&self.editor) {
                        ui.horizontal(|ui| {
                            ui.label(&row.name);
                            ui.weak(&row.kind);
                            if row.stale {
                                ui.colored_label(egui::Color32::YELLOW, "stale");
                            }
                            for clip in &row.clips {
                                ui.weak(format!("▸{clip}"));
                            }
                        });
                    }
                }
            });
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        if let Some(error) = &self.viewport_error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
            return;
        }
        let Some(texture) = self.viewport.clone() else {
            ui.label("Nothing drawn yet.");
            return;
        };

        let response = ui.add(
            egui::Image::new(&texture)
                .fit_to_exact_size(egui::vec2(VIEWPORT.0 as f32, VIEWPORT.1 as f32))
                .sense(egui::Sense::click_and_drag()),
        );

        let viewport = Viewport::from_sidecar(&self.editor, VIEWPORT);
        let handles = gizmos(&self.editor);
        let local = |p: egui::Pos2| (p.x - response.rect.min.x, p.y - response.rect.min.y);

        if response.clicked() {
            if let Some(at) = response.interact_pointer_pos() {
                match pick(&handles, &viewport, local(at)) {
                    Some(node) => self.act(Action::Select(vec![node])),
                    None => self.act(Action::SelectNone),
                }
            }
        }
        // Dragging a selected node moves it, through the bus like everything
        // else. The pointer is in pixels and the scene is in fixed point; the
        // viewport does that conversion, not this file.
        if response.dragged() {
            if let (Some(node), Some(at)) =
                (self.editor.selected(), response.interact_pointer_pos())
            {
                let to: Vec2Fx = viewport.to_world(local(at));
                self.act(Action::Move { node, to });
            }
        }

        // Handles over the image, so a node with no sprite is still clickable.
        let painter = ui.painter_at(response.rect);
        for gizmo in handles {
            let (x, y) = viewport.to_screen(gizmo.at);
            let centre = response.rect.min + egui::vec2(x, y);
            let colour = if gizmo.selected {
                egui::Color32::from_rgb(0xFF, 0xB3, 0x47)
            } else {
                egui::Color32::from_white_alpha(80)
            };
            painter.circle_stroke(centre, 5.0, egui::Stroke::new(1.5, colour));
        }
    }
}
