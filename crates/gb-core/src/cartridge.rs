//! Lecture de l'en-tête des cartouches Game Boy (octets 0x0100 à 0x014F).
//!
//! Le logo présent à 0x0104 n'est volontairement ni vérifié ni stocké :
//! l'émulateur n'en a pas besoin et ce projet n'embarque aucune donnée protégée.

use emu_core::EmuError;

const HEADER_END: usize = 0x0150;
const TITLE_START: usize = 0x0134;
const CGB_FLAG: usize = 0x0143;
const CART_TYPE: usize = 0x0147;
const ROM_SIZE: usize = 0x0148;
const RAM_SIZE: usize = 0x0149;
const HEADER_CHECKSUM: usize = 0x014D;

/// Type de contrôleur mémoire (MBC) présent sur la cartouche.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mbc {
    RomOnly,
    Mbc1,
    Mbc2,
    Mbc3,
    Mbc5,
    Unknown(u8),
}

impl Mbc {
    fn from_byte(byte: u8) -> Self {
        match byte {
            0x00 => Mbc::RomOnly,
            0x01..=0x03 => Mbc::Mbc1,
            0x05..=0x06 => Mbc::Mbc2,
            0x0F..=0x13 => Mbc::Mbc3,
            0x19..=0x1E => Mbc::Mbc5,
            other => Mbc::Unknown(other),
        }
    }
}

/// Informations extraites de l'en-tête d'une cartouche.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub mbc: Mbc,
    pub cgb_support: bool,
    pub rom_size: usize,
    pub ram_size: usize,
    /// `true` si la somme de contrôle de l'en-tête est correcte.
    pub checksum_ok: bool,
}

/// Une cartouche chargée en mémoire, avec l'état de son contrôleur de banques.
#[derive(Debug, Clone)]
pub struct Cartridge {
    pub header: Header,
    pub rom: Vec<u8>,
    /// RAM de sauvegarde présente sur la cartouche (vide si aucune).
    pub ram: Vec<u8>,
    ram_enabled: bool,
    /// MBC1 : 5 bits bas du numéro de banque ROM (jamais 0).
    bank_lo: u8,
    /// MBC1 : 2 bits hauts, banque ROM haute ou banque RAM selon le mode.
    bank_hi: u8,
    /// MBC1 : mode de banking avancé.
    mode: bool,
}

impl Cartridge {
    /// Lit dans la zone ROM (0x0000-0x7FFF).
    pub fn read_rom(&self, addr: u16) -> u8 {
        let bank = match (self.header.mbc, addr) {
            (Mbc::Mbc1, 0x0000..=0x3FFF) if self.mode => (self.bank_hi as usize) << 5,
            (Mbc::Mbc1, 0x4000..=0x7FFF) => ((self.bank_hi as usize) << 5) | self.bank_lo as usize,
            (_, 0x0000..=0x3FFF) => 0,
            _ => 1,
        };
        let offset = bank * 0x4000 + (addr as usize & 0x3FFF);
        if self.rom.is_empty() {
            return 0xFF;
        }
        self.rom[offset % self.rom.len()]
    }

    /// Écriture dans la zone ROM : sert à piloter le contrôleur de banques.
    pub fn write_rom(&mut self, addr: u16, value: u8) {
        if self.header.mbc != Mbc::Mbc1 {
            return;
        }
        match addr {
            0x0000..=0x1FFF => self.ram_enabled = value & 0x0F == 0x0A,
            0x2000..=0x3FFF => self.bank_lo = (value & 0x1F).max(1),
            0x4000..=0x5FFF => self.bank_hi = value & 0x03,
            _ => self.mode = value & 0x01 != 0,
        }
    }

    fn ram_offset(&self, addr: u16) -> Option<usize> {
        if self.ram.is_empty() || (self.header.mbc == Mbc::Mbc1 && !self.ram_enabled) {
            return None;
        }
        let bank = if self.header.mbc == Mbc::Mbc1 && self.mode {
            self.bank_hi as usize
        } else {
            0
        };
        Some((bank * 0x2000 + (addr as usize & 0x1FFF)) % self.ram.len())
    }

    /// Lit dans la RAM de cartouche (0xA000-0xBFFF).
    pub fn read_ram(&self, addr: u16) -> u8 {
        self.ram_offset(addr).map_or(0xFF, |i| self.ram[i])
    }

    /// Écrit dans la RAM de cartouche (0xA000-0xBFFF).
    pub fn write_ram(&mut self, addr: u16, value: u8) {
        if let Some(i) = self.ram_offset(addr) {
            self.ram[i] = value;
        }
    }

