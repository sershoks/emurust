//! Une partie en cours : l'émulateur, son image, son son et sa sauvegarde.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, ColorImage, Key, TextureHandle, TextureOptions};
use emu_core::{Button, Emulator};

use crate::audio::AudioOutput;
use crate::save::SaveFile;
use crate::systems::System;
use crate::theme;

/// Touches du clavier → boutons de la console.
pub const KEYMAP: [(Key, Button); 8] = [
    (Key::ArrowUp, Button::Up),
    (Key::ArrowDown, Button::Down),
    (Key::ArrowLeft, Button::Left),
    (Key::ArrowRight, Button::Right),
    (Key::X, Button::A),
    (Key::Z, Button::B),
    (Key::Enter, Button::Start),
    (Key::Backspace, Button::Select),
];

/// Une sauvegarde automatique toutes les 5 secondes environ.
const AUTOSAVE_FRAMES: u32 = 300;
/// Au plus 4 images émulées par image affichée, pour rattraper un retard sans s'emballer.
const MAX_FRAMES_PER_UPDATE: u32 = 4;

/// Nombre d'emplacements de sauvegarde instantanée par jeu.
pub const STATE_SLOTS: u8 = 4;

pub struct GameSession {
    pub title: String,
    rom_path: PathBuf,
    /// L'image doit être renvoyée à l'écran (après un chargement d'état).
    frame_stale: bool,
    emu: Box<dyn Emulator>,
    save: SaveFile,
    texture: Option<TextureHandle>,
    pixels: Vec<u8>,
    frames: u32,
    /// Temps accumulé quand il n'y a pas de carte son pour donner le rythme.
    time_budget: f64,
    /// En pause, le jeu ne tourne plus mais reste affiché.
    pub paused: bool,
    /// Son coupé : le jeu tourne toujours au même rythme, mais en silence.
    pub muted: bool,
}

impl GameSession {
    pub fn start(system: &System, path: &Path, sample_rate: Option<u32>) -> Result<Self, String> {
        let data = std::fs::read(path)
            .map_err(|err| format!("Impossible de lire {} : {err}", path.display()))?;
        let mut emu = (system.create)();
        emu.load_rom(&data).map_err(|err| err.to_string())?;
        if let Some(rate) = sample_rate {
            emu.set_sample_rate(rate);
        }

        let mut save = SaveFile::new(path.with_extension("sav"));
        save.load(emu.as_mut());

        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        Ok(GameSession {
            title: format!("{} - {}", system.info.name, name),
            rom_path: path.to_path_buf(),
            frame_stale: false,
            emu,
            save,
            texture: None,
            pixels: Vec::new(),
            frames: 0,
            time_budget: 0.0,
            paused: false,
            muted: false,
        })
    }

