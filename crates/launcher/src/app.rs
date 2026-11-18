//! Interface du launcher, pensée pour se piloter au clavier comme une console :
//!
//! 1. **Consoles** : un carrousel, une carte par émulateur.
//! 2. **Jeux** : la liste des jeux de la console, avec fiche et aperçu en direct.
//! 3. **Partie** : le jeu en plein écran.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, Event, Id, Key, Pos2, Rect, Sense, Vec2};

use crate::art;
use crate::audio::AudioOutput;
use crate::game::GameSession;
use crate::library::Library;
use crate::preview::{Preview, Previews};
use crate::systems::{self, System};
use crate::theme::{self, alpha};

const HEADER_HEIGHT: f32 = 72.0;
const FOOTER_HEIGHT: f32 = 52.0;
const ROW_HEIGHT: f32 = 46.0;
const WHEEL_STEP: f32 = 40.0;
const TOAST_SECONDS: f64 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Systems,
    Games,
    Playing,
}

pub struct EmurustApp {
    systems: Vec<System>,
    library: Library,
    audio: Option<AudioOutput>,
    session: Option<GameSession>,
    screen: Screen,
    system_index: usize,
    /// Jeu sélectionné, pour chaque console.
    game_index: Vec<usize>,
    /// Première ligne visible de la liste de jeux.
    list_top: usize,
    wheel: f32,
    previews: Previews,
    crt: bool,
    toast: Option<(String, f64)>,
    window_title: String,
}

impl EmurustApp {
    pub fn new(ctx: &egui::Context, roms_dir: PathBuf, start_game: Option<PathBuf>) -> Self {
        theme::apply(ctx);
        let systems = systems::all();
        let library = Library::scan(&roms_dir, &systems);
        let audio = AudioOutput::open();
        let mut app = EmurustApp {
            game_index: vec![0; systems.len()],
            systems,
            library,
            audio,
            session: None,
            screen: Screen::Systems,
            system_index: 0,
            list_top: 0,
            wheel: 0.0,
            previews: Previews::new(),
            crt: true,
            toast: None,
            window_title: String::new(),
        };
        if app.audio.is_none() {
            app.notify(
                "Aucune sortie audio trouvée : les jeux tourneront sans son.",
                0.0,
            );
        }
        if let Some(path) = start_game {
            app.launch(&path, 0.0);
        }
        app
    }

    fn notify(&mut self, message: impl Into<String>, now: f64) {
        self.toast = Some((message.into(), now + TOAST_SECONDS));
    }

    fn system(&self) -> &System {
        &self.systems[self.system_index]
    }

    fn launch(&mut self, path: &Path, now: f64) {
        let Some(index) = systems::for_path(&self.systems, path) else {
            self.notify(format!("Aucun émulateur ne gère {}", path.display()), now);
            return;
        };
        let sample_rate = self.audio.as_ref().map(|out| out.sample_rate);
        match GameSession::start(&self.systems[index], path, sample_rate) {
            Ok(session) => {
                self.system_index = index;
                self.session = Some(session);
                self.screen = Screen::Playing;
            }
            Err(err) => self.notify(err, now),
        }
    }

    fn rescan(&mut self, now: f64) {
        self.library = Library::scan(&self.library.root, &self.systems);
        self.previews.clear();
        for (index, games) in self.game_index.iter_mut().zip(&self.library.games) {
            *index = (*index).min(games.len().saturating_sub(1));
        }
        let total = self.library.total();
        self.notify(format!("Bibliothèque actualisée : {total} jeu(x)"), now);
    }

    fn stop_game(&mut self) {
        self.session = None;
        self.screen = Screen::Games;
    }

