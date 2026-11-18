//! Cartouches Game Boy : lecture de l'en-tête (octets 0x0100 à 0x014F)
//! et contrôleurs de banques (MBC1, MBC3 avec horloge, MBC5).
//!
//! Le logo présent à 0x0104 n'est volontairement ni vérifié ni stocké :
//! l'émulateur n'en a pas besoin et ce projet n'embarque aucune donnée protégée.

use serde::{Deserialize, Serialize};

use std::time::{SystemTime, UNIX_EPOCH};

use emu_core::EmuError;

const HEADER_END: usize = 0x0150;
const TITLE_START: usize = 0x0134;
const CGB_FLAG: usize = 0x0143;
const CART_TYPE: usize = 0x0147;
const ROM_SIZE: usize = 0x0148;
const RAM_SIZE: usize = 0x0149;
const HEADER_CHECKSUM: usize = 0x014D;

/// Type de contrôleur mémoire (MBC) présent sur la cartouche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub title: String,
    pub mbc: Mbc,
    pub cgb_support: bool,
    /// Le jeu ne fonctionne que sur Game Boy Color (drapeau 0xC0).
    pub cgb_only: bool,
    pub rom_size: usize,
    pub ram_size: usize,
    /// `true` si la somme de contrôle de l'en-tête est correcte.
    pub checksum_ok: bool,
    /// La cartouche a une pile : sa RAM (et son horloge) survivent à l'extinction.
    pub battery: bool,
    /// La cartouche a une horloge temps réel (MBC3 avec timer).
    pub rtc: bool,
}

/// Horloge temps réel (RTC) des cartouches MBC3.
///
/// Elle avance avec le temps émulé : une seconde toutes les 4 194 304 T-cycles.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rtc {
    pub seconds: u8,
    pub minutes: u8,
    pub hours: u8,
    /// Jours, sur 9 bits.
    pub days: u16,
    /// Horloge en pause (bit 6 de DH).
    pub halted: bool,
    /// Le compteur de jours a dépassé 511 (bit 7 de DH).
    pub day_carry: bool,
}

impl Rtc {
    fn read(&self, reg: u8) -> u8 {
        match reg {
            0x08 => self.seconds,
            0x09 => self.minutes,
            0x0A => self.hours,
            0x0B => self.days as u8,
            _ => {
                ((self.days >> 8) as u8 & 0x01)
                    | (self.halted as u8) << 6
                    | (self.day_carry as u8) << 7
            }
        }
    }

    fn write(&mut self, reg: u8, value: u8) {
        match reg {
            0x08 => self.seconds = value & 0x3F,
            0x09 => self.minutes = value & 0x3F,
            0x0A => self.hours = value & 0x1F,
            0x0B => self.days = (self.days & 0x100) | value as u16,
            _ => {
                self.days = (self.days & 0xFF) | ((value as u16 & 0x01) << 8);
                self.halted = value & 0x40 != 0;
                self.day_carry = value & 0x80 != 0;
            }
        }
    }

    /// Avance de `seconds` secondes d'un coup (temps passé émulateur fermé).
    pub fn advance_by(&mut self, seconds: u64) {
        if self.halted || seconds == 0 {
            return;
        }
        let total = self.seconds as u64
            + self.minutes as u64 * 60
            + self.hours as u64 * 3600
            + self.days as u64 * 86_400
            + seconds;
        self.seconds = (total % 60) as u8;
        self.minutes = (total / 60 % 60) as u8;
        self.hours = (total / 3600 % 24) as u8;
        let days = total / 86_400;
        if days > 0x1FF {
            self.day_carry = true;
        }
        self.days = (days & 0x1FF) as u16;
    }

    fn to_footer(&self, out: &mut Vec<u8>) {
        for reg in 0x08..=0x0C {
            out.extend_from_slice(&(self.read(reg) as u32).to_le_bytes());
        }
    }

    fn from_footer(bytes: &[u8]) -> Self {
        let mut rtc = Rtc::default();
        for (i, reg) in (0x08..=0x0C).enumerate() {
            rtc.write(reg, bytes[i * 4]);
        }
        rtc
    }

