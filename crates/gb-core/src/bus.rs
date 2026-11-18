//! Bus mémoire de la Game Boy : relie le CPU à la cartouche, à la RAM
//! et aux registres matériels (timer, port série, manette, écran, son...).
//!
//! Carte mémoire :
//! - `0000-7FFF` ROM de cartouche
//! - `8000-9FFF` VRAM (2 banques en mode Color)
//! - `A000-BFFF` RAM de cartouche
//! - `C000-CFFF` WRAM banque 0
//! - `D000-DFFF` WRAM banque 1 (1 à 7 en mode Color) ; `E000-FDFF` en est un miroir
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
    /// Mode Game Boy Color.
    pub cgb: bool,
    /// 8 banques de 4 Kio (seules les 2 premières servent en mode classique).
    wram: Vec<u8>,
    /// Banque WRAM visible en D000-DFFF (registre SVBK, mode Color).
    wram_bank: u8,
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

    // Double vitesse (registre KEY1, mode Color).
    pub double_speed: bool,
    speed_switch_armed: bool,

    // HDMA (mode Color) : copies vers la VRAM.
    hdma_src: u16,
    hdma_dst: u16,
    /// Blocs de 16 octets restant à copier pendant les HBlank.
    hdma_blocks: u8,
    hdma_active: bool,

    /// Temps réel écoulé, en T-cycles « normaux » (la double vitesse n'accélère pas l'écran).
    pub clock: u64,

    pub ppu: Ppu,
    pub apu: Apu,

    /// Boutons appuyés, un bit par bouton (voir `GameBoy::button_bit`).
    pub buttons: u8,
    /// Octets envoyés sur le port série. Les ROMs de test y écrivent leurs résultats.
    pub serial_output: Vec<u8>,
}

impl GbBus {
    pub fn new(cartridge: Cartridge, cgb: bool) -> Self {
        let mut io = vec![0u8; 0x80];
        // Valeurs laissées par la boot ROM.
        io[0x00] = 0x30; // JOYP : aucune sélection
        GbBus {
            cartridge,
            cgb,
            wram: vec![0; 0x8000],
            wram_bank: 1,
            hram: vec![0; 0x7F],
            io,
            ie: 0,
            if_: 0x01,
            div: 0xABCC,
            tima: 0,
            tma: 0,
            tac: 0,
            double_speed: false,
            speed_switch_armed: false,
            hdma_src: 0,
            hdma_dst: 0,
            hdma_blocks: 0,
            hdma_active: false,
            clock: 0,
            ppu: Ppu::with_mode(cgb),
            apu: Apu::new(),
            buttons: 0,
            serial_output: Vec::new(),
        }
    }

    fn request_interrupt(&mut self, mask: u8) {
        self.if_ |= mask;
    }

    /// Le timer incrémente TIMA sur chaque front descendant du bit de DIV
    /// sélectionné par TAC, comme le vrai circuit. Il suit la vitesse du CPU.
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

