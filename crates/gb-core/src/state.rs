//! Sauvegardes instantanées (save states) : une photo complète de la machine
//! (CPU, mémoire, écran, son, cartouche) qu'on peut recharger à tout moment.
//!
//! Format : `EMURGBST`, une version sur 2 octets, puis l'état encodé avec
//! `postcard`. La ROM elle-même n'est jamais incluse.

use emu_core::EmuError;
use serde::{Deserialize, Serialize};

use crate::bus::GbBus;
use crate::cpu::Cpu;

const MAGIC: &[u8; 8] = b"EMURGBST";
/// À augmenter dès que le contenu de l'état change de forme.
const VERSION: u16 = 1;
const PREFIX_LEN: usize = MAGIC.len() + 2;

/// Identifie le jeu : titre de l'en-tête et sommes de contrôle (0x014D-0x014F).
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
struct GameId {
    title: String,
    checksums: [u8; 3],
}

impl GameId {
    fn of(bus: &GbBus) -> Self {
        let rom = &bus.cartridge.rom;
        let byte = |i: usize| rom.get(i).copied().unwrap_or(0);
        GameId {
            title: bus.cartridge.header.title.clone(),
            checksums: [byte(0x014D), byte(0x014E), byte(0x014F)],
        }
    }
}

#[derive(Serialize)]
struct StateRef<'a> {
    game: GameId,
    cpu: &'a Cpu,
    bus: &'a GbBus,
}

#[derive(Deserialize)]
struct State {
    game: GameId,
    cpu: Cpu,
    bus: GbBus,
}

pub fn encode(cpu: &Cpu, bus: &GbBus) -> Vec<u8> {
    let state = StateRef {
        game: GameId::of(bus),
        cpu,
        bus,
    };
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&VERSION.to_le_bytes());
    // L'encodage vers un Vec ne peut échouer que par manque de mémoire.
    out.extend(postcard::to_allocvec(&state).expect("encodage de l'état"));
    out
}

/// Décode un état et vérifie qu'il appartient bien au jeu chargé dans `current`.
/// La cartouche renvoyée n'a pas de ROM : à l'appelant de remettre la sienne.
pub fn decode(data: &[u8], current: &GbBus) -> Result<(Cpu, GbBus), EmuError> {
    if data.len() < PREFIX_LEN || &data[..MAGIC.len()] != MAGIC {
        return Err(EmuError::InvalidSave(
            "ce fichier n'est pas une sauvegarde instantanée Game Boy".into(),
        ));
    }
    let version = u16::from_le_bytes([data[8], data[9]]);
    if version != VERSION {
        return Err(EmuError::InvalidSave(format!(
            "version {version} non prise en charge (attendue : {VERSION})"
        )));
    }
    let state: State = postcard::from_bytes(&data[PREFIX_LEN..])
        .map_err(|err| EmuError::InvalidSave(format!("fichier abîmé ({err})")))?;
    if state.game != GameId::of(current) {
        return Err(EmuError::InvalidSave(format!(
            "cet état appartient à un autre jeu ({})",
            state.game.title
        )));
    }
    Ok((state.cpu, state.bus))
}