    /// Analyse une ROM et vérifie que son en-tête est cohérent.
    pub fn from_bytes(data: &[u8]) -> Result<Self, EmuError> {
        if data.len() < HEADER_END {
            return Err(EmuError::InvalidRom(format!(
                "fichier trop petit ({} octets, minimum {HEADER_END})",
                data.len()
            )));
        }

        let cgb_flag = data[CGB_FLAG];
        let cgb_support = cgb_flag & 0x80 != 0;
        // Sur les cartouches compatibles Color, le dernier octet du titre est le drapeau CGB.
        let title_end = if cgb_support { CGB_FLAG } else { CGB_FLAG + 1 };
        let title = data[TITLE_START..title_end]
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '?'
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string();

        let rom_code = data[ROM_SIZE];
        if rom_code > 0x08 {
            return Err(EmuError::InvalidRom(format!(
                "code de taille ROM inconnu : {rom_code:#04X}"
            )));
        }
        let rom_size = (32 * 1024) << rom_code;

        let ram_size = match data[RAM_SIZE] {
            0x00 => 0,
            0x02 => 8 * 1024,
            0x03 => 32 * 1024,
            0x04 => 128 * 1024,
            0x05 => 64 * 1024,
            other => {
                return Err(EmuError::InvalidRom(format!(
                    "code de taille RAM inconnu : {other:#04X}"
                )));
            }
        };

        let header = Header {
            title,
            mbc: Mbc::from_byte(data[CART_TYPE]),
            cgb_support,
            rom_size,
            ram_size,
            checksum_ok: header_checksum(data) == data[HEADER_CHECKSUM],
        };

        Ok(Cartridge {
            ram: vec![0; header.ram_size],
            header,
            rom: data.to_vec(),
            ram_enabled: false,
            bank_lo: 1,
            bank_hi: 0,
            mode: false,
        })
    }
}

/// Somme de contrôle de l'en-tête, calculée sur les octets 0x0134 à 0x014C.
pub fn header_checksum(data: &[u8]) -> u8 {
    data[TITLE_START..HEADER_CHECKSUM]
        .iter()
        .fold(0u8, |acc, &b| acc.wrapping_sub(b).wrapping_sub(1))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Fabrique une ROM factice de 32 Kio, entièrement générée par le test.
    pub fn fake_rom(title: &str, cart_type: u8) -> Vec<u8> {
        let mut rom = vec![0u8; 32 * 1024];
        rom[TITLE_START..TITLE_START + title.len()].copy_from_slice(title.as_bytes());
        rom[CART_TYPE] = cart_type;
        rom[HEADER_CHECKSUM] = header_checksum(&rom);
        rom
    }

    #[test]
    fn parse_header() {
        let cart = Cartridge::from_bytes(&fake_rom("EMURUST TEST", 0x01)).unwrap();
        assert_eq!(cart.header.title, "EMURUST TEST");
        assert_eq!(cart.header.mbc, Mbc::Mbc1);
        assert_eq!(cart.header.rom_size, 32 * 1024);
        assert_eq!(cart.header.ram_size, 0);
        assert!(!cart.header.cgb_support);
        assert!(cart.header.checksum_ok);
    }

    #[test]
    fn detect_bad_checksum() {
        let mut rom = fake_rom("TEST", 0x00);
        rom[HEADER_CHECKSUM] ^= 0xFF;
        assert!(!Cartridge::from_bytes(&rom).unwrap().header.checksum_ok);
    }

    #[test]
    fn mbc1_switches_rom_banks() {
        let mut rom = vec![0u8; 128 * 1024];
        rom[CART_TYPE] = 0x01;
        rom[ROM_SIZE] = 0x02;
        for bank in 0..8 {
            rom[bank * 0x4000] = bank as u8;
        }
        let mut cart = Cartridge::from_bytes(&rom).unwrap();
        assert_eq!(cart.read_rom(0x4000), 1);
        cart.write_rom(0x2000, 5);
        assert_eq!(cart.read_rom(0x4000), 5);
        // La banque 0 est remplacée par la 1 dans la zone haute.
        cart.write_rom(0x2000, 0);
        assert_eq!(cart.read_rom(0x4000), 1);
        assert_eq!(cart.read_rom(0x0000), 0);
    }

    #[test]
    fn mbc1_ram_needs_enable() {
        let mut rom = fake_rom("RAM", 0x03);
        rom[RAM_SIZE] = 0x02;
        let mut cart = Cartridge::from_bytes(&rom).unwrap();
        cart.write_ram(0xA000, 0x42);
        assert_eq!(cart.read_ram(0xA000), 0xFF);
        cart.write_rom(0x0000, 0x0A);
        cart.write_ram(0xA000, 0x42);
        assert_eq!(cart.read_ram(0xA000), 0x42);
    }

    #[test]
    fn reject_too_small() {
        assert!(Cartridge::from_bytes(&[0u8; 0x100]).is_err());
    }
}
