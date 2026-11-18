//! Launcher d'emurust.
//!
//! Il affiche tous les émulateurs, avec sous chacun les jeux trouvés dans
//! le dossier `roms/<console>/`. Un clic sur un jeu le lance.
//!
//! ```text
//! emurust                 ouvre la bibliothèque
//! emurust jeu.gb          lance directement ce jeu
//! emurust --roms <dir>    utilise un autre dossier de ROMs
//! ```

// Pas de console noire derrière la fenêtre sous Windows en version finale.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod game;
mod library;
mod save;
mod systems;

use std::path::PathBuf;
use std::process::ExitCode;

use eframe::egui;

/// Dossier des ROMs par défaut : `roms` dans le dossier courant.
/// En double-cliquant sur `emurust.exe`, c'est le dossier de l'exécutable.
fn default_roms_dir() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("roms")
}

fn main() -> ExitCode {
    let mut roms_dir = default_roms_dir();
    let mut start_game = None;

    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--roms" {
            match args.next() {
                Some(dir) => roms_dir = PathBuf::from(dir),
                None => {
                    eprintln!("--roms attend un dossier");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            start_game = Some(PathBuf::from(arg));
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("emurust")
            .with_inner_size([960.0, 720.0])
            .with_min_inner_size([480.0, 360.0]),
        ..Default::default()
    };

    let result = eframe::run_native(
        "emurust",
        options,
        Box::new(move |_cc| Ok(Box::new(app::EmurustApp::new(roms_dir, start_game)))),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Erreur de fenêtre : {err}");
            ExitCode::FAILURE
        }
    }
}
