//! Cœur d'émulation Game Boy (DMG) et Game Boy Color (CGB).
//!
//! État actuel : CPU complet, bus mémoire, MBC1/MBC3/MBC5, timer, port série,
//! écran (PPU) et son (APU).

pub mod apu;
pub mod bus;
pub mod cartridge;
pub mod cpu;
pub mod ppu;
pub mod state;

use bus::GbBus;
use cartridge::{Cartridge, Mbc};
use cpu::Cpu;
use emu_core::{Button, EmuError, Emulator, SystemInfo};

pub const SCREEN_WIDTH: u32 = 160;
pub const SCREEN_HEIGHT: u32 = 144;

/// T-cycles par image : 154 lignes de 456 cycles.
pub const CYCLES_PER_FRAME: u64 = 70_224;

pub use ppu::DMG_PALETTE;

const INFO_DMG: SystemInfo = SystemInfo {
    id: "gb",
    name: "Game Boy",
    screen_width: SCREEN_WIDTH,
    screen_height: SCREEN_HEIGHT,
    // 4 194 304 Hz / 70 224 cycles par image
    frame_rate: 59.727_5,
    extensions: &["gb"],
};

const INFO_CGB: SystemInfo = SystemInfo {
    id: "gbc",
    name: "Game Boy Color",
    // Une Color lit aussi les anciennes cartouches.
    extensions: &["gbc", "gb"],
    ..INFO_DMG
};

/// La machine Game Boy complète.
pub struct GameBoy {
    cpu: Cpu,
    bus: Option<GbBus>,
    framebuffer: Vec<u32>,
    /// Un bit par bouton, 1 = appuyé.
    buttons: u8,
    sample_rate: u32,
    /// La machine émulée est une Game Boy Color.
    color_hardware: bool,
    /// Le jeu chargé tourne en mode Color (console Color + cartouche compatible).
    cgb_mode: bool,
}

impl GameBoy {
    /// Une Game Boy classique : tous les jeux tournent en 4 teintes de vert.
    pub fn new() -> Self {
        Self::with_hardware(false)
    }

    /// Une Game Boy Color : les jeux Color tournent en couleur, les anciens
    /// jeux en mode classique.
    pub fn color() -> Self {
        Self::with_hardware(true)
    }

    fn with_hardware(color_hardware: bool) -> Self {
        GameBoy {
            color_hardware,
            cgb_mode: false,
            cpu: Cpu::post_boot_dmg(),
            bus: None,
            framebuffer: vec![DMG_PALETTE[0]; (SCREEN_WIDTH * SCREEN_HEIGHT) as usize],
            buttons: 0,
            sample_rate: 48_000,
        }
    }

    /// Cartouche actuellement chargée, s'il y en a une.
    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.bus.as_ref().map(|bus| &bus.cartridge)
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Le jeu chargé tourne en mode Color.
    pub fn is_cgb_mode(&self) -> bool {
        self.cgb_mode
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
        if self.color_hardware {
            INFO_CGB
        } else {
            INFO_DMG
        }
    }

    fn load_rom(&mut self, data: &[u8]) -> Result<(), EmuError> {
        let cartridge = Cartridge::from_bytes(data)?;
        match cartridge.header.mbc {
            Mbc::RomOnly | Mbc::Mbc1 | Mbc::Mbc3 | Mbc::Mbc5 => {}
            other => {
                return Err(EmuError::Unsupported(format!(
                    "contrôleur de cartouche {other:?} pas encore émulé"
                )));
            }
        }
        if cartridge.header.cgb_only && !self.color_hardware {
            return Err(EmuError::Unsupported(
                "ce jeu ne fonctionne que sur Game Boy Color".into(),
            ));
        }
        self.cgb_mode = self.color_hardware && cartridge.header.cgb_support;
        self.bus = Some(GbBus::new(cartridge, self.cgb_mode));
        self.reset();
        Ok(())
    }

    fn reset(&mut self) {
        self.cpu = if self.cgb_mode {
            Cpu::post_boot_cgb()
        } else {
            Cpu::post_boot_dmg()
        };
        if let Some(bus) = self.bus.take() {
            let mut bus = GbBus::new(bus.cartridge, self.cgb_mode);
            bus.apu.set_sample_rate(self.sample_rate);
            self.bus = Some(bus);
        }
        self.framebuffer.fill(DMG_PALETTE[0]);
        self.buttons = 0;
    }

    fn run_frame(&mut self) {
        let Some(bus) = self.bus.as_mut() else {
            return;
        };
        bus.buttons = self.buttons;
        // On compte en temps réel : en double vitesse, le CPU fait deux fois
        // plus de cycles pendant la même image.
        let target = bus.clock + CYCLES_PER_FRAME;
        while bus.clock < target {
            self.cpu.step(bus);
        }
    }

    fn framebuffer(&self) -> &[u32] {
        match &self.bus {
            Some(bus) => bus.ppu.frame(),
            None => &self.framebuffer,
        }
    }

    fn save_data(&self) -> Option<Vec<u8>> {
        self.bus.as_ref()?.cartridge.save_data()
    }

    fn load_save_data(&mut self, data: &[u8]) -> Result<(), EmuError> {
        match self.bus.as_mut() {
            Some(bus) => bus.cartridge.load_save_data(data),
            None => Ok(()),
        }
    }

    fn save_state(&self) -> Option<Vec<u8>> {
        Some(state::encode(&self.cpu, self.bus.as_ref()?))
    }

