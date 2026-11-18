//! Cœur d'émulation Game Boy (DMG).
//!
//! État actuel : lecture de l'en-tête de cartouche et squelette de la machine.
//! CPU, mémoire, PPU, timers et son viendront étape par étape.

pub mod cartridge;

use cartridge::Cartridge;
use emu_core::{Button, EmuError, Emulator, SystemInfo};

pub const SCREEN_WIDTH: u32 = 160;
pub const SCREEN_HEIGHT: u32 = 144;

/// Palette verte de 4 teintes, du plus clair au plus foncé (format 0x00RRGGBB).
pub const DMG_PALETTE: [u32; 4] = [0x00E0_F8D0, 0x0088_C070, 0x0034_6856, 0x0008_1820];

const INFO: SystemInfo = SystemInfo {
    id: "gb",
    name: "Game Boy",
    screen_width: SCREEN_WIDTH,
    screen_height: SCREEN_HEIGHT,
    // 4 194 304 Hz / 70 224 cycles par image
    frame_rate: 59.727_5,
    extensions: &["gb"],
};

/// La machine Game Boy complète.
pub struct GameBoy {
    cartridge: Option<Cartridge>,
    framebuffer: Vec<u32>,
    /// Un bit par bouton, 1 = appuyé.
    buttons: u8,
}

impl GameBoy {
    pub fn new() -> Self {
        GameBoy {
            cartridge: None,
            framebuffer: vec![DMG_PALETTE[0]; (SCREEN_WIDTH * SCREEN_HEIGHT) as usize],
            buttons: 0,
        }
    }

    /// Cartouche actuellement chargée, s'il y en a une.
    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    fn button_bit(button: Button) -> u8 {
        match button {
            Button::Right => 0,
            Button::Left => 1,
            Button::Up => 2,
            Button::Down => 3,
            Button::A => 4,
            Button::B => 5,
            Button::Select => 6,
            Button::Start => 7,
        }
    }
}

impl Default for GameBoy {
    fn default() -> Self {
        Self::new()
    }
}

impl Emulator for GameBoy {
    fn info(&self) -> SystemInfo {
        INFO
    }

    fn load_rom(&mut self, data: &[u8]) -> Result<(), EmuError> {
        self.cartridge = Some(Cartridge::from_bytes(data)?);
        self.reset();
        Ok(())
    }

    fn reset(&mut self) {
        self.framebuffer.fill(DMG_PALETTE[0]);
        self.buttons = 0;
    }

    fn run_frame(&mut self) {
        // Le CPU n'est pas encore émulé : rien ne se passe pour l'instant.
    }

    fn framebuffer(&self) -> &[u32] {
        &self.framebuffer
    }

    fn set_button(&mut self, button: Button, pressed: bool) {
        let mask = 1 << Self::button_bit(button);
        if pressed {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cartridge::tests::fake_rom;

    #[test]
    fn framebuffer_has_screen_size() {
        let gb = GameBoy::new();
        assert_eq!(gb.framebuffer().len(), 160 * 144);
    }

    #[test]
    fn load_rom_keeps_cartridge() {
        let mut gb = GameBoy::new();
        gb.load_rom(&fake_rom("HELLO", 0x00)).unwrap();
        assert_eq!(gb.cartridge().unwrap().header.title, "HELLO");
    }

    #[test]
    fn buttons_toggle() {
        let mut gb = GameBoy::new();
        gb.set_button(Button::Start, true);
        gb.set_button(Button::A, true);
        assert_eq!(gb.buttons, 0b1001_0000);
        gb.set_button(Button::Start, false);
        assert_eq!(gb.buttons, 0b0001_0000);
    }
}