    fn wram_index(&self, addr: u16) -> usize {
        let offset = (addr as usize - 0xC000) & 0x1FFF;
        if offset < 0x1000 {
            offset
        } else {
            self.wram_bank as usize * 0x1000 + (offset - 0x1000)
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

    /// Copie un bloc de 16 octets pour la HDMA.
    fn hdma_copy_block(&mut self) {
        for _ in 0..16 {
            let byte = self.read(self.hdma_src);
            self.ppu.write_vram(0x8000 | self.hdma_dst, byte);
            self.hdma_src = self.hdma_src.wrapping_add(1);
            self.hdma_dst = (self.hdma_dst + 1) & 0x1FFF;
        }
    }

    fn write_hdma_control(&mut self, value: u8) {
        let blocks = (value & 0x7F) + 1;
        if self.hdma_active && value & 0x80 == 0 {
            // Écrire avec le bit 7 à 0 pendant une HDMA l'interrompt.
            self.hdma_active = false;
        } else if value & 0x80 == 0 {
            // DMA générale : tout est copié immédiatement.
            for _ in 0..blocks {
                self.hdma_copy_block();
            }
        } else {
            // DMA HBlank : un bloc au début de chaque HBlank.
            self.hdma_blocks = blocks;
            self.hdma_active = true;
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
            0xFF4D if self.cgb => {
                0x7E | (self.double_speed as u8) << 7 | self.speed_switch_armed as u8
            }
            0xFF4F | 0xFF68..=0xFF6B if self.cgb => self.ppu.read_reg(addr),
            0xFF55 if self.cgb => {
                if self.hdma_active {
                    (self.hdma_blocks - 1) & 0x7F
                } else {
                    0xFF
                }
            }
            0xFF70 if self.cgb => 0xF8 | self.wram_bank,
            // Registres propres à la Color : absents sur une Game Boy classique.
            0xFF4D | 0xFF4F | 0xFF51..=0xFF55 | 0xFF68..=0xFF6B | 0xFF70 => 0xFF,
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
            0xFF4D if self.cgb => self.speed_switch_armed = value & 0x01 != 0,
            0xFF4F | 0xFF68..=0xFF6B if self.cgb => self.ppu.write_reg(addr, value),
            0xFF51 if self.cgb => self.hdma_src = (self.hdma_src & 0x00FF) | (value as u16) << 8,
            0xFF52 if self.cgb => self.hdma_src = (self.hdma_src & 0xFF00) | (value & 0xF0) as u16,
            0xFF53 if self.cgb => {
                self.hdma_dst = (self.hdma_dst & 0x00FF) | ((value & 0x1F) as u16) << 8;
            }
            0xFF54 if self.cgb => self.hdma_dst = (self.hdma_dst & 0xFF00) | (value & 0xF0) as u16,
            0xFF55 if self.cgb => self.write_hdma_control(value),
            0xFF70 if self.cgb => self.wram_bank = (value & 0x07).max(1),
            0xFF4D | 0xFF4F | 0xFF51..=0xFF55 | 0xFF68..=0xFF6B | 0xFF70 => {}
            _ => self.io[(addr - 0xFF00) as usize] = value,
        }
    }
}

impl Bus for GbBus {
    fn read(&mut self, addr: u16) -> u8 {
        match addr {
            0x0000..=0x7FFF => self.cartridge.read_rom(addr),
            0x8000..=0x9FFF => self.ppu.read_vram(addr),
            0xA000..=0xBFFF => self.cartridge.read_ram(addr),
            0xC000..=0xFDFF => self.wram[self.wram_index(addr)],
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
            0x8000..=0x9FFF => self.ppu.write_vram(addr, value),
            0xA000..=0xBFFF => self.cartridge.write_ram(addr, value),
            0xC000..=0xFDFF => {
                let i = self.wram_index(addr);
                self.wram[i] = value;
            }
            0xFE00..=0xFE9F => self.ppu.oam[(addr - 0xFE00) as usize] = value,
            0xFEA0..=0xFEFF => {}
            0xFF00..=0xFF7F => self.write_io(addr, value),
            0xFF80..=0xFFFE => self.hram[(addr - 0xFF80) as usize] = value,
            0xFFFF => self.ie = value,
        }
    }

    fn tick(&mut self) {
        self.tick_timer();
        // En double vitesse, un M-cycle ne dure que 2 T-cycles de temps réel :
        // l'écran, le son et l'horloge de la cartouche avancent deux fois moins.
        let real = if self.double_speed { 2 } else { 4 };
        self.clock += real as u64;
        self.cartridge.tick(real);
        self.apu.tick(real);
        let irq = self.ppu.tick(real);
        self.request_interrupt(irq);
        if self.ppu.take_hblank() && self.hdma_active {
            self.hdma_copy_block();
            self.hdma_blocks -= 1;
            if self.hdma_blocks == 0 {
                self.hdma_active = false;
            }
        }
    }

    fn stop(&mut self) {
        if self.cgb && self.speed_switch_armed {
            self.double_speed = !self.double_speed;
            self.speed_switch_armed = false;
            self.div = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::tests::fake_rom;

    fn bus() -> GbBus {
        GbBus::new(
            Cartridge::from_bytes(&fake_rom("BUS", 0x00)).unwrap(),
            false,
        )
    }

    fn cgb_bus() -> GbBus {
        GbBus::new(Cartridge::from_bytes(&fake_rom("BUS", 0x00)).unwrap(), true)
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

    #[test]
    fn cgb_wram_banks() {
        let mut bus = cgb_bus();
        bus.write(0xD000, 0x11); // banque 1 par défaut
        bus.write(0xFF70, 3);
        assert_eq!(bus.read(0xD000), 0x00);
        bus.write(0xD000, 0x33);
        bus.write(0xFF70, 0); // 0 sélectionne la banque 1
        assert_eq!(bus.read(0xFF70) & 0x07, 1);
        assert_eq!(bus.read(0xD000), 0x11);
        bus.write(0xC000, 0x77); // la banque 0 ne bouge jamais
        bus.write(0xFF70, 3);
        assert_eq!(bus.read(0xC000), 0x77);
        assert_eq!(bus.read(0xD000), 0x33);
    }

    #[test]
    fn classic_mode_hides_color_registers() {
        let mut bus = bus();
        bus.write(0xFF70, 3);
        assert_eq!(bus.read(0xFF70), 0xFF);
        assert_eq!(bus.read(0xFF4D), 0xFF);
        bus.write(0xD000, 0x55);
        assert_eq!(bus.read(0xD000), 0x55);
    }

    #[test]
    fn general_dma_copies_immediately() {
        let mut bus = cgb_bus();
        for i in 0..32u16 {
            bus.write(0xC000 + i, i as u8 + 1);
        }
        bus.write(0xFF51, 0xC0);
        bus.write(0xFF52, 0x00);
        bus.write(0xFF53, 0x01); // destination 0x8100
        bus.write(0xFF54, 0x00);
        bus.write(0xFF55, 0x01); // 2 blocs, DMA générale
        assert_eq!(bus.read(0x8100), 1);
        assert_eq!(bus.read(0x811F), 32);
        assert_eq!(bus.read(0xFF55), 0xFF);
    }

    #[test]
    fn hblank_dma_copies_one_block_per_line() {
        let mut bus = cgb_bus();
        for i in 0..32u16 {
            bus.write(0xC000 + i, 0xA0 + i as u8);
        }
        bus.write(0xFF51, 0xC0);
        bus.write(0xFF52, 0x00);
        bus.write(0xFF53, 0x00);
        bus.write(0xFF54, 0x00);
        bus.write(0xFF55, 0x81); // 2 blocs, pendant les HBlank
        assert_eq!(bus.read(0xFF55), 0x01, "2 blocs restants");
        assert_eq!(bus.read(0x8000), 0x00);

        // Une ligne complète : un HBlank, donc un bloc.
        for _ in 0..456 / 4 {
            bus.tick();
        }
        assert_eq!(bus.read(0x8000), 0xA0);
        assert_eq!(bus.read(0x8010), 0x00);
        assert_eq!(bus.read(0xFF55), 0x00, "1 bloc restant");
        for _ in 0..456 / 4 {
            bus.tick();
        }
        assert_eq!(bus.read(0x8010), 0xB0);
        assert_eq!(bus.read(0xFF55), 0xFF, "terminée");
    }

    #[test]
    fn double_speed_halves_screen_time() {
        let mut bus = cgb_bus();
        bus.write(0xFF4D, 0x01);
        bus.stop();
        assert!(bus.double_speed);
        assert_eq!(bus.read(0xFF4D) & 0x81, 0x80);
        // 456 dots par ligne : il faut 228 M-cycles en double vitesse.
        for _ in 0..456 / 4 {
            bus.tick();
        }
        assert_eq!(bus.read(0xFF44), 0);
        for _ in 0..456 / 4 {
            bus.tick();
        }
        assert_eq!(bus.read(0xFF44), 1);
    }
}
