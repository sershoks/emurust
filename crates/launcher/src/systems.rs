//! Registre de tous les émulateurs connus du launcher.
//!
//! Pour ajouter une console : implémenter `emu_core::Emulator` dans un
//! nouveau crate, puis ajouter une ligne dans [`all`].

use std::path::Path;

use emu_core::{Emulator, SystemInfo};
use gb_core::GameBoy;

pub struct System {
    pub info: SystemInfo,
    /// Crée une machine neuve, prête à recevoir une ROM.
    pub create: fn() -> Box<dyn Emulator>,
}

impl System {
    /// Le fichier a-t-il une extension gérée par ce système ?
    pub fn accepts(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                self.info
                    .extensions
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(ext))
            })
    }
}

pub fn all() -> Vec<System> {
    vec![System {
        info: GameBoy::new().info(),
        create: || Box::new(GameBoy::new()),
    }]
}

/// Indice du système qui sait lancer ce fichier.
pub fn for_path(systems: &[System], path: &Path) -> Option<usize> {
    systems.iter().position(|system| system.accepts(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_boy_accepts_gb_files_in_any_case() {
        let systems = all();
        assert_eq!(for_path(&systems, Path::new("jeu.gb")), Some(0));
        assert_eq!(for_path(&systems, Path::new("JEU.GB")), Some(0));
        assert_eq!(for_path(&systems, Path::new("jeu.txt")), None);
        assert_eq!(for_path(&systems, Path::new("sans_extension")), None);
    }
}