    fn advance_second(&mut self) {
        if self.halted {
            return;
        }
        // Comme sur le vrai circuit, on compare à la valeur exacte : une valeur
        // hors limites écrite par le jeu continue de compter jusqu'au débordement.
        self.seconds = (self.seconds + 1) & 0x3F;
        if self.seconds != 60 {
            return;
        }
        self.seconds = 0;
        self.minutes = (self.minutes + 1) & 0x3F;
        if self.minutes != 60 {
            return;
        }
        self.minutes = 0;
        self.hours = (self.hours + 1) & 0x1F;
        if self.hours != 24 {
            return;
        }
        self.hours = 0;
        self.days += 1;
        if self.days > 0x1FF {
            self.days = 0;
            self.day_carry = true;
        }
    }
}

const CYCLES_PER_SECOND: u32 = 4_194_304;

/// Taille du bloc horloge ajouté en fin de `.sav` : 5 registres courants,
/// 5 registres figés (4 octets chacun) puis l'heure Unix de la sauvegarde.
/// C'est le format utilisé par la plupart des émulateurs Game Boy.
const RTC_FOOTER_LEN: usize = 48;
/// Variante plus ancienne, avec une heure Unix sur 32 bits.
const RTC_FOOTER_LEN_OLD: usize = 44;

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Une cartouche chargée en mémoire, avec l'état de son contrôleur de banques.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cartridge {
    pub header: Header,
    /// La ROM n'est jamais copiée dans les sauvegardes instantanées.
    #[serde(skip)]
    pub rom: Vec<u8>,
    /// RAM de sauvegarde présente sur la cartouche (vide si aucune).
    pub ram: Vec<u8>,
    ram_enabled: bool,
    /// Banque ROM visible en 0x4000-0x7FFF.
    /// MBC1 : 5 bits, MBC3 : 7 bits, MBC5 : 9 bits.
    rom_bank: u16,
    /// MBC1 : 2 bits hauts (banque ROM haute ou banque RAM selon le mode).
    /// MBC3 : banque RAM (0-3) ou registre RTC (0x08-0x0C).
    /// MBC5 : banque RAM (0-15).
    ram_bank: u8,
    /// MBC1 : mode de banking avancé.
    mode: bool,
    /// MBC3 : horloge qui tourne, et copie figée que lit le jeu.
    pub rtc: Rtc,
    rtc_latched: Rtc,
    /// MBC3 : le verrouillage se fait en écrivant 0x00 puis 0x01.
    latch_ready: bool,
    rtc_cycles: u32,
}

impl Cartridge {
    fn rom_byte(&self, bank: usize, addr: u16) -> u8 {
        if self.rom.is_empty() {
            return 0xFF;
        }
        let offset = bank * 0x4000 + (addr as usize & 0x3FFF);
        self.rom[offset % self.rom.len()]
    }

    /// Lit dans la zone ROM (0x0000-0x7FFF).
    pub fn read_rom(&self, addr: u16) -> u8 {
        let low_area = addr < 0x4000;
        let bank = match self.header.mbc {
            Mbc::Mbc1 if low_area && self.mode => (self.ram_bank as usize) << 5,
            Mbc::Mbc1 if !low_area => ((self.ram_bank as usize) << 5) | self.rom_bank as usize,
            Mbc::Mbc3 | Mbc::Mbc5 if !low_area => self.rom_bank as usize,
            _ if low_area => 0,
            _ => 1,
        };
        self.rom_byte(bank, addr)
    }

    /// Écriture dans la zone ROM : sert à piloter le contrôleur de banques.
    pub fn write_rom(&mut self, addr: u16, value: u8) {
        match (self.header.mbc, addr) {
            (Mbc::Mbc1 | Mbc::Mbc3 | Mbc::Mbc5, 0x0000..=0x1FFF) => {
                self.ram_enabled = value & 0x0F == 0x0A;
            }

            (Mbc::Mbc1, 0x2000..=0x3FFF) => self.rom_bank = (value as u16 & 0x1F).max(1),
            (Mbc::Mbc1, 0x4000..=0x5FFF) => self.ram_bank = value & 0x03,
            (Mbc::Mbc1, _) => self.mode = value & 0x01 != 0,

            (Mbc::Mbc3, 0x2000..=0x3FFF) => self.rom_bank = (value as u16 & 0x7F).max(1),
            (Mbc::Mbc3, 0x4000..=0x5FFF) => self.ram_bank = value,
            (Mbc::Mbc3, _) => {
                if self.latch_ready && value == 0x01 {
                    self.rtc_latched = self.rtc.clone();
                }
                self.latch_ready = value == 0x00;
            }

            // Sur MBC5, la banque 0 peut aussi être placée en 0x4000.
            (Mbc::Mbc5, 0x2000..=0x2FFF) => self.rom_bank = (self.rom_bank & 0x100) | value as u16,
            (Mbc::Mbc5, 0x3000..=0x3FFF) => {
                self.rom_bank = (self.rom_bank & 0xFF) | ((value as u16 & 0x01) << 8);
            }
            (Mbc::Mbc5, 0x4000..=0x5FFF) => self.ram_bank = value & 0x0F,

            _ => {}
        }
    }

