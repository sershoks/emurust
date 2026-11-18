//! Bibliothèque de jeux : un sous-dossier par console dans le dossier `roms`.
//!
//! ```text
//! roms/
//!   gb/
//!     Mon jeu.gb
//!     Un autre jeu.gb
//! ```

use std::path::{Path, PathBuf};

use crate::systems::System;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    /// Nom affiché : le nom du fichier sans son extension.
    pub name: String,
    pub path: PathBuf,
}

pub struct Library {
    pub root: PathBuf,
    /// Jeux trouvés, dans le même ordre que la liste des systèmes.
    pub games: Vec<Vec<Game>>,
}

impl Library {
    /// Dossier d'un système dans la bibliothèque (ex. `roms/gb`).
    pub fn system_dir(&self, system: &System) -> PathBuf {
        self.root.join(system.info.id)
    }

    /// Parcourt le dossier `root` et crée les sous-dossiers manquants.
    pub fn scan(root: &Path, systems: &[System]) -> Self {
        let games = systems
            .iter()
            .map(|system| {
                let dir = root.join(system.info.id);
                let _ = std::fs::create_dir_all(&dir);
                scan_dir(&dir, system)
            })
            .collect();
        Library {
            root: root.to_path_buf(),
            games,
        }
    }

    pub fn total(&self) -> usize {
        self.games.iter().map(Vec::len).sum()
    }
}

fn scan_dir(dir: &Path, system: &System) -> Vec<Game> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut games: Vec<Game> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && system.accepts(path))
        .map(|path| Game {
            name: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path,
        })
        .collect();
    games.sort_by_key(|game| game.name.to_lowercase());
    games
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::systems;

    #[test]
    fn scan_creates_folders_and_lists_games() {
        let root = std::env::temp_dir().join(format!("emurust-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let systems = systems::all();

        let library = Library::scan(&root, &systems);
        assert!(root.join("gb").is_dir());
        assert_eq!(library.total(), 0);

        let gb = root.join("gb");
        std::fs::write(gb.join("zelda-like.gb"), b"").unwrap();
        std::fs::write(gb.join("Aventure.GB"), b"").unwrap();
        std::fs::write(gb.join("notes.txt"), b"").unwrap();
        std::fs::create_dir(gb.join("dossier.gb")).unwrap();

        let library = Library::scan(&root, &systems);
        let names: Vec<_> = library.games[0].iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Aventure", "zelda-like"]);

        let _ = std::fs::remove_dir_all(&root);
    }
}
