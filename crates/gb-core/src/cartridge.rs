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

/// Une cartouche chargée en mémoire.
#[derive(Debug, Clone)]
pub struct Cartridge {
    pub header: Header,
    pub rom: Vec<u8>,
}

impl Cartridge {
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
            header,
            rom: data.to_vec(),
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
    fn reject_too_small() {
        assert!(Cartridge::from_bytes(&[0u8; 0x100]).is_err());
    }
}