    fn ram_offset(&self, addr: u16) -> Option<usize> {
        let needs_enable = matches!(self.header.mbc, Mbc::Mbc1 | Mbc::Mbc3 | Mbc::Mbc5);
        if self.ram.is_empty() || (needs_enable && !self.ram_enabled) {
            return None;
        }
        let bank = match self.header.mbc {
            Mbc::Mbc1 if self.mode => self.ram_bank as usize,
            Mbc::Mbc3 if self.ram_bank <= 0x03 => self.ram_bank as usize,
            Mbc::Mbc3 => return None, // un registre RTC est sélectionné
            Mbc::Mbc5 => self.ram_bank as usize,
            _ => 0,
        };
        Some((bank * 0x2000 + (addr as usize & 0x1FFF)) % self.ram.len())
    }

    fn rtc_selected(&self) -> bool {
        self.header.mbc == Mbc::Mbc3 && self.ram_enabled && (0x08..=0x0C).contains(&self.ram_bank)
    }

    /// Lit dans la RAM de cartouche (0xA000-0xBFFF).
    pub fn read_ram(&self, addr: u16) -> u8 {
        if self.rtc_selected() {
            return self.rtc_latched.read(self.ram_bank);
        }
        self.ram_offset(addr).map_or(0xFF, |i| self.ram[i])
    }

    /// Écrit dans la RAM de cartouche (0xA000-0xBFFF).
    pub fn write_ram(&mut self, addr: u16, value: u8) {
        if self.rtc_selected() {
            self.rtc.write(self.ram_bank, value);
            self.rtc_cycles = 0;
            return;
        }
        if let Some(i) = self.ram_offset(addr) {
            self.ram[i] = value;
        }
    }

    /// Contenu du fichier `.sav` : la RAM de la cartouche, suivie de l'horloge
    /// pour les MBC3 qui en ont une. `None` si la cartouche n'a pas de pile.
    pub fn save_data(&self) -> Option<Vec<u8>> {
        self.save_data_at(unix_now())
    }

    fn save_data_at(&self, now: u64) -> Option<Vec<u8>> {
        if !self.header.battery || (self.ram.is_empty() && !self.header.rtc) {
            return None;
        }
        let mut out = self.ram.clone();
        if self.header.rtc {
            self.rtc.to_footer(&mut out);
            self.rtc_latched.to_footer(&mut out);
            out.extend_from_slice(&now.to_le_bytes());
        }
        Some(out)
    }

    /// Recharge un fichier `.sav`. L'horloge rattrape le temps passé depuis la sauvegarde.
    pub fn load_save_data(&mut self, data: &[u8]) -> Result<(), EmuError> {
        self.load_save_data_at(data, unix_now())
    }

    fn load_save_data_at(&mut self, data: &[u8], now: u64) -> Result<(), EmuError> {
        let ram_len = self.ram.len();
        if data.len() < ram_len {
            return Err(EmuError::InvalidSave(format!(
                "{} octets au lieu de {ram_len} au minimum",
                data.len()
            )));
        }
        self.ram.copy_from_slice(&data[..ram_len]);

        let footer = &data[ram_len..];
        if self.header.rtc && footer.len() >= RTC_FOOTER_LEN_OLD {
            self.rtc = Rtc::from_footer(&footer[..20]);
            self.rtc_latched = Rtc::from_footer(&footer[20..40]);
            let saved_at = if footer.len() >= RTC_FOOTER_LEN {
                u64::from_le_bytes(footer[40..48].try_into().unwrap())
            } else {
                u32::from_le_bytes(footer[40..44].try_into().unwrap()) as u64
            };
            self.rtc.advance_by(now.saturating_sub(saved_at));
        }
        Ok(())
    }