    fn load_state(&mut self, data: &[u8]) -> Result<(), EmuError> {
        let Some(bus) = self.bus.as_mut() else {
            return Err(EmuError::InvalidSave("aucun jeu chargé".into()));
        };
        let (cpu, mut restored) = state::decode(data, bus)?;
        // La ROM ne fait pas partie de l'état : on garde celle déjà chargée.
        restored.cartridge.rom = std::mem::take(&mut bus.cartridge.rom);
        restored.apu.set_sample_rate(self.sample_rate);
        self.cpu = cpu;
        self.bus = Some(restored);
        Ok(())
    }

    fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
        if let Some(bus) = self.bus.as_mut() {
            bus.apu.set_sample_rate(rate);
        }
    }

    fn drain_audio(&mut self, out: &mut Vec<f32>) {
        if let Some(bus) = self.bus.as_mut() {
            bus.apu.drain_samples(out);
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
        let err = gb.load_rom(&fake_rom("MBC2", 0x05)).unwrap_err();
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

    /// INC A ; LD (0xC000),A ; JR -6 : A et la WRAM changent sans arrêt.
    const COUNTER: [u8; 6] = [0x3C, 0xEA, 0x00, 0xC0, 0x18, 0xFA];

    #[test]
    fn save_state_round_trip() {
        let mut gb = GameBoy::new();
        gb.load_rom(&rom_with_program(&COUNTER)).unwrap();
        gb.run_frame();
        let snapshot = gb.save_state().unwrap();
        let regs = gb.cpu().regs.clone();
        let cycles = gb.cpu().cycles;
        let screen = gb.framebuffer().to_vec();

        for _ in 0..3 {
            gb.run_frame();
        }
        assert_ne!(gb.cpu().cycles, cycles);

        gb.load_state(&snapshot).unwrap();
        assert_eq!(gb.cpu().regs, regs);
        assert_eq!(gb.cpu().cycles, cycles);
        assert_eq!(gb.framebuffer(), &screen[..]);
        // La ROM est toujours là : le jeu continue de tourner.
        gb.run_frame();
        assert!(gb.cpu().cycles > cycles);
        assert!(!gb.cpu().locked);
    }

    #[test]
    fn save_state_from_another_game_is_rejected() {
        let mut first = GameBoy::new();
        first.load_rom(&rom_with_program(&COUNTER)).unwrap();
        let snapshot = first.save_state().unwrap();

        let mut other_rom = rom_with_program(&COUNTER);
        other_rom[0x0134..0x0139].copy_from_slice(b"AUTRE");
        other_rom[0x014D] = cartridge::header_checksum(&other_rom);
        let mut second = GameBoy::new();
        second.load_rom(&other_rom).unwrap();
        assert!(matches!(
            second.load_state(&snapshot),
            Err(EmuError::InvalidSave(_))
        ));
    }

    #[test]
    fn garbage_save_state_is_rejected() {
        let mut gb = GameBoy::new();
        gb.load_rom(&rom_with_program(&COUNTER)).unwrap();
        assert!(gb.load_state(b"pas un etat").is_err());
        let mut truncated = gb.save_state().unwrap();
        truncated.truncate(truncated.len() / 2);
        assert!(gb.load_state(&truncated).is_err());
        // L'émulateur reste utilisable après un échec.
        gb.run_frame();
    }

    /// Fausse ROM Color (0x80 : compatible Color et classique).
    fn color_rom_with_program(program: &[u8]) -> Vec<u8> {
        let mut rom = rom_with_program(program);
        rom[0x0143] = 0x80;
        rom[0x014D] = cartridge::header_checksum(&rom);
        rom
    }

    #[test]
    fn color_console_runs_color_games_in_color_mode() {
        let mut gb = GameBoy::color();
        gb.load_rom(&color_rom_with_program(&[0x18, 0xFE])).unwrap();
        assert!(gb.is_cgb_mode());
        assert_eq!(gb.cpu().regs.a, 0x11, "le jeu doit reconnaître une Color");
        assert_eq!(gb.info().id, "gbc");

        // Un vieux jeu sur Color tourne en mode classique.
        gb.load_rom(&rom_with_program(&[0x18, 0xFE])).unwrap();
        assert!(!gb.is_cgb_mode());
        assert_eq!(gb.cpu().regs.a, 0x01);

        // Une Game Boy classique ignore le drapeau Color...
        let mut dmg = GameBoy::new();
        dmg.load_rom(&color_rom_with_program(&[0x18, 0xFE]))
            .unwrap();
        assert!(!dmg.is_cgb_mode());

        // ...mais refuse les jeux exclusivement Color.
        let mut exclusive = color_rom_with_program(&[0x18, 0xFE]);
        exclusive[0x0143] = 0xC0;
        exclusive[0x014D] = cartridge::header_checksum(&exclusive);
        assert!(matches!(
            dmg.load_rom(&exclusive),
            Err(EmuError::Unsupported(_))
        ));
        assert!(GameBoy::color().load_rom(&exclusive).is_ok());
    }

    #[test]
    fn stop_switches_to_double_speed() {
        #[rustfmt::skip]
        let program = [
            0x3E, 0x01, 0xE0, 0x4D, // LD A,1 ; LDH (KEY1),A
            0x10, 0x00,             // STOP
            0xF0, 0x4D,             // LDH A,(KEY1)
            0xEA, 0x00, 0xC0,       // LD (0xC000),A
            0x18, 0xFE,             // boucle infinie
        ];
        let mut gb = GameBoy::color();
        gb.load_rom(&color_rom_with_program(&program)).unwrap();
        gb.run_frame();
        let bus = gb.bus.as_mut().unwrap();
        assert!(bus.double_speed);
        assert_eq!(cpu::Bus::read(bus, 0xC000) & 0x80, 0x80);
        // En double vitesse, une image contient deux fois plus de cycles CPU.
        let before = gb.cpu().cycles;
        gb.run_frame();
        assert_eq!(gb.cpu().cycles - before, 2 * CYCLES_PER_FRAME);
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
