//! Registre de tous les émulateurs connus du launcher.
//!
//! Pour ajouter une console : implémenter `emu_core::Emulator` dans un
//! nouveau crate, puis ajouter une entrée dans [`all`].

use std::path::Path;

use eframe::egui::Color32;
use emu_core::{Emulator, SystemInfo};
use gb_core::GameBoy;
use gb_core::cartridge::{Cartridge, Mbc};

pub struct System {
    pub info: SystemInfo,
    /// Crée une machine neuve, prête à recevoir une ROM.
    pub create: fn() -> Box<dyn Emulator>,
    /// Infos affichées dans la fiche d'un jeu, lues dans la ROM.
    pub details: fn(&[u8]) -> Vec<(String, String)>,
    /// Couleur néon de la console dans le launcher.
    pub accent: Color32,
    /// Couleur de l'écran allumé sur l'illustration.
    pub screen: Color32,
    /// Fabricant et année, affichés sous le nom.
    pub tagline: &'static str,
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
        details: game_boy_details,
        accent: Color32::from_rgb(120, 255, 140),
        screen: Color32::from_rgb(150, 190, 60),
        tagline: "Console portable · 1989",
    }]
}

/// Indice du système qui sait lancer ce fichier.
pub fn for_path(systems: &[System], path: &Path) -> Option<usize> {
    systems.iter().position(|system| system.accepts(path))
}

fn size_label(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} Mio", bytes / (1024 * 1024))
    } else {
        format!("{} Kio", bytes / 1024)
    }
}

fn game_boy_details(data: &[u8]) -> Vec<(String, String)> {
    let Ok(cart) = Cartridge::from_bytes(data) else {
        return vec![("Fichier".into(), "en-tête illisible".into())];
    };
    let h = &cart.header;
    let mbc = match h.mbc {
        Mbc::RomOnly => "ROM seule".to_string(),
        Mbc::Unknown(code) => format!("inconnue ({code:#04X})"),
        other => format!("{other:?}").to_uppercase(),
    };
    let mut rows = vec![
        (
            "Titre interne".into(),
            if h.title.is_empty() {
                "—".into()
            } else {
                h.title.clone()
            },
        ),
        ("Cartouche".into(), mbc),
        ("Taille".into(), size_label(h.rom_size)),
    ];
    if h.ram_size > 0 {
        rows.push(("RAM".into(), size_label(h.ram_size)));
    }
    rows.push((
        "Sauvegarde".into(),
        match (h.battery, h.rtc) {
            (true, true) => "oui, avec horloge".into(),
            (true, false) => "oui".into(),
            _ => "non".into(),
        },
    ));
    if h.cgb_support {
        rows.push(("Couleur".into(), "compatible Game Boy Color".into()));
    }
    rows
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

    #[test]
    fn details_read_the_cartridge_header() {
        let mut rom = vec![0u8; 32 * 1024];
        rom[0x0134..0x0138].copy_from_slice(b"DEMO");
        rom[0x0147] = 0x13; // MBC3 + RAM + pile
        rom[0x0149] = 0x03;
        let rows = game_boy_details(&rom);
        let get = |k: &str| {
            rows.iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("Titre interne"), Some("DEMO"));
        assert_eq!(get("Cartouche"), Some("MBC3"));
        assert_eq!(get("RAM"), Some("32 Kio"));
        assert_eq!(get("Sauvegarde"), Some("oui"));
    }
}
