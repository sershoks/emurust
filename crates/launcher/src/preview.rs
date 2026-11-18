//! Aperçus des jeux : chaque ROM est lancée quelques secondes en arrière-plan,
//! sans son ni affichage, et sa dernière image sert de vignette.
//! Rien n'est stocké sur le disque.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use emu_core::Emulator;

use crate::systems::System;

/// ≈ 6 secondes de jeu : en général on arrive sur l'écran titre.
const PREVIEW_FRAMES: u32 = 360;

pub enum Preview {
    Pending,
    Ready {
        details: Vec<(String, String)>,
        image: Option<TextureHandle>,
        /// Pourquoi le jeu ne peut pas être lancé, le cas échéant.
        error: Option<String>,
    },
}

struct Rendered {
    details: Vec<(String, String)>,
    frame: Option<ColorImage>,
    error: Option<String>,
}

pub struct Previews {
    cache: HashMap<PathBuf, Preview>,
    tx: Sender<(PathBuf, Rendered)>,
    rx: Receiver<(PathBuf, Rendered)>,
}

impl Previews {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Previews {
            cache: HashMap::new(),
            tx,
            rx,
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Aperçu d'un jeu ; lance sa génération en arrière-plan s'il n'existe pas encore.
    pub fn get(&mut self, system: &System, path: &Path) -> &Preview {
        if !self.cache.contains_key(path) {
            self.cache.insert(path.to_path_buf(), Preview::Pending);
            let tx = self.tx.clone();
            let create = system.create;
            let details = system.details;
            let path = path.to_path_buf();
            std::thread::spawn(move || {
                let rendered = render(create, details, &path);
                let _ = tx.send((path, rendered));
            });
        }
        &self.cache[path]
    }

    /// Récupère les aperçus terminés et les transforme en textures.
    pub fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((path, rendered)) = self.rx.try_recv() {
            let image = rendered.frame.map(|frame| {
                ctx.load_texture(path.display().to_string(), frame, TextureOptions::NEAREST)
            });
            self.cache.insert(
                path,
                Preview::Ready {
                    details: rendered.details,
                    image,
                    error: rendered.error,
                },
            );
        }
    }
}

fn render(
    create: fn() -> Box<dyn Emulator>,
    details: fn(&[u8]) -> Vec<(String, String)>,
    path: &Path,
) -> Rendered {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) => {
            return Rendered {
                details: Vec::new(),
                frame: None,
                error: Some(format!("Lecture impossible : {err}")),
            };
        }
    };
    let details = details(&data);

    let mut emu = create();
    if let Err(err) = emu.load_rom(&data) {
        return Rendered {
            details,
            frame: None,
            error: Some(err.to_string()),
        };
    }
    for _ in 0..PREVIEW_FRAMES {
        emu.run_frame();
    }

    let info = emu.info();
    let mut rgb = Vec::with_capacity(emu.framebuffer().len() * 3);
    for &p in emu.framebuffer() {
        rgb.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]);
    }
    let size = [info.screen_width as usize, info.screen_height as usize];
    Rendered {
        details,
        frame: Some(ColorImage::from_rgb(size, &rgb)),
        error: None,
    }
}
