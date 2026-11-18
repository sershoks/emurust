//! Cœur d'émulation Game Boy (DMG).
//!
//! État actuel : CPU complet, bus mémoire, MBC1, timer, port série et écran (PPU).
//! Le son viendra ensuite.

pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod ppu;

use bus::GbBus;
use cartridge::{Cartridge, Mbc};
use cpu::Cpu;
use emu_core::{Button, EmuError, Emulator, SystemInfo};

pub const SCREEN_WIDTH: u32 = 160;
pub const SCREEN_HEIGHT: u32 = 144;

/// T-cycles par image : 154 lignes de 456 cycles.
pub const CYCLES_PER_FRAME: u64 = 70_224;

pub use ppu::DMG_PALETTE;

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
    cpu: Cpu,
    bus: Option<GbBus>,
    framebuffer: Vec<u32>,
    /// Un bit par bouton, 1 = appuyé.
    buttons: u8,
}

impl GameBoy {
    pub fn new() -> Self {
        GameBoy {
            cpu: Cpu::post_boot_dmg(),
            bus: None,
            framebuffer: vec![DMG_PALETTE[0]; (SCREEN_WIDTH * SCREEN_HEIGHT) as usize],
            buttons: 0,
        }
    }

    /// Cartouche actuellement chargée, s'il y en a une.
    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.bus.as_ref().map(|bus| &bus.cartridge)
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Tout ce que la ROM a envoyé sur le port série depuis le démarrage.
    pub fn serial_output(&self) -> &[u8] {
        self.bus.as_ref().map_or(&[], |bus| &bus.serial_output)
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
        let cartridge = Cartridge::from_bytes(data)?;
        match cartridge.header.mbc {
            Mbc::RomOnly | Mbc::Mbc1 => {}
            other => {
                return Err(EmuError::Unsupported(format!(
                    "contrôleur de cartouche {other:?} pas encore émulé"
                )));
            }
        }
        self.bus = Some(GbBus::new(cartridge));
        self.reset();
        Ok(())
    }

    fn reset(&mut self) {
        self.cpu = Cpu::post_boot_dmg();
        if let Some(bus) = self.bus.take() {
            self.bus = Some(GbBus::new(bus.cartridge));
        }
        self.framebuffer.fill(DMG_PALETTE[0]);
        self.buttons = 0;
    }

    fn run_frame(&mut self) {
        let Some(bus) = self.bus.as_mut() else {
            return;
        };
        bus.buttons = self.buttons;
        let target = self.cpu.cycles + CYCLES_PER_FRAME;
        while self.cpu.cycles < target {
            self.cpu.step(bus);
        }
    }

    fn framebuffer(&self) -> &[u32] {
        match &self.bus {
            Some(bus) => bus.ppu.frame(),
            None => &self.framebuffer,
        }
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

    /// Fausse ROM dont le programme démarre en 0x0150.
    fn rom_with_program(program: &[u8]) -> Vec<u8> {
        let mut rom = fake_rom("PROGRAM", 0x00);
        rom[0x0100..0x0103].copy_from_slice(&[0xC3, 0x50, 0x01]); // JP 0x0150
        rom[0x0150..0x0150 + program.len()].copy_from_slice(program);
        rom
    }

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
    fn unsupported_mbc_is_rejected() {
        let mut gb = GameBoy::new();
        let err = gb.load_rom(&fake_rom("MBC5", 0x19)).unwrap_err();
        assert!(matches!(err, EmuError::Unsupported(_)));
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

    #[test]
    fn program_writes_to_serial() {
        #[rustfmt::skip]
        let program = [
            0x3E, b'O', 0xE0, 0x01, 0x3E, 0x81, 0xE0, 0x02, // envoie 'O'
            0x3E, b'K', 0xE0, 0x01, 0x3E, 0x81, 0xE0, 0x02, // envoie 'K'
            0x18, 0xFE,                                     // boucle infinie
        ];
        let mut gb = GameBoy::new();
        gb.load_rom(&rom_with_program(&program)).unwrap();
        gb.run_frame();
        assert_eq!(gb.serial_output(), b"OK");
    }
}
