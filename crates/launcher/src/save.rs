//! Fichier de sauvegarde (`.sav`) placé à côté de la ROM.

use std::path::PathBuf;

use emu_core::Emulator;

pub struct SaveFile {
    path: PathBuf,
    /// Dernier contenu écrit, pour ne pas réécrire le disque pour rien.
    last_written: Option<Vec<u8>>,
}

impl SaveFile {
    pub fn new(path: PathBuf) -> Self {
        SaveFile {
            path,
            last_written: None,
        }
    }

    /// Charge la sauvegarde si elle existe.
    pub fn load(&mut self, emu: &mut dyn Emulator) {
        let Ok(data) = std::fs::read(&self.path) else {
            return;
        };
        match emu.load_save_data(&data) {
            Ok(()) => {
                println!("Sauvegarde chargée : {}", self.path.display());
                self.last_written = Some(data);
            }
            Err(err) => eprintln!("{} : {err}", self.path.display()),
        }
    }

    /// Écrit la sauvegarde si elle a changé. On écrit d'abord dans un fichier
    /// temporaire puis on le renomme : une coupure ne peut pas corrompre l'ancienne.
    pub fn write(&mut self, emu: &dyn Emulator) {
        let Some(data) = emu.save_data() else {
            return;
        };
        if self.last_written.as_ref() == Some(&data) {
            return;
        }
        let tmp = self.path.with_extension("sav.tmp");
        let result = std::fs::write(&tmp, &data).and_then(|()| std::fs::rename(&tmp, &self.path));
        match result {
            Ok(()) => self.last_written = Some(data),
            Err(err) => eprintln!("Impossible d'écrire {} : {err}", self.path.display()),
        }
    }
}