    fn set_window_title(&mut self, ctx: &egui::Context) {
        let title = match &self.session {
            Some(session) => format!("emurust - {}", session.title),
            None => "emurust".to_string(),
        };
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }
    }

    // ------------------------------------------------------------------
    // Touches communes aux écrans de la bibliothèque
    // ------------------------------------------------------------------

    fn common_keys(&mut self, ui: &egui::Ui, now: f64) {
        if ui.input(|i| i.key_pressed(Key::F5)) {
            self.rescan(now);
        }
        if ui.input(|i| i.key_pressed(Key::F3)) {
            open_in_file_manager(&self.library.root);
        }
        if ui.input(|i| i.key_pressed(Key::F2)) {
            self.crt = !self.crt;
            let state = if self.crt { "activé" } else { "désactivé" };
            self.notify(format!("Effet écran cathodique {state}"), now);
        }
    }

    // ------------------------------------------------------------------
    // En-tête, bas de page et notifications
    // ------------------------------------------------------------------

    fn header(&self, painter: &egui::Painter, rect: Rect) {
        let left = Pos2::new(rect.left() + 28.0, rect.center().y);
        let logo = theme::neon_text(
            painter,
            left,
            Align2::LEFT_CENTER,
            "EMURUST",
            theme::mono(30.0),
            theme::NEON_CYAN,
        );

        let section = match self.screen {
            Screen::Systems => "CONSOLES".to_string(),
            _ => self.system().info.name.to_uppercase(),
        };
        painter.text(
            Pos2::new(logo.right() + 18.0, rect.center().y + 3.0),
            Align2::LEFT_CENTER,
            format!("▸ {section}"),
            theme::mono(16.0),
            theme::NEON_PINK,
        );
        painter.text(
            Pos2::new(rect.right() - 28.0, rect.center().y),
            Align2::RIGHT_CENTER,
            match self.library.total() {
                0 => "AUCUN JEU".to_string(),
                1 => "1 JEU".to_string(),
                n => format!("{n} JEUX"),
            },
            theme::mono(14.0),
            theme::TEXT_DIM,
        );
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            egui::Stroke::new(1.0, alpha(theme::NEON_CYAN, 0.2)),
        );
    }

    fn toast(&mut self, painter: &egui::Painter, rect: Rect, now: f64) {
        let Some((message, until)) = &self.toast else {
            return;
        };
        if now > *until {
            self.toast = None;
            return;
        }
        let galley = painter.layout_no_wrap(message.clone(), theme::prop(15.0), theme::TEXT);
        let size = galley.size() + Vec2::new(32.0, 18.0);
        let pos = Pos2::new(
            rect.center().x,
            rect.top() + HEADER_HEIGHT + 16.0 + size.y / 2.0,
        );
        let bubble = Rect::from_center_size(pos, size);
        theme::glow_panel(painter, bubble, theme::NEON_PINK, 0.6, 10);
        painter.galley(bubble.min + Vec2::new(16.0, 9.0), galley, theme::TEXT);
    }

    // ------------------------------------------------------------------
    // Écran 1 : carrousel des consoles
    // ------------------------------------------------------------------

    fn systems_ui(&mut self, ui: &mut egui::Ui, body: Rect, time: f64) {
        let count = self.systems.len();
        if ui.input(|i| i.key_pressed(Key::ArrowLeft)) {
            self.system_index = self.system_index.saturating_sub(1);
        }
        if ui.input(|i| i.key_pressed(Key::ArrowRight)) {
            self.system_index = (self.system_index + 1).min(count - 1);
        }
        let mut open = ui.input(|i| i.key_pressed(Key::Enter));

        let anim =
            ui.ctx()
                .animate_value_with_time(Id::unique("carousel"), self.system_index as f32, 0.3);
        let card_h = (body.height() * 0.8).max(260.0);
        let card_w = card_h * 0.72;
        let center = Pos2::new(body.center().x, body.center().y - 10.0);
        let painter = ui.painter().clone();

        for i in 0..count {
            let offset = i as f32 - anim;
            let distance = offset.abs().min(1.5);
            let scale = 1.0 - 0.18 * distance.min(1.0);
            let card_center = center + Vec2::new(offset * (card_w + 70.0), 0.0);
            let card = Rect::from_center_size(card_center, Vec2::new(card_w, card_h) * scale);
            if !card.intersects(body) {
                continue;
            }
            let system = &self.systems[i];
            let focus = (1.0 - distance).max(0.0);
            let fade = 1.0 - 0.55 * distance / 1.5;

            theme::glow_panel(&painter, card, alpha(system.accent, fade), focus, 18);

            let art_rect = Rect::from_min_max(
                card.min + Vec2::new(card.width() * 0.18, card.height() * 0.07),
                Pos2::new(
                    card.right() - card.width() * 0.18,
                    card.top() + card.height() * 0.66,
                ),
            );
            art::handheld(
                &painter,
                art_rect,
                alpha(system.accent, fade),
                alpha(system.screen, fade),
                time,
            );

            let name_y = card.top() + card.height() * 0.76;
            theme::neon_text(
                &painter,
                Pos2::new(card.center().x, name_y),
                Align2::CENTER_CENTER,
                &system.info.name.to_uppercase(),
                theme::mono(28.0 * scale),
                alpha(system.accent, fade),
            );
            painter.text(
                Pos2::new(card.center().x, name_y + 32.0 * scale),
                Align2::CENTER_CENTER,
                system.tagline,
                theme::prop(15.0 * scale),
                alpha(theme::TEXT_DIM, fade),
            );
            let games = self.library.games[i].len();
            let label = match games {
                0 => "AUCUN JEU".to_string(),
                1 => "1 JEU".to_string(),
                n => format!("{n} JEUX"),
            };
            painter.text(
                Pos2::new(card.center().x, name_y + 62.0 * scale),
                Align2::CENTER_CENTER,
                label,
                theme::mono(15.0 * scale),
                alpha(theme::NEON_PINK, fade),
            );

            let response = ui.interact(card, Id::unique(("system_card", i)), Sense::click());
            if response.clicked() {
                if i == self.system_index {
                    open = true;
                } else {
                    self.system_index = i;
                }
            }
        }

        if count > 1 {
            let pulse = 0.5 + 0.5 * (time * 3.0).sin() as f32;
            for (visible, glyph, x) in [
                (self.system_index > 0, "◀", body.left() + 40.0),
                (self.system_index + 1 < count, "▶", body.right() - 40.0),
            ] {
                if visible {
                    painter.text(
                        Pos2::new(x, center.y),
                        Align2::CENTER_CENTER,
                        glyph,
                        theme::mono(34.0),
                        alpha(theme::NEON_CYAN, 0.5 + 0.5 * pulse),
                    );
                }
            }
        }

        painter.text(
            Pos2::new(body.center().x, body.bottom() - 10.0),
            Align2::CENTER_BOTTOM,
            format!("Dossier des ROMs : {}", self.library.root.display()),
            theme::prop(13.0),
            theme::TEXT_DIM,
        );

        if open {
            self.screen = Screen::Games;
            self.list_top = 0;
        }
    }

    // ------------------------------------------------------------------
    // Écran 2 : liste des jeux, fiche et aperçu
    // ------------------------------------------------------------------

    fn games_ui(&mut self, ui: &mut egui::Ui, body: Rect, time: f64) {
        let sys = self.system_index;
        let count = self.library.games[sys].len();

        let margin = 32.0;
        let inner = body.shrink2(Vec2::new(margin, 16.0));
        let list_rect = Rect::from_min_max(
            inner.min,
            Pos2::new(inner.left() + inner.width() * 0.44, inner.bottom()),
        );
        let detail_rect =
            Rect::from_min_max(Pos2::new(list_rect.right() + 28.0, inner.top()), inner.max);
        let rows_rect = Rect::from_min_max(
            list_rect.min + Vec2::new(14.0, 58.0),
            list_rect.max - Vec2::new(14.0, 14.0),
        );
        let visible = ((rows_rect.height() / ROW_HEIGHT).floor() as usize).max(1);

        // --- Clavier et molette ---
        if ui.input(|i| i.key_pressed(Key::Escape) || i.key_pressed(Key::Backspace)) {
            self.screen = Screen::Systems;
            return;
        }
        let mut selected = self.game_index[sys];
        if count > 0 {
            let (up, down, page_up, page_down, home, end) = ui.input(|i| {
                (
                    i.key_pressed(Key::ArrowUp),
                    i.key_pressed(Key::ArrowDown),
                    i.key_pressed(Key::PageUp),
                    i.key_pressed(Key::PageDown),
                    i.key_pressed(Key::Home),
                    i.key_pressed(Key::End),
                )
            });
            if up {
                selected = selected.saturating_sub(1);
            }
            if down {
                selected = (selected + 1).min(count - 1);
            }
            if page_up {
                selected = selected.saturating_sub(visible);
            }
            if page_down {
                selected = (selected + visible).min(count - 1);
            }
            if home {
                selected = 0;
            }
            if end {
                selected = count - 1;
            }

            if ui.rect_contains_pointer(list_rect) {
                self.wheel += ui.input(|i| i.smooth_scroll_delta.y);
                while self.wheel >= WHEEL_STEP {
                    selected = selected.saturating_sub(1);
                    self.wheel -= WHEEL_STEP;
                }
                while self.wheel <= -WHEEL_STEP {
                    selected = (selected + 1).min(count - 1);
                    self.wheel += WHEEL_STEP;
                }
            }

            // Taper une lettre saute au premier jeu qui commence par elle.
            let typed: Vec<char> = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|e| match e {
                        Event::Text(t) => t.chars().next(),
                        _ => None,
                    })
                    .collect()
            });
            if let Some(&c) = typed.last() {
                let c = c.to_lowercase().next().unwrap_or(c);
                if let Some(pos) = self.library.games[sys]
                    .iter()
                    .position(|g| g.name.to_lowercase().starts_with(c))
                {
                    selected = pos;
                }
            }
        }

        // Garde la sélection visible.
        if selected < self.list_top {
            self.list_top = selected;
        }
        if selected >= self.list_top + visible {
            self.list_top = selected + 1 - visible;
        }

        let accent = self.systems[sys].accent;
        let painter = ui.painter().clone();
        theme::glow_panel(&painter, list_rect, accent, 0.25, 14);
        theme::glow_panel(&painter, detail_rect, accent, 0.25, 14);

        painter.text(
            list_rect.min + Vec2::new(22.0, 30.0),
            Align2::LEFT_CENTER,
            format!(
                "{}  ·  {count} jeu{}",
                self.systems[sys].info.name,
                if count > 1 { "x" } else { "" }
            ),
            theme::mono(16.0),
            accent,
        );

        let mut launch = ui.input(|i| i.key_pressed(Key::Enter)) && count > 0;

        if count == 0 {
            self.empty_library(&painter, list_rect, detail_rect);
        } else {
            // --- Liste ---
            let scroll = ui.ctx().animate_value_with_time(
                Id::unique(("list", sys)),
                self.list_top as f32,
                0.12,
            );
            let clip = painter.with_clip_rect(rows_rect);
            for (i, game) in self.library.games[sys].iter().enumerate() {
                let y = rows_rect.top() + (i as f32 - scroll) * ROW_HEIGHT;
                if y + ROW_HEIGHT < rows_rect.top() || y > rows_rect.bottom() {
                    continue;
                }
                let row = Rect::from_min_size(
                    Pos2::new(rows_rect.left(), y),
                    Vec2::new(rows_rect.width(), ROW_HEIGHT - 4.0),
                );
                let is_selected = i == selected;
                if is_selected {
                    clip.rect_filled(row, 8, alpha(accent, 0.16));
                    clip.rect_filled(
                        Rect::from_min_size(row.min, Vec2::new(4.0, row.height())),
                        2,
                        accent,
                    );
                }
                let name = fit_text(&clip, &game.name, row.width() - 48.0, theme::prop(17.0));
                if is_selected {
                    clip.text(
                        Pos2::new(row.left() + 16.0, row.center().y),
                        Align2::LEFT_CENTER,
                        "▸",
                        theme::mono(18.0),
                        accent,
                    );
                }
                clip.text(
                    Pos2::new(row.left() + 36.0, row.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    theme::prop(17.0),
                    if is_selected {
                        theme::TEXT
                    } else {
                        theme::TEXT_DIM
                    },
                );

                let response = ui.interact(
                    row.intersect(rows_rect),
                    Id::unique(("game_row", sys, i)),
                    Sense::click(),
                );
                if response.clicked() {
                    if is_selected {
                        launch = true;
                    } else {
                        selected = i;
                    }
                }
            }

            // Barre de défilement.
            if count > visible {
                let track = Rect::from_min_max(
                    Pos2::new(list_rect.right() - 8.0, rows_rect.top()),
                    Pos2::new(list_rect.right() - 4.0, rows_rect.bottom()),
                );
                painter.rect_filled(track, 2, alpha(accent, 0.12));
                let h = track.height() * visible as f32 / count as f32;
                let y = track.top() + (track.height() - h) * scroll / (count - visible) as f32;
                painter.rect_filled(
                    Rect::from_min_size(Pos2::new(track.left(), y), Vec2::new(4.0, h)),
                    2,
                    accent,
                );
            }

            // --- Fiche du jeu ---
            let game_path = self.library.games[sys][selected].path.clone();
            let game_name = self.library.games[sys][selected].name.clone();
            self.detail_panel(ui, &painter, detail_rect, &game_path, &game_name, time);
        }

        self.game_index[sys] = selected;
        if launch {
            let path = self.library.games[sys][selected].path.clone();
            self.launch(&path, time);
        }
    }

    fn empty_library(&self, painter: &egui::Painter, list_rect: Rect, detail_rect: Rect) {
        let system = self.system();
        let dir = self.library.system_dir(system);
        let lines = [
            "Aucun jeu pour le moment.".to_string(),
            String::new(),
            format!(
                "Copiez vos fichiers .{} dans :",
                system.info.extensions.join(", .")
            ),
            dir.display().to_string(),
            String::new(),
            "puis appuyez sur F5 pour actualiser.".to_string(),
            "F3 ouvre ce dossier dans l'explorateur.".to_string(),
        ];
        for (i, line) in lines.iter().enumerate() {
            painter.text(
                list_rect.min + Vec2::new(24.0, 80.0 + i as f32 * 24.0),
                Align2::LEFT_CENTER,
                fit_text(painter, line, list_rect.width() - 48.0, theme::prop(15.0)),
                theme::prop(15.0),
                if i == 3 {
                    theme::NEON_CYAN
                } else {
                    theme::TEXT_DIM
                },
            );
        }
        art::handheld(
            painter,
            detail_rect.shrink(60.0),
            alpha(system.accent, 0.35),
            alpha(system.screen, 0.25),
            0.0,
        );
    }

    fn detail_panel(
        &mut self,
        ui: &mut egui::Ui,
        painter: &egui::Painter,
        rect: Rect,
        path: &Path,
        name: &str,
        time: f64,
    ) {
        let system = &self.systems[self.system_index];
        let accent = system.accent;
        let (w, h) = (
            system.info.screen_width as f32,
            system.info.screen_height as f32,
        );

        // Écran d'aperçu, au plus grand facteur entier qui rentre.
        let max_w = rect.width() - 64.0;
        let max_h = rect.height() * 0.52;
        let fit = (max_w / w).min(max_h / h);
        let scale = if fit >= 1.0 { fit.floor() } else { fit };
        let screen = Rect::from_center_size(
            Pos2::new(rect.center().x, rect.top() + 28.0 + h * scale / 2.0 + 10.0),
            Vec2::new(w, h) * scale,
        );
        let bezel = screen.expand(12.0);
        painter.rect_filled(bezel, 10, Color32::from_rgb(8, 6, 20));
        painter.rect_stroke(
            bezel,
            10,
            egui::Stroke::new(1.5, alpha(accent, 0.6)),
            egui::StrokeKind::Inside,
        );

        let preview = self.previews.get(system, path);
        let (details, error) = match preview {
            Preview::Pending => {
                let dots = ".".repeat(((time * 3.0) as usize % 3) + 1);
                painter.text(
                    screen.center(),
                    Align2::CENTER_CENTER,
                    format!("CHARGEMENT{dots}"),
                    theme::mono(16.0),
                    theme::TEXT_DIM,
                );
                (Vec::new(), None)
            }
            Preview::Ready {
                details,
                image,
                error,
            } => {
                match image {
                    Some(texture) => {
                        ui.put(
                            screen,
                            egui::Image::from_texture(texture).fit_to_exact_size(screen.size()),
                        );
                        if self.crt && scale >= 2.0 {
                            theme::scanlines(painter, screen, scale, 0.35);
                        }
                    }
                    None => {
                        painter.text(
                            screen.center(),
                            Align2::CENTER_CENTER,
                            "PAS D'APERÇU",
                            theme::mono(16.0),
                            theme::TEXT_DIM,
                        );
                    }
                }
                (details.clone(), error.clone())
            }
        };

        // Nom du jeu et infos de la cartouche.
        let mut y = bezel.bottom() + 34.0;
        let title_font = theme::mono(24.0);
        let title = fit_text(
            painter,
            &name.to_uppercase(),
            rect.width() - 56.0,
            title_font.clone(),
        );
        theme::neon_text(
            painter,
            Pos2::new(rect.left() + 28.0, y),
            Align2::LEFT_CENTER,
            &title,
            title_font,
            accent,
        );
        y += 36.0;

        for (key, value) in &details {
            if y > rect.bottom() - 60.0 {
                break;
            }
            painter.text(
                Pos2::new(rect.left() + 28.0, y),
                Align2::LEFT_CENTER,
                key,
                theme::prop(15.0),
                theme::TEXT_DIM,
            );
            painter.text(
                Pos2::new(rect.left() + 170.0, y),
                Align2::LEFT_CENTER,
                fit_text(painter, value, rect.width() - 200.0, theme::prop(15.0)),
                theme::prop(15.0),
                theme::TEXT,
            );
            y += 24.0;
        }
        if path.with_extension("sav").exists() {
            painter.text(
                Pos2::new(rect.left() + 28.0, y),
                Align2::LEFT_CENTER,
                "Partie",
                theme::prop(15.0),
                theme::TEXT_DIM,
            );
            painter.text(
                Pos2::new(rect.left() + 170.0, y),
                Align2::LEFT_CENTER,
                "sauvegarde trouvée",
                theme::prop(15.0),
                theme::TEXT,
            );
        }

        let bottom = Pos2::new(rect.center().x, rect.bottom() - 28.0);
        match error {
            Some(err) => {
                painter.text(
                    bottom,
                    Align2::CENTER_CENTER,
                    fit_text(painter, &err, rect.width() - 40.0, theme::prop(15.0)),
                    theme::prop(15.0),
                    theme::NEON_PINK,
                );
            }
            None => {
                let pulse = 0.55 + 0.45 * (time * 3.0).sin() as f32;
                theme::neon_text(
                    painter,
                    bottom,
                    Align2::CENTER_CENTER,
                    "▶  ENTRÉE POUR JOUER",
                    theme::mono(18.0),
                    alpha(theme::NEON_CYAN, pulse),
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // Écran 3 : partie en cours
    // ------------------------------------------------------------------

    fn playing_ui(&mut self, ui: &mut egui::Ui, rect: Rect) {
        if ui.input(|i| i.key_pressed(Key::Escape)) {
            self.stop_game();
            return;
        }
        if ui.input(|i| i.key_pressed(Key::F2)) {
            self.crt = !self.crt;
        }
        let footer = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 40.0), rect.max);
        let area = Rect::from_min_max(rect.min, Pos2::new(rect.right(), footer.top()));
        let audio = self.audio.as_ref();
        if let Some(session) = self.session.as_mut() {
            session.ui(ui, area, audio, self.crt);
        }

        let painter = ui.painter();
        painter.rect_filled(footer, 0.0, Color32::from_rgb(6, 4, 16));
        theme::footer(
            painter,
            footer,
            &[
                ("ÉCHAP", "Bibliothèque"),
                ("F2", if self.crt { "CRT : oui" } else { "CRT : non" }),
                ("FLÈCHES", "Croix"),
                ("X", "A"),
                ("Z", "B"),
                ("ENTRÉE", "Start"),
                ("RETOUR", "Select"),
            ],
        );
    }
}

impl eframe::App for EmurustApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Fermer la fenêtre pendant une partie écrit la sauvegarde.
        if ui.input(|i| i.viewport().close_requested()) {
            self.session = None;
        }
        self.previews.poll(&ctx);
        let rect = ui.max_rect();
        let now = ui.input(|i| i.time);

        if self.screen == Screen::Playing && self.session.is_some() {
            self.playing_ui(ui, rect);
        } else {
            if self.screen == Screen::Playing {
                self.screen = Screen::Games;
            }
            let painter = ui.painter().clone();
            theme::background(&painter, rect, now);

            let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), HEADER_HEIGHT));
            let footer = Rect::from_min_max(
                Pos2::new(rect.left(), rect.bottom() - FOOTER_HEIGHT),
                rect.max,
            );
            let body = Rect::from_min_max(
                Pos2::new(rect.left(), header.bottom()),
                Pos2::new(rect.right(), footer.top()),
            );

            self.common_keys(ui, now);
            self.header(&painter, header);
            match self.screen {
                Screen::Systems => {
                    self.systems_ui(ui, body, now);
                    theme::footer(
                        &painter,
                        footer,
                        &[
                            ("◀ ▶", "Choisir"),
                            ("ENTRÉE", "Ouvrir"),
                            ("F5", "Actualiser"),
                            ("F3", "Dossier ROMs"),
                            ("F2", "Effet CRT"),
                        ],
                    );
                }
                _ => {
                    self.games_ui(ui, body, now);
                    theme::footer(
                        &painter,
                        footer,
                        &[
                            ("▲ ▼", "Choisir"),
                            ("ENTRÉE", "Jouer"),
                            ("ÉCHAP", "Consoles"),
                            ("A-Z", "Aller à"),
                            ("F5", "Actualiser"),
                            ("F3", "Dossier ROMs"),
                        ],
                    );
                }
            }
            self.toast(&painter, rect, now);
            if self.crt {
                theme::scanlines(&painter, rect, 3.0, 0.10);
            }
            ctx.request_repaint();
        }
        self.set_window_title(&ctx);
    }
}

/// Raccourcit un texte avec « … » pour qu'il tienne dans `max_width`.
fn fit_text(painter: &egui::Painter, text: &str, max_width: f32, font: egui::FontId) -> String {
    let width = |s: &str| {
        painter
            .layout_no_wrap(s.to_owned(), font.clone(), Color32::WHITE)
            .size()
            .x
    };
    if width(text) <= max_width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if width(&candidate) <= max_width {
            return candidate;
        }
    }
    "…".to_owned()
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
