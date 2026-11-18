//! Launcher d'emurust.
//!
//! Sans argument, il liste les systèmes disponibles.
//! Avec un fichier ROM, il choisit l'émulateur et ouvre une fenêtre de jeu.

mod audio;
mod save;

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use audio::AudioOutput;
use save::SaveFile;

use emu_core::{Button, Emulator, SystemInfo};
use gb_core::GameBoy;
use minifb::{Key, Scale, Window, WindowOptions};

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

    let mut save = SaveFile::new(path.with_extension("sav"));
    save.load(emu.as_mut());

    let title = format!(
        "emurust - {} - {}",
        emu.info().name,
        path.file_stem().unwrap_or_default().to_string_lossy()
    );
    let result = run_window(emu.as_mut(), &title, &mut save);
    // Quoi qu'il arrive à la fenêtre, on écrit la sauvegarde avant de quitter.
    save.write(emu.as_ref());
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Erreur de fenêtre : {err}");
            ExitCode::FAILURE
        }
    }
}

/// Une sauvegarde automatique toutes les 5 secondes environ.
const AUTOSAVE_FRAMES: u32 = 300;

/// Touches du clavier → boutons de la console.
const KEYMAP: [(Key, Button); 8] = [
    (Key::Up, Button::Up),
    (Key::Down, Button::Down),
    (Key::Left, Button::Left),
    (Key::Right, Button::Right),
    (Key::X, Button::A),
    (Key::Z, Button::B),
    (Key::Enter, Button::Start),
    (Key::Backspace, Button::Select),
];

fn run_window(
    emu: &mut dyn Emulator,
    title: &str,
    save: &mut SaveFile,
) -> Result<(), minifb::Error> {
    let info = emu.info();
    let (width, height) = (info.screen_width as usize, info.screen_height as usize);

    let mut window = Window::new(
        title,
        width,
        height,
        WindowOptions {
            scale: Scale::X4,
            resize: true,
            ..WindowOptions::default()
        },
    )?;
    let audio = AudioOutput::open();
    match &audio {
        Some(out) => {
            emu.set_sample_rate(out.sample_rate);
            // C'est la carte son qui donne le rythme : pas de limite côté fenêtre.
            window.set_target_fps(0);
        }
        None => {
            eprintln!("Aucune sortie audio trouvée : le jeu tournera sans son.");
            window.set_target_fps(info.frame_rate.round() as usize);
        }
    }
    // On garde environ 50 ms de son d'avance : assez pour éviter les coupures,
    // assez peu pour que le son reste synchronisé avec l'image.
    let max_queued = audio
        .as_ref()
        .map_or(0, |out| out.sample_rate as usize / 20);

    println!(
        "Flèches : croix | X : A | Z : B | Entrée : Start | Retour arrière : Select | Échap : quitter"
    );

    let mut samples = Vec::new();
    let mut frames = 0u32;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if let Some(out) = &audio {
            while out.queued_frames() > max_queued {
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        for (key, button) in KEYMAP {
            emu.set_button(button, window.is_key_down(key));
        }
        emu.run_frame();

        samples.clear();
        emu.drain_audio(&mut samples);
        if let Some(out) = &audio {
            out.push(&samples);
        }

        frames += 1;
        if frames.is_multiple_of(AUTOSAVE_FRAMES) {
            save.write(emu);
        }

        window.update_with_buffer(emu.framebuffer(), width, height)?;
    }
    Ok(())
}
