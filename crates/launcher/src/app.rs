//! Interface du launcher : la bibliothèque de jeux et l'écran de jeu.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Key, RichText};

use crate::audio::AudioOutput;
use crate::game::GameSession;
use crate::library::Library;
use crate::systems::{self, System};

const CARD_SIZE: [f32; 2] = [200.0, 56.0];

pub struct EmurustApp {
    systems: Vec<System>,
    library: Library,
    audio: Option<AudioOutput>,
    session: Option<GameSession>,
    /// Message d'erreur affiché en haut de la bibliothèque.
    error: Option<String>,
    search: String,
}

impl EmurustApp {
    pub fn new(roms_dir: PathBuf, start_game: Option<PathBuf>) -> Self {
        let systems = systems::all();
        let library = Library::scan(&roms_dir, &systems);
        let audio = AudioOutput::open();
        if audio.is_none() {
            eprintln!("Aucune sortie audio trouvée : les jeux tourneront sans son.");
        }
        let mut app = EmurustApp {
            systems,
            library,
            audio,
            session: None,
            error: None,
            search: String::new(),
        };
        if let Some(path) = start_game {
            app.launch(&path);
        }
        app
    }

    fn launch(&mut self, path: &Path) {
        let Some(index) = systems::for_path(&self.systems, path) else {
            self.error = Some(format!(
                "Aucun émulateur ne gère ce fichier : {}",
                path.display()
            ));
            return;
        };
        let sample_rate = self.audio.as_ref().map(|out| out.sample_rate);
        match GameSession::start(&self.systems[index], path, sample_rate) {
            Ok(session) => {
                self.error = None;
                self.session = Some(session);
            }
            Err(err) => self.error = Some(err),
        }
    }

    fn rescan(&mut self) {
        self.library = Library::scan(&self.library.root, &self.systems);
    }

    fn game_ui(&mut self, ui: &mut egui::Ui) {
        let quit = ui.input(|i| i.key_pressed(Key::Escape));

        egui::Panel::bottom("game_help").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("⬅ Bibliothèque (Échap)").clicked() {
                    self.session = None;
                }
                ui.separator();
                ui.label(
                    "Flèches : croix   X : A   Z : B   Entrée : Start   Retour arrière : Select",
                );
            });
        });

        let audio = self.audio.as_ref();
        if let Some(session) = self.session.as_mut() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "emurust - {}",
                    session.title
                )));
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(Color32::BLACK))
                .show(ui, |ui| session.ui(ui, audio));
        }

        if quit {
            self.session = None;
        }
        if self.session.is_none() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title("emurust".into()));
        }
    }

    fn library_ui(&mut self, ui: &mut egui::Ui) {
        let mut to_launch = None;

        egui::Panel::top("library_header").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading(RichText::new("emurust").strong());
                ui.label(format!("{} jeu(x)", self.library.total()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("📂 Ouvrir le dossier des ROMs").clicked() {
                        open_in_file_manager(&self.library.root);
                    }
                    if ui.button("⟳ Actualiser").clicked() {
                        self.rescan();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("Rechercher un jeu")
                            .desired_width(180.0),
                    );
                });
            });
            ui.label(
                RichText::new(format!(
                    "Dossier des ROMs : {}",
                    self.library.root.display()
                ))
                .small()
                .weak(),
            );
            if let Some(err) = &self.error {
                ui.colored_label(Color32::from_rgb(220, 80, 80), err);
            }
            ui.add_space(6.0);
        });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let filter = self.search.to_lowercase();
                for (system, games) in self.systems.iter().zip(&self.library.games) {
                    ui.add_space(8.0);
                    ui.heading(format!("{}  ({})", system.info.name, games.len()));
                    ui.separator();

                    if games.is_empty() {
                        let dir = self.library.system_dir(system);
                        ui.label(
                            RichText::new(format!(
                                "Aucun jeu pour l'instant. Placez vos fichiers .{} dans {} puis cliquez sur Actualiser.",
                                system.info.extensions.join(", ."),
                                dir.display()
                            ))
                            .weak(),
                        );
                        continue;
                    }

                    ui.horizontal_wrapped(|ui| {
                        for game in games
                            .iter()
                            .filter(|game| game.name.to_lowercase().contains(&filter))
                        {
                            let button = egui::Button::new(RichText::new(&game.name).size(15.0));
                            let response = ui.add_sized(CARD_SIZE, button).on_hover_text(game.path.display().to_string());
                            if response.clicked() {
                                to_launch = Some(game.path.clone());
                            }
                        }
                    });
                }
            });
        });

        if let Some(path) = to_launch {
            self.launch(&path);
        }
    }
}

impl eframe::App for EmurustApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Fermer la fenêtre pendant une partie écrit la sauvegarde.
        if ui.input(|i| i.viewport().close_requested()) {
            self.session = None;
        }
        if self.session.is_some() {
            self.game_ui(ui);
        } else {
            self.library_ui(ui);
        }
    }
}

fn open_in_file_manager(path: &Path) {
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Err(err) = std::process::Command::new(program).arg(path).spawn() {
        eprintln!("Impossible d'ouvrir {} : {err}", path.display());
    }
}
