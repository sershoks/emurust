//! Bus mémoire de la Game Boy : relie le CPU à la cartouche, à la RAM
//! et aux registres matériels (timer, port série, manette, écran, son...).
//!
//! Carte mémoire :
//! - `0000-7FFF` ROM de cartouche
//! - `8000-9FFF` VRAM
//! - `A000-BFFF` RAM de cartouche
//! - `C000-DFFF` WRAM (`E000-FDFF` en est un miroir)
//! - `FE00-FE9F` OAM (sprites)
//! - `FF00-FF7F` registres matériels
//! - `FF80-FFFE` HRAM
//! - `FFFF`      IE (interruptions activées)

use serde::{Deserialize, Serialize};

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::cpu::Bus;
use crate::ppu::Ppu;

pub const INT_VBLANK: u8 = 0x01;
pub const INT_STAT: u8 = 0x02;
pub const INT_TIMER: u8 = 0x04;
pub const INT_SERIAL: u8 = 0x08;

#[derive(Serialize, Deserialize)]
pub struct GbBus {
    pub cartridge: Cartridge,
    wram: Vec<u8>,
    hram: Vec<u8>,
    /// Registres matériels pas encore émulés : simplement mémorisés.
    io: Vec<u8>,
    ie: u8,
    if_: u8,

    // Timer
    div: u16,
    tima: u8,
    tma: u8,
    tac: u8,

    pub ppu: Ppu,
    pub apu: Apu,

    /// Boutons appuyés, un bit par bouton (voir `GameBoy::button_bit`).
    pub buttons: u8,
    /// Octets envoyés sur le port série. Les ROMs de test y écrivent leurs résultats.
    pub serial_output: Vec<u8>,
}

impl GbBus {
    pub fn new(cartridge: Cartridge) -> Self {
        let mut io = vec![0u8; 0x80];
        // Valeurs laissées par la boot ROM.
        io[0x00] = 0x30; // JOYP : aucune sélection
        GbBus {
            cartridge,
            wram: vec![0; 0x2000],
            hram: vec![0; 0x7F],
            io,
            ie: 0,
            if_: 0x01,
            div: 0xABCC,
            tima: 0,
            tma: 0,
            tac: 0,
            ppu: Ppu::new(),
            apu: Apu::new(),
            buttons: 0,
            serial_output: Vec::new(),
        }
    }

    fn request_interrupt(&mut self, mask: u8) {
        self.if_ |= mask;
    }

    /// Le timer incrémente TIMA sur chaque front descendant du bit de DIV
    /// sélectionné par TAC, comme le vrai circuit.
    fn tick_timer(&mut self) {
        let bit = [9, 3, 5, 7][(self.tac & 0x03) as usize];
        for _ in 0..4 {
            let old = self.div;
            self.div = self.div.wrapping_add(1);
            let fell = (old >> bit) & 1 == 1 && (self.div >> bit) & 1 == 0;
            if self.tac & 0x04 != 0 && fell {
                let (v, overflow) = self.tima.overflowing_add(1);
                if overflow {
                    self.tima = self.tma;
                    self.request_interrupt(INT_TIMER);
                } else {
                    self.tima = v;
                }
            }
        }
    }

    /// Copie 160 octets depuis `XX00` vers l'OAM (les sprites).
    /// Sur la vraie console la copie dure 160 M-cycles ; ici elle est instantanée.
    fn oam_dma(&mut self, page: u8) {
        self.io[0x46] = page;
        let src = (page as u16) << 8;
        for i in 0..0xA0u16 {
            let byte = self.read(src + i);
            self.ppu.oam[i as usize] = byte;
        }
    }

    fn read_joypad(&self) -> u8 {
        let select = self.io[0x00] & 0x30;
        let mut low = 0x0F;
        if select & 0x10 == 0 {
            low &= !(self.buttons & 0x0F); // croix directionnelle
        }
        if select & 0x20 == 0 {
            low &= !(self.buttons >> 4); // A, B, Select, Start
        }
        0xC0 | select | low
    }

    fn read_io(&self, addr: u16) -> u8 {
        match addr {
            0xFF00 => self.read_joypad(),
            0xFF04 => (self.div >> 8) as u8,
            0xFF05 => self.tima,
            0xFF06 => self.tma,
            0xFF07 => self.tac | 0xF8,
            0xFF0F => self.if_ | 0xE0,
            0xFF10..=0xFF3F => self.apu.read(addr),
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.read_reg(addr),
            _ => self.io[(addr - 0xFF00) as usize],
        }
    }