    /// Avance de `cycles` T-cycles de temps réel : fait tourner l'horloge des MBC3.
    pub fn tick(&mut self, cycles: u32) {
        if self.header.mbc != Mbc::Mbc3 {
            return;
        }
        self.rtc_cycles += cycles;
        if self.rtc_cycles >= CYCLES_PER_SECOND {
            self.rtc_cycles -= CYCLES_PER_SECOND;
            self.rtc.advance_second();
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
            cgb_only: cgb_flag == 0xC0,
            rom_size,
            ram_size,
            checksum_ok: header_checksum(data) == data[HEADER_CHECKSUM],
            battery: matches!(
                data[CART_TYPE],
                0x03 | 0x06 | 0x09 | 0x0D | 0x0F | 0x10 | 0x13 | 0x1B | 0x1E | 0xFF
            ),
            rtc: matches!(data[CART_TYPE], 0x0F | 0x10),
        };

        Ok(Cartridge {
            ram: vec![0; header.ram_size],
            header,
            rom: data.to_vec(),
            ram_enabled: false,
            rom_bank: 1,
            ram_bank: 0,
            mode: false,
            rtc: Rtc::default(),
            rtc_latched: Rtc::default(),
            latch_ready: false,
            rtc_cycles: 0,
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

    /// ROM factice de `banks` banques ; chaque banque contient son numéro en 0x0001-0x0002.
    fn banked_rom(cart_type: u8, rom_code: u8, banks: usize) -> Vec<u8> {
        let mut rom = vec![0u8; banks * 0x4000];
        rom[CART_TYPE] = cart_type;
        rom[ROM_SIZE] = rom_code;
        for bank in 0..banks {
            rom[bank * 0x4000 + 1] = bank as u8;
            rom[bank * 0x4000 + 2] = (bank >> 8) as u8;
        }
        rom
    }

    fn bank_at_4000(cart: &Cartridge) -> usize {
        cart.read_rom(0x4001) as usize | (cart.read_rom(0x4002) as usize) << 8
    }

    #[test]
    fn mbc3_switches_rom_and_ram_banks() {
        let mut rom = banked_rom(0x13, 0x06, 128);
        rom[RAM_SIZE] = 0x03; // 32 Kio = 4 banques
        let mut cart = Cartridge::from_bytes(&rom).unwrap();
        cart.write_rom(0x2000, 0x7F);
        assert_eq!(bank_at_4000(&cart), 127);
        cart.write_rom(0x2000, 0x00);
        assert_eq!(bank_at_4000(&cart), 1);

        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, 0x02);
        cart.write_ram(0xA000, 0x22);
        cart.write_rom(0x4000, 0x00);
        assert_eq!(cart.read_ram(0xA000), 0x00);
        cart.write_rom(0x4000, 0x02);
        assert_eq!(cart.read_ram(0xA000), 0x22);
    }

    #[test]
    fn mbc3_rtc_counts_and_latches() {
        let mut cart = Cartridge::from_bytes(&banked_rom(0x10, 0x00, 2)).unwrap();
        cart.write_rom(0x0000, 0x0A);
        for _ in 0..(CYCLES_PER_SECOND / 4) * 3 {
            cart.tick(4);
        }
        cart.write_rom(0x4000, 0x08); // registre des secondes
        assert_eq!(
            cart.read_ram(0xA000),
            0,
            "rien n'est visible avant le verrouillage"
        );

        cart.write_rom(0x6000, 0x00);
        cart.write_rom(0x6000, 0x01);
        assert_eq!(cart.read_ram(0xA000), 3);

        // La valeur reste figée tant qu'on ne verrouille pas à nouveau.
        for _ in 0..CYCLES_PER_SECOND / 4 {
            cart.tick(4);
        }
        assert_eq!(cart.read_ram(0xA000), 3);
    }

    #[test]
    fn rtc_rolls_over_into_days() {
        let mut rtc = Rtc {
            seconds: 59,
            minutes: 59,
            hours: 23,
            days: 0x1FF,
            ..Rtc::default()
        };
        rtc.advance_second();
        assert_eq!(
            (rtc.seconds, rtc.minutes, rtc.hours, rtc.days),
            (0, 0, 0, 0)
        );
        assert!(rtc.day_carry);

        rtc.halted = true;
        rtc.advance_second();
        assert_eq!(rtc.seconds, 0);
    }

    #[test]
    fn mbc5_uses_9_bit_rom_bank() {
        let mut cart = Cartridge::from_bytes(&banked_rom(0x19, 0x08, 512)).unwrap();
        cart.write_rom(0x2000, 0x2A);
        cart.write_rom(0x3000, 0x01);
        assert_eq!(bank_at_4000(&cart), 0x12A);
        // La banque 0 est autorisée en zone haute.
        cart.write_rom(0x2000, 0x00);
        cart.write_rom(0x3000, 0x00);
        assert_eq!(bank_at_4000(&cart), 0);
    }

    #[test]
    fn mbc5_switches_ram_banks() {
        let mut rom = banked_rom(0x1B, 0x00, 2);
        rom[RAM_SIZE] = 0x04; // 128 Kio = 16 banques
        let mut cart = Cartridge::from_bytes(&rom).unwrap();
        cart.write_rom(0x0000, 0x0A);
        cart.write_rom(0x4000, 0x0F);
        cart.write_ram(0xBFFF, 0x55);
        assert_eq!(cart.ram[15 * 0x2000 + 0x1FFF], 0x55);
    }

    #[test]
    fn battery_ram_round_trips() {
        let mut rom = fake_rom("SAVE", 0x03); // MBC1 + RAM + pile
        rom[RAM_SIZE] = 0x02;
        let mut cart = Cartridge::from_bytes(&rom).unwrap();
        assert!(cart.header.battery);
        cart.write_rom(0x0000, 0x0A);
        cart.write_ram(0xA123, 0x99);
        let data = cart.save_data().unwrap();
        assert_eq!(data.len(), 8 * 1024);

        let mut fresh = Cartridge::from_bytes(&rom).unwrap();
        fresh.load_save_data(&data).unwrap();
        fresh.write_rom(0x0000, 0x0A);
        assert_eq!(fresh.read_ram(0xA123), 0x99);
    }

    #[test]
    fn no_save_without_battery() {
        let mut rom = fake_rom("NOSAVE", 0x02); // MBC1 + RAM, sans pile
        rom[RAM_SIZE] = 0x02;
        assert!(Cartridge::from_bytes(&rom).unwrap().save_data().is_none());
    }

    #[test]
    fn save_too_small_is_rejected() {
        let mut rom = fake_rom("SAVE", 0x03);
        rom[RAM_SIZE] = 0x02;
        let mut cart = Cartridge::from_bytes(&rom).unwrap();
        assert!(cart.load_save_data(&[0; 100]).is_err());
    }

    #[test]
    fn rtc_catches_up_with_time_spent_closed() {
        let mut rom = fake_rom("CLOCK", 0x10); // MBC3 + horloge + RAM + pile
        rom[RAM_SIZE] = 0x02;
        let mut cart = Cartridge::from_bytes(&rom).unwrap();
        cart.rtc.seconds = 10;
        let data = cart.save_data_at(1_000).unwrap();
        assert_eq!(data.len(), 8 * 1024 + RTC_FOOTER_LEN);

        let mut fresh = Cartridge::from_bytes(&rom).unwrap();
        // Rouvert 1 jour, 1 heure, 1 minute et 1 seconde plus tard.
        fresh.load_save_data_at(&data, 1_000 + 90_061).unwrap();
        let rtc = &fresh.rtc;
        assert_eq!(
            (rtc.days, rtc.hours, rtc.minutes, rtc.seconds),
            (1, 1, 1, 11)
        );
    }

    #[test]
    fn rtc_advance_by_sets_day_carry() {
        let mut rtc = Rtc {
            days: 511,
            ..Rtc::default()
        };
        rtc.advance_by(86_400);
        assert_eq!(rtc.days, 0);
        assert!(rtc.day_carry);
    }

    #[test]
    fn reject_too_small() {
        assert!(Cartridge::from_bytes(&[0u8; 0x100]).is_err());
    }
}
