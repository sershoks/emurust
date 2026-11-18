//! Launcher d'emurust.
//!
//! Pour l'instant en ligne de commande : il liste les systèmes disponibles
//! ou ouvre une ROM et affiche les informations de sa cartouche.
//! L'interface graphique viendra quand le premier cœur affichera des images.

use std::path::Path;
use std::process::ExitCode;

use emu_core::{Emulator, SystemInfo};
use gb_core::GameBoy;

/// Tous les émulateurs connus du launcher.
fn emulators() -> Vec<Box<dyn Emulator>> {
    vec![Box::new(GameBoy::new())]
}

fn find_emulator(path: &Path) -> Option<Box<dyn Emulator>> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    emulators()
        .into_iter()
        .find(|emu| emu.info().extensions.contains(&ext.as_str()))
}

fn print_systems() {
    println!("emurust {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("Systèmes disponibles :");
    for emu in emulators() {
        let SystemInfo {
            id,
            name,
            extensions,
            ..
        } = emu.info();
        println!("  {id:<6} {name:<12} (.{})", extensions.join(", ."));
    }
    println!();
    println!("Utilisation : emurust <fichier ROM>");
}

fn main() -> ExitCode {
    let Some(arg) = std::env::args_os().nth(1) else {
        print_systems();
        return ExitCode::SUCCESS;
    };
    let path = Path::new(&arg);

    let Some(mut emu) = find_emulator(path) else {
        eprintln!(
            "Aucun émulateur ne gère ce type de fichier : {}",
            path.display()
        );
        return ExitCode::FAILURE;
    };

    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) => {
            eprintln!("Impossible de lire {} : {err}", path.display());
            return ExitCode::FAILURE;
        }
    };

    if let Err(err) = emu.load_rom(&data) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }

    let info = emu.info();
    println!("[{}] ROM chargée : {}", info.name, path.display());
    println!("Le cœur ne sait pas encore exécuter de code : c'est la prochaine étape.");
    ExitCode::SUCCESS
}