    fn write_io(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF00 => self.io[0x00] = value & 0x30,
            0xFF02 if value & 0x81 == 0x81 => {
                // Transfert série instantané, sans partenaire branché.
                self.serial_output.push(self.io[0x01]);
                self.io[0x01] = 0xFF;
                self.io[0x02] = value & 0x7F;
                self.request_interrupt(INT_SERIAL);
            }
            0xFF04 => self.div = 0,
            0xFF05 => self.tima = value,
            0xFF06 => self.tma = value,
            0xFF07 => self.tac = value & 0x07,
            0xFF0F => self.if_ = value & 0x1F,
            0xFF10..=0xFF3F => self.apu.write(addr, value),
            0xFF46 => self.oam_dma(value),
            0xFF40..=0xFF45 | 0xFF47..=0xFF4B => self.ppu.write_reg(addr, value),
            _ => self.io[(addr - 0xFF00) as usize] = value,
        }
    }
}

impl Bus for GbBus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cartridge.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize],
            0xA000..=0xBFFF => self.cartridge.read_ram(addr),
            0xC000..=0xFDFF => self.wram[(addr as usize - 0xC000) & 0x1FFF],
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize],
            0xFEA0..=0xFEFF => 0xFF,
            0xFF00..=0xFF7F => self.read_io(addr),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize],
            0xFFFF => self.ie,
        }
    }

    fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x7FFF => self.cartridge.write_rom(addr, value),
            0x8000..=0x9FFF => self.ppu.vram[(addr - 0x8000) as usize] = value,
            0xA000..=0xBFFF => self.cartridge.write_ram(addr, value),
            0xC000..=0xFDFF => self.wram[(addr as usize - 0xC000) & 0x1FFF] = value,
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize] = value,
            0xFEA0..=0xFEFF => {}
            0xFF00..=0xFF7F => self.write_io(addr, value),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.ie = value,
        }
    }

    fn tick(&mut self) {
        self.tick_timer();
        self.cartridge.tick();
        self.apu.tick();
        let irq = self.ppu.tick();
        self.request_interrupt(irq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::tests::fake_rom;

    fn bus() -> GbBus {
        GbBus::new(Cartridge::from_bytes(&fake_rom("BUS", 0x00)).unwrap())
    }

    #[test]
    fn echo_ram_mirrors_wram() {
        let mut bus = bus();
        bus.write(0xC123, 0x42);
        assert_eq!(bus.read(0xE123), 0x42);
    }

    #[test]
    fn timer_counts_and_overflows() {
        let mut bus = bus();
        bus.write(0xFF04, 0); // remet DIV à zéro
        bus.write(0xFF07, 0x05); // timer actif, une incrémentation tous les 16 T-cycles
        for _ in 0..4 {
            bus.tick();
        }
        assert_eq!(bus.read(0xFF05), 1);

        bus.write(0xFF05, 0xFF);
        bus.write(0xFF06, 0x80);
        bus.write(0xFF0F, 0);
        for _ in 0..4 {
            bus.tick();
        }
        assert_eq!(bus.read(0xFF05), 0x80);
        assert_eq!(bus.read(0xFF0F) & INT_TIMER, INT_TIMER);
    }

    #[test]
    fn oam_dma_copies_to_sprites() {
        let mut bus = bus();
        for i in 0..0xA0u16 {
            bus.write(0xC100 + i, i as u8);
        }
        bus.write(0xFF46, 0xC1);
        assert_eq!(bus.read(0xFE00), 0x00);
        assert_eq!(bus.read(0xFE9F), 0x9F);
    }

    #[test]
    fn joypad_reports_pressed_buttons() {
        let mut bus = bus();
        bus.buttons = 0b1000_0001; // Start + Droite
        bus.write(0xFF00, 0x20); // sélectionne la croix
        assert_eq!(bus.read(0xFF00) & 0x0F, 0b1110);
        bus.write(0xFF00, 0x10); // sélectionne les boutons
        assert_eq!(bus.read(0xFF00) & 0x0F, 0b0111);
    }
}