    /// Fait avancer le jeu au bon rythme, puis le dessine dans `area`.
    /// Avec `crt`, l'image reçoit des scanlines façon écran cathodique.
    /// `pad` liste les boutons maintenus sur les manettes.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        area: egui::Rect,
        audio: Option<&AudioOutput>,
        crt: bool,
        pad: &[Button],
    ) {
        for (key, button) in KEYMAP {
            let down = !self.paused && (ui.input(|i| i.key_down(key)) || pad.contains(&button));
            self.emu.set_button(button, down);
        }

        let ran = match audio {
            _ if self.paused => {
                self.time_budget = 0.0;
                0
            }
            // La carte son donne le rythme : on garde environ 50 ms de son d'avance.
            Some(out) => {
                let target = out.sample_rate as usize / 20;
                let mut ran = 0;
                while out.queued_frames() < target && ran < MAX_FRAMES_PER_UPDATE {
                    self.run_frame(Some(out));
                    ran += 1;
                }
                ran
            }
            None => {
                let frame_time = 1.0 / self.emu.info().frame_rate;
                self.time_budget += ui.input(|i| i.stable_dt as f64).min(0.1);
                let mut ran = 0;
                while self.time_budget >= frame_time && ran < MAX_FRAMES_PER_UPDATE {
                    self.run_frame(None);
                    self.time_budget -= frame_time;
                    ran += 1;
                }
                ran
            }
        };
        if ran > 0 || self.texture.is_none() || self.frame_stale {
            self.frame_stale = false;
            self.upload_frame(ui.ctx());
        }

        self.draw(ui, area, crt);
        ui.ctx().request_repaint();
    }

    fn run_frame(&mut self, audio: Option<&AudioOutput>) {
        self.emu.run_frame();
        let mut samples = Vec::new();
        self.emu.drain_audio(&mut samples);
        if self.muted {
            samples.fill(0.0);
        }
        if let Some(out) = audio {
            out.push(&samples);
        }
        self.frames += 1;
        if self.frames.is_multiple_of(AUTOSAVE_FRAMES) {
            self.save.write(self.emu.as_ref());
        }
    }

    fn upload_frame(&mut self, ctx: &egui::Context) {
        let info = self.emu.info();
        let size = [info.screen_width as usize, info.screen_height as usize];
        self.pixels.clear();
        for &p in self.emu.framebuffer() {
            self.pixels
                .extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
        }
        let image = ColorImage::from_rgb(size, &self.pixels);
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::NEAREST),
            None => self.texture = Some(ctx.load_texture("screen", image, TextureOptions::NEAREST)),
        }
    }

    /// Affiche l'écran au plus grand facteur entier qui rentre, centré sur fond noir.
    fn draw(&self, ui: &mut egui::Ui, rect: egui::Rect, crt: bool) {
        let Some(texture) = &self.texture else {
            return;
        };
        ui.painter().rect_filled(rect, 0.0, Color32::BLACK);

        let screen = texture.size_vec2();
        let fit = (rect.width() / screen.x).min(rect.height() / screen.y);
        let scale = if fit >= 1.0 { fit.floor() } else { fit };
        let target = egui::Rect::from_center_size(rect.center(), screen * scale);
        ui.put(
            target,
            egui::Image::from_texture(texture).fit_to_exact_size(target.size()),
        );
        if crt && scale >= 2.0 {
            // Une ligne sombre sous chaque ligne de pixels de la console.
            theme::scanlines(ui.painter(), target, scale, 0.35);
        }
    }

    /// Fichier d'un emplacement, à côté de la ROM : `Jeu.slot1.state`.
    pub fn state_path(&self, slot: u8) -> PathBuf {
        self.rom_path.with_extension(format!("slot{slot}.state"))
    }

    pub fn has_state(&self, slot: u8) -> bool {
        self.state_path(slot).is_file()
    }

    /// Sauvegarde instantanée dans un emplacement.
    pub fn save_state(&mut self, slot: u8) -> Result<(), String> {
        let data = self
            .emu
            .save_state()
            .ok_or("Ce jeu ne prend pas en charge les sauvegardes instantanées")?;
        let path = self.state_path(slot);
        let tmp = path.with_extension("state.tmp");
        std::fs::write(&tmp, &data)
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|err| format!("Impossible d'écrire {} : {err}", path.display()))
    }

    /// Recharge un emplacement. En cas d'échec, la partie continue telle quelle.
    pub fn load_state(&mut self, slot: u8) -> Result<(), String> {
        let path = self.state_path(slot);
        let data = std::fs::read(&path).map_err(|_| format!("L'emplacement {slot} est vide"))?;
        self.emu.load_state(&data).map_err(|err| err.to_string())?;
        self.time_budget = 0.0;
        self.frame_stale = true;
        Ok(())
    }

    /// Redémarre le jeu depuis le début. La sauvegarde de la cartouche est conservée.
    pub fn reset(&mut self) {
        self.save.write(self.emu.as_ref());
        self.emu.reset();
        self.time_budget = 0.0;
    }

    /// Écrit la sauvegarde (appelé en quittant la partie).
    pub fn close(&mut self) {
        self.save.write(self.emu.as_ref());
    }
}

impl Drop for GameSession {
    fn drop(&mut self) {
        self.close();
    }
}
