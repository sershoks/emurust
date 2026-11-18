//! PPU (Picture Processing Unit) : génère l'image de la Game Boy.
//!
//! Chaque ligne dure 456 dots (1 dot = 1 T-cycle) et passe par trois modes :
//! - mode 2 (80 dots)  : recherche des sprites dans l'OAM
//! - mode 3 (172 dots) : envoi des pixels à l'écran
//! - mode 0 (204 dots) : HBlank
//!
//! Après 144 lignes visibles viennent 10 lignes de VBlank (mode 1).
//! Le rendu est fait ligne par ligne au début du HBlank, ce qui suffit
//! pour la très grande majorité des jeux.
//!
//! En mode Game Boy Color, la VRAM a deux banques, chaque tuile du fond a
//! des attributs (palette, banque, miroirs, priorité) et les couleurs viennent
//! de 8 + 8 palettes de 4 couleurs sur 15 bits.

use serde::{Deserialize, Serialize};

use crate::bus::{INT_STAT, INT_VBLANK};

pub const SCREEN_WIDTH: usize = 160;
pub const SCREEN_HEIGHT: usize = 144;

/// Palette verte de 4 teintes, du plus clair au plus foncé (format 0x00RRGGBB).
pub const DMG_PALETTE: [u32; 4] = [0x00E0_F8D0, 0x0088_C070, 0x0034_6856, 0x0008_1820];

const DOTS_PER_LINE: u16 = 456;
const MODE3_START: u16 = 80;
const HBLANK_START: u16 = 252;
const VBLANK_LINE: u8 = 144;
const LINES_PER_FRAME: u8 = 154;

// Bits de LCDC
const LCDC_BG_ON: u8 = 0x01;
const LCDC_OBJ_ON: u8 = 0x02;
const LCDC_OBJ_TALL: u8 = 0x04;
const LCDC_BG_MAP: u8 = 0x08;
const LCDC_TILE_DATA: u8 = 0x10;
const LCDC_WIN_ON: u8 = 0x20;
const LCDC_WIN_MAP: u8 = 0x40;
const LCDC_LCD_ON: u8 = 0x80;

#[derive(Serialize, Deserialize)]
pub struct Ppu {
    /// Deux banques de 8 Kio (la seconde n'existe qu'en mode Color).
    pub vram: Vec<u8>,
    pub oam: Vec<u8>,
    /// Mode Game Boy Color.
    pub cgb: bool,
    /// Banque VRAM visible par le CPU (registre VBK).
    vram_bank: u8,
    /// Palettes Color : 8 palettes × 4 couleurs × 2 octets (RGB 15 bits).
    bg_palettes: Vec<u8>,
    obj_palettes: Vec<u8>,
    /// Index d'écriture dans les palettes (BCPS / OCPS), bit 7 = auto-incrément.
    bcps: u8,
    ocps: u8,
    /// Passe à `true` au début de chaque HBlank (utilisé par la HDMA).
    hblank_started: bool,

    lcdc: u8,
    /// Seuls les bits 3 à 6 (sources d'interruption STAT) sont stockés.
    stat: u8,
    scy: u8,
    scx: u8,
    ly: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8,
    wy: u8,
    wx: u8,

    mode: u8,
    dot: u16,
    /// Compteur de lignes interne de la fenêtre.
    window_line: u8,
    /// État de la ligne d'interruption STAT (déclenchée sur front montant).
    stat_line: bool,

    /// Image en cours de dessin.
    back: Vec<u32>,
    /// Dernière image terminée, celle qu'on affiche.
    front: Vec<u32>,
}

impl Ppu {
    pub fn new() -> Self {
        Self::with_mode(false)
    }

    pub fn with_mode(cgb: bool) -> Self {
        let blank_color = if cgb { 0x00FF_FFFF } else { DMG_PALETTE[0] };
        let blank = vec![blank_color; SCREEN_WIDTH * SCREEN_HEIGHT];
        Ppu {
            vram: vec![0; 0x4000],
            oam: vec![0; 0xA0],
            cgb,
            vram_bank: 0,
            // Toutes les couleurs démarrent en blanc (0x7FFF).
            bg_palettes: vec![0xFF; 64],
            obj_palettes: vec![0xFF; 64],
            bcps: 0,
            ocps: 0,
            hblank_started: false,
            // Valeurs laissées par la boot ROM.
            lcdc: 0x91,
            stat: 0,
            scy: 0,
            scx: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xFC,
            obp0: 0xFF,
            obp1: 0xFF,
            wy: 0,
            wx: 0,
            mode: 2,
            dot: 0,
            window_line: 0,
            stat_line: false,
            back: blank.clone(),
            front: blank,
        }
    }

    /// Dernière image complète, un pixel `0x00RRGGBB` par `u32`.
    pub fn frame(&self) -> &[u32] {
        &self.front
    }

    fn lcd_on(&self) -> bool {
        self.lcdc & LCDC_LCD_ON != 0
    }

    fn blank_color(&self) -> u32 {
        if self.cgb {
            0x00FF_FFFF
        } else {
            DMG_PALETTE[0]
        }
    }

    /// Lecture CPU dans 0x8000-0x9FFF, dans la banque sélectionnée.
    pub fn read_vram(&self, addr: u16) -> u8 {
        self.vram[self.vram_bank as usize * 0x2000 + (addr as usize & 0x1FFF)]
    }

    pub fn write_vram(&mut self, addr: u16, value: u8) {
        self.vram[self.vram_bank as usize * 0x2000 + (addr as usize & 0x1FFF)] = value;
    }

    /// Indique (une seule fois) qu'un HBlank vient de commencer.
    pub fn take_hblank(&mut self) -> bool {
        std::mem::take(&mut self.hblank_started)
    }

    /// Couleur `color` (0-3) de la palette Color `palette` (0-7), en 0x00RRGGBB.
    fn cgb_color(palettes: &[u8], palette: u8, color: u8) -> u32 {
        let i = palette as usize * 8 + color as usize * 2;
        let rgb555 = u16::from_le_bytes([palettes[i], palettes[i + 1]]);
        // 5 bits → 8 bits, en recopiant les bits hauts dans les bits bas.
        let expand = |c: u16| {
            let c = (c & 0x1F) as u32;
            (c << 3) | (c >> 2)
        };
        expand(rgb555) << 16 | expand(rgb555 >> 5) << 8 | expand(rgb555 >> 10)
    }

    pub fn read_reg(&self, addr: u16) -> u8 {
        match addr {
            0xFF40 => self.lcdc,
            0xFF41 => {
                let coincidence = ((self.ly == self.lyc) as u8) << 2;
                let mode = if self.lcd_on() { self.mode } else { 0 };
                0x80 | self.stat | coincidence | mode
            }
            0xFF42 => self.scy,
            0xFF43 => self.scx,
            0xFF44 => self.ly,
            0xFF45 => self.lyc,
            0xFF47 => self.bgp,
            0xFF48 => self.obp0,
            0xFF49 => self.obp1,
            0xFF4A => self.wy,
            0xFF4B => self.wx,
            0xFF4F if self.cgb => 0xFE | self.vram_bank,
            0xFF68 if self.cgb => self.bcps | 0x40,
            0xFF69 if self.cgb => self.bg_palettes[(self.bcps & 0x3F) as usize],
            0xFF6A if self.cgb => self.ocps | 0x40,
            0xFF6B if self.cgb => self.obj_palettes[(self.ocps & 0x3F) as usize],
            _ => 0xFF,
        }
    }

    pub fn write_reg(&mut self, addr: u16, value: u8) {
        match addr {
            0xFF40 => {
                let was_on = self.lcd_on();
                self.lcdc = value;
                if was_on && !self.lcd_on() {
                    // Écran éteint : il reste blanc et LY revient à 0.
                    self.ly = 0;
                    self.dot = 0;
                    self.mode = 0;
                    self.window_line = 0;
                    let blank = self.blank_color();
                    self.front.fill(blank);
                } else if !was_on && self.lcd_on() {
                    self.mode = 2;
                }
            }
            0xFF41 => self.stat = value & 0x78,
            0xFF42 => self.scy = value,
            0xFF43 => self.scx = value,
            0xFF44 => {} // lecture seule
            0xFF45 => self.lyc = value,
            0xFF47 => self.bgp = value,
            0xFF48 => self.obp0 = value,
            0xFF49 => self.obp1 = value,
            0xFF4A => self.wy = value,
            0xFF4B => self.wx = value,
            0xFF4F if self.cgb => self.vram_bank = value & 0x01,
            0xFF68 if self.cgb => self.bcps = value & 0xBF,
            0xFF69 if self.cgb => {
                self.bg_palettes[(self.bcps & 0x3F) as usize] = value;
                self.bcps = auto_increment(self.bcps);
            }
            0xFF6A if self.cgb => self.ocps = value & 0xBF,
            0xFF6B if self.cgb => {
                self.obj_palettes[(self.ocps & 0x3F) as usize] = value;
                self.ocps = auto_increment(self.ocps);
            }
            _ => {}
        }
    }

    /// Avance de `dots` dots : 4 par M-cycle, 2 en double vitesse.
    /// Renvoie les interruptions à demander.
    pub fn tick(&mut self, dots: u32) -> u8 {
        if !self.lcd_on() {
            return 0;
        }
        let mut irq = 0;
        for _ in 0..dots {
            irq |= self.step_dot();
        }
        irq
    }

    fn step_dot(&mut self) -> u8 {
        let mut irq = 0;
        self.dot += 1;

        if self.ly < VBLANK_LINE {
            if self.dot == MODE3_START {
                self.mode = 3;
            } else if self.dot == HBLANK_START {
                self.render_line();
                self.mode = 0;
                self.hblank_started = true;
            }
        }

        if self.dot == DOTS_PER_LINE {
            self.dot = 0;
            self.ly += 1;
            if self.ly == VBLANK_LINE {
                self.mode = 1;
                irq |= INT_VBLANK;
                std::mem::swap(&mut self.front, &mut self.back);
            } else if self.ly == LINES_PER_FRAME {
                self.ly = 0;
                self.window_line = 0;
                self.mode = 2;
            } else if self.ly < VBLANK_LINE {
                self.mode = 2;
            }
        }

        irq | self.update_stat_line()
    }

    /// L'interruption STAT n'est déclenchée que quand la ligne passe de 0 à 1.
    fn update_stat_line(&mut self) -> u8 {
        let line = (self.stat & 0x08 != 0 && self.mode == 0)
            || (self.stat & 0x10 != 0 && self.mode == 1)
            || (self.stat & 0x20 != 0 && self.mode == 2)
            || (self.stat & 0x40 != 0 && self.ly == self.lyc);
        let rising = line && !self.stat_line;
        self.stat_line = line;
        if rising { INT_STAT } else { 0 }
    }

    /// Couleur (0 à 3) d'un pixel dans une tuile de 8x8 stockée à `addr` en VRAM.
    fn tile_pixel(&self, addr: usize, row: usize, col: usize) -> u8 {
        let lo = self.vram[addr + row * 2];
        let hi = self.vram[addr + row * 2 + 1];
        let bit = 7 - col;
        (((hi >> bit) & 1) << 1) | ((lo >> bit) & 1)
    }

    /// Adresse en VRAM d'une tuile de fond ou de fenêtre.
    fn bg_tile_addr(&self, index: u8) -> usize {
        if self.lcdc & LCDC_TILE_DATA != 0 {
            index as usize * 16
        } else {
            (0x1000 + index as i8 as isize * 16) as usize
        }
    }

    fn render_line(&mut self) {
        if self.cgb {
            self.render_line_cgb();
            return;
        }
        let ly = self.ly;
        // Couleurs brutes du fond, nécessaires pour la priorité des sprites.
        let mut bg_color = [0u8; SCREEN_WIDTH];
        let mut shades = [0u8; SCREEN_WIDTH];

        if self.lcdc & LCDC_BG_ON != 0 {
            let window_visible = self.lcdc & LCDC_WIN_ON != 0 && self.wy <= ly && self.wx <= 166;
            let window_x = self.wx as i16 - 7;
            let mut window_drawn = false;

            for x in 0..SCREEN_WIDTH {
                let (map_bit, px, py) = if window_visible && x as i16 >= window_x {
                    window_drawn = true;
                    (LCDC_WIN_MAP, (x as i16 - window_x) as u8, self.window_line)
                } else {
                    (
                        LCDC_BG_MAP,
                        (x as u8).wrapping_add(self.scx),
                        ly.wrapping_add(self.scy),
                    )
                };
                let map = if self.lcdc & map_bit != 0 {
                    0x1C00
                } else {
                    0x1800
                };
                let index = self.vram[map + (py as usize / 8) * 32 + px as usize / 8];
                let color =
                    self.tile_pixel(self.bg_tile_addr(index), py as usize % 8, px as usize % 8);
                bg_color[x] = color;
                shades[x] = (self.bgp >> (color * 2)) & 3;
            }

            if window_drawn {
                self.window_line += 1;
            }
        }

        if self.lcdc & LCDC_OBJ_ON != 0 {
            self.render_sprites(ly, &bg_color, &mut shades);
        }

        let row = &mut self.back[ly as usize * SCREEN_WIDTH..][..SCREEN_WIDTH];
        for (pixel, &shade) in row.iter_mut().zip(&shades) {
            *pixel = DMG_PALETTE[shade as usize];
        }
    }

    fn render_sprites(&self, ly: u8, bg_color: &[u8; SCREEN_WIDTH], shades: &mut [u8]) {
        let height: i16 = if self.lcdc & LCDC_OBJ_TALL != 0 {
            16
        } else {
            8
        };
        let line = ly as i16 + 16;

        // Au plus 10 sprites par ligne, pris dans l'ordre de l'OAM.
        let mut sprites = [(0u8, 0usize); 10];
        let mut count = 0;
        for i in 0..40 {
            let y = self.oam[i * 4] as i16;
            if line >= y && line < y + height {
                sprites[count] = (self.oam[i * 4 + 1], i);
                count += 1;
                if count == 10 {
                    break;
                }
            }
        }
        // Le sprite le plus à gauche gagne ; à égalité, celui qui vient en premier dans l'OAM.
        let sprites = &mut sprites[..count];
        sprites.sort_unstable();

        for (x, shade) in shades.iter_mut().enumerate() {
            let x = x as i16;
            for &(sprite_x, i) in sprites.iter() {
                let left = sprite_x as i16 - 8;
                if x < left || x >= left + 8 {
                    continue;
                }
                let y = self.oam[i * 4] as i16;
                let mut tile = self.oam[i * 4 + 2];
                let attr = self.oam[i * 4 + 3];
                if height == 16 {
                    tile &= 0xFE;
                }
                let mut row = line - y;
                if attr & 0x40 != 0 {
                    row = height - 1 - row;
                }
                let mut col = x - left;
                if attr & 0x20 != 0 {
                    col = 7 - col;
                }
                let color = self.tile_pixel(tile as usize * 16, row as usize, col as usize);
                if color == 0 {
                    continue; // transparent : on regarde le sprite suivant
                }
                // Bit 7 : le sprite passe derrière les couleurs 1 à 3 du fond.
                if attr & 0x80 == 0 || bg_color[x as usize] == 0 {
                    let palette = if attr & 0x10 != 0 {
                        self.obp1
                    } else {
                        self.obp0
                    };
                    *shade = (palette >> (color * 2)) & 3;
                }
                break;
            }
        }
    }
}

impl Ppu {
    fn render_line_cgb(&mut self) {
        let ly = self.ly;
        let mut bg_color = [0u8; SCREEN_WIDTH];
        let mut bg_priority = [false; SCREEN_WIDTH];
        let mut pixels = [0u32; SCREEN_WIDTH];

        // En mode Color, le fond est toujours dessiné : le bit 0 de LCDC
        // décide seulement si le fond peut passer devant les sprites.
        let window_visible = self.lcdc & LCDC_WIN_ON != 0 && self.wy <= ly && self.wx <= 166;
        let window_x = self.wx as i16 - 7;
        let mut window_drawn = false;

        for x in 0..SCREEN_WIDTH {
            let (map_bit, px, py) = if window_visible && x as i16 >= window_x {
                window_drawn = true;
                (LCDC_WIN_MAP, (x as i16 - window_x) as u8, self.window_line)
            } else {
                (
                    LCDC_BG_MAP,
                    (x as u8).wrapping_add(self.scx),
                    ly.wrapping_add(self.scy),
                )
            };
            let map = if self.lcdc & map_bit != 0 {
                0x1C00
            } else {
                0x1800
            };
            let offset = map + (py as usize / 8) * 32 + px as usize / 8;
            let index = self.vram[offset];
            // Les attributs sont au même endroit, dans la banque 1.
            let attr = self.vram[0x2000 + offset];

            let mut row = py as usize % 8;
            if attr & 0x40 != 0 {
                row = 7 - row;
            }
            let mut col = px as usize % 8;
            if attr & 0x20 != 0 {
                col = 7 - col;
            }
            let bank = ((attr >> 3) & 1) as usize * 0x2000;
            let color = self.tile_pixel(bank + self.bg_tile_addr(index), row, col);

            bg_color[x] = color;
            bg_priority[x] = attr & 0x80 != 0;
            pixels[x] = Self::cgb_color(&self.bg_palettes, attr & 0x07, color);
        }
        if window_drawn {
            self.window_line += 1;
        }

        if self.lcdc & LCDC_OBJ_ON != 0 {
            self.render_sprites_cgb(ly, &bg_color, &bg_priority, &mut pixels);
        }

        self.back[ly as usize * SCREEN_WIDTH..][..SCREEN_WIDTH].copy_from_slice(&pixels);
    }

    fn render_sprites_cgb(
        &self,
        ly: u8,
        bg_color: &[u8; SCREEN_WIDTH],
        bg_priority: &[bool; SCREEN_WIDTH],
        pixels: &mut [u32; SCREEN_WIDTH],
    ) {
        let height: i16 = if self.lcdc & LCDC_OBJ_TALL != 0 {
            16
        } else {
            8
        };
        let line = ly as i16 + 16;
        // Sans le bit 0 de LCDC, les sprites passent toujours devant le fond.
        let bg_can_win = self.lcdc & LCDC_BG_ON != 0;

        // Au plus 10 sprites par ligne. En mode Color, la priorité suit
        // uniquement l'ordre dans l'OAM (pas la position horizontale).
        let mut sprites = [0usize; 10];
        let mut count = 0;
        for i in 0..40 {
            let y = self.oam[i * 4] as i16;
            if line >= y && line < y + height {
                sprites[count] = i;
                count += 1;
                if count == 10 {
                    break;
                }
            }
        }

        for (x, pixel) in pixels.iter_mut().enumerate() {
            let sx = x as i16;
            for &i in &sprites[..count] {
                let left = self.oam[i * 4 + 1] as i16 - 8;
                if sx < left || sx >= left + 8 {
                    continue;
                }
                let y = self.oam[i * 4] as i16;
                let mut tile = self.oam[i * 4 + 2];
                let attr = self.oam[i * 4 + 3];
                if height == 16 {
                    tile &= 0xFE;
                }
                let mut row = line - y;
                if attr & 0x40 != 0 {
                    row = height - 1 - row;
                }
                let mut col = sx - left;
                if attr & 0x20 != 0 {
                    col = 7 - col;
                }
                let bank = ((attr >> 3) & 1) as usize * 0x2000;
                let color = self.tile_pixel(bank + tile as usize * 16, row as usize, col as usize);
                if color == 0 {
                    continue;
                }
                let behind = attr & 0x80 != 0 || bg_priority[x];
                if !bg_can_win || bg_color[x] == 0 || !behind {
                    *pixel = Self::cgb_color(&self.obj_palettes, attr & 0x07, color);
                }
                break;
            }
        }
    }
}

/// Avance l'index d'une palette si l'auto-incrément (bit 7) est actif.
fn auto_increment(index: u8) -> u8 {
    if index & 0x80 != 0 {
        0x80 | ((index + 1) & 0x3F)
    } else {
        index
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_dots(ppu: &mut Ppu, dots: u32) -> u8 {
        let mut irq = 0;
        for _ in 0..dots / 4 {
            irq |= ppu.tick(4);
        }
        irq
    }

    fn pixel(ppu: &Ppu, x: usize, y: usize) -> u32 {
        ppu.frame()[y * SCREEN_WIDTH + x]
    }

    /// Écrit une couleur RGB 15 bits dans une palette Color.
    fn set_cgb_color(ppu: &mut Ppu, obj: bool, palette: u8, color: u8, rgb555: u16) {
        let (index_reg, data_reg) = if obj {
            (0xFF6A, 0xFF6B)
        } else {
            (0xFF68, 0xFF69)
        };
        ppu.write_reg(index_reg, 0x80 | (palette * 8 + color * 2));
        let [lo, hi] = rgb555.to_le_bytes();
        ppu.write_reg(data_reg, lo);
        ppu.write_reg(data_reg, hi);
    }

    const RED: u16 = 0x001F;
    const BLUE: u16 = 0x7C00;

    #[test]
    fn cgb_palette_index_auto_increments() {
        let mut ppu = Ppu::with_mode(true);
        ppu.write_reg(0xFF68, 0x80 | 0x3F);
        ppu.write_reg(0xFF69, 0x12);
        // Après 0x3F, l'index revient à 0 en gardant l'auto-incrément.
        assert_eq!(ppu.read_reg(0xFF68), 0x80 | 0x40);
        ppu.write_reg(0xFF68, 0x3F);
        assert_eq!(ppu.read_reg(0xFF69), 0x12);
    }

    #[test]
    fn cgb_vram_has_two_banks() {
        let mut ppu = Ppu::with_mode(true);
        ppu.write_vram(0x8000, 0x11);
        ppu.write_reg(0xFF4F, 1);
        assert_eq!(ppu.read_vram(0x8000), 0x00);
        ppu.write_vram(0x8000, 0x22);
        ppu.write_reg(0xFF4F, 0);
        assert_eq!(ppu.read_vram(0x8000), 0x11);
        assert_eq!(ppu.vram[0x2000], 0x22);
        // En mode classique, VBK n'existe pas.
        assert_eq!(Ppu::new().read_reg(0xFF4F), 0xFF);
    }

    #[test]
    fn cgb_background_uses_tile_attributes() {
        let mut ppu = Ppu::with_mode(true);
        set_cgb_color(&mut ppu, false, 3, 1, RED);
        // Tuile 0 en banque 1 : couleur 1 partout. La tuile 0 de la banque 0 reste vide.
        for row in 0..8 {
            ppu.vram[0x2000 + row * 2] = 0xFF;
        }
        // Attribut de la première case de la carte : palette 3, tuile en banque 1.
        ppu.vram[0x2000 + 0x1800] = 0x08 | 0x03;
        run_dots(&mut ppu, 456 * 144);
        assert_eq!(pixel(&ppu, 0, 0), 0x00FF_0000);
        // La case suivante n'a pas d'attribut : tuile vide, palette 0 blanche.
        assert_eq!(pixel(&ppu, 8, 0), 0x00FF_FFFF);
    }

    #[test]
    fn cgb_sprite_priority_follows_oam_order() {
        let mut ppu = Ppu::with_mode(true);
        ppu.write_reg(0xFF40, 0x93);
        set_cgb_color(&mut ppu, true, 0, 3, RED);
        set_cgb_color(&mut ppu, true, 1, 3, BLUE);
        for row in 0..8 {
            ppu.vram[16 + row * 2] = 0xFF;
            ppu.vram[16 + row * 2 + 1] = 0xFF;
        }
        // Le sprite 0 est plus à droite mais passe quand même devant (ordre OAM).
        ppu.oam[..8].copy_from_slice(&[16, 12, 1, 0x00, 16, 8, 1, 0x01]);
        run_dots(&mut ppu, 456 * 144);
        assert_eq!(pixel(&ppu, 4, 0), 0x00FF_0000, "sprite 0 devant");
        assert_eq!(
            pixel(&ppu, 0, 0),
            0x0000_00FF,
            "seul le sprite 1 couvre x = 0"
        );
    }

    #[test]
    fn cgb_bg_priority_attribute_hides_sprites() {
        let mut ppu = Ppu::with_mode(true);
        ppu.write_reg(0xFF40, 0x93);
        set_cgb_color(&mut ppu, false, 0, 1, BLUE);
        set_cgb_color(&mut ppu, true, 0, 3, RED);
        for row in 0..8 {
            ppu.vram[row * 2] = 0xFF; // fond : couleur 1
            ppu.vram[16 + row * 2] = 0xFF; // sprite : couleur 3
            ppu.vram[16 + row * 2 + 1] = 0xFF;
        }
        ppu.vram[0x2000 + 0x1800] = 0x80; // case prioritaire
        ppu.oam[..4].copy_from_slice(&[16, 8, 1, 0]);
        run_dots(&mut ppu, 456 * 144);
        assert_eq!(pixel(&ppu, 0, 0), 0x0000_00FF, "le fond passe devant");

        // Sans le bit 0 de LCDC, le sprite repasse devant.
        let mut ppu2 = Ppu::with_mode(true);
        ppu2.write_reg(0xFF40, 0x92);
        ppu2.vram.copy_from_slice(&ppu.vram);
        ppu2.oam.copy_from_slice(&ppu.oam);
        ppu2.bg_palettes.copy_from_slice(&ppu.bg_palettes);
        ppu2.obj_palettes.copy_from_slice(&ppu.obj_palettes);
        run_dots(&mut ppu2, 456 * 144);
        assert_eq!(pixel(&ppu2, 0, 0), 0x00FF_0000);
    }

    #[test]
    fn line_modes_follow_timing() {
        let mut ppu = Ppu::new();
        assert_eq!(ppu.read_reg(0xFF41) & 3, 2);
        run_dots(&mut ppu, 80);
        assert_eq!(ppu.read_reg(0xFF41) & 3, 3);
        run_dots(&mut ppu, 172);
        assert_eq!(ppu.read_reg(0xFF41) & 3, 0);
        run_dots(&mut ppu, 204);
        assert_eq!(ppu.read_reg(0xFF44), 1);
        assert_eq!(ppu.read_reg(0xFF41) & 3, 2);
    }

    #[test]
    fn vblank_interrupt_and_full_frame() {
        let mut ppu = Ppu::new();
        let irq = run_dots(&mut ppu, 456 * 144);
        assert_eq!(irq & INT_VBLANK, INT_VBLANK);
        assert_eq!(ppu.read_reg(0xFF41) & 3, 1);
        run_dots(&mut ppu, 456 * 10);
        assert_eq!(ppu.read_reg(0xFF44), 0);
    }

    #[test]
    fn lyc_triggers_stat_interrupt() {
        let mut ppu = Ppu::new();
        ppu.write_reg(0xFF45, 3);
        ppu.write_reg(0xFF41, 0x40);
        assert_eq!(run_dots(&mut ppu, 456 * 2) & INT_STAT, 0);
        assert_eq!(run_dots(&mut ppu, 456) & INT_STAT, INT_STAT);
        assert_ne!(ppu.read_reg(0xFF41) & 0x04, 0);
    }

    #[test]
    fn lcd_off_resets_ly() {
        let mut ppu = Ppu::new();
        run_dots(&mut ppu, 456 * 5);
        ppu.write_reg(0xFF40, 0x11);
        assert_eq!(ppu.read_reg(0xFF44), 0);
        assert_eq!(ppu.read_reg(0xFF41) & 3, 0);
        run_dots(&mut ppu, 456 * 5);
        assert_eq!(ppu.read_reg(0xFF44), 0);
    }

    #[test]
    fn renders_background_tile() {
        let mut ppu = Ppu::new();
        ppu.write_reg(0xFF47, 0xE4); // palette identité : couleur n → teinte n
        // Tuile 0 : toutes ses lignes en couleur 1. La carte 0x9800 est remplie de 0.
        for row in 0..8 {
            ppu.vram[row * 2] = 0xFF;
        }
        run_dots(&mut ppu, 456 * 144);
        assert_eq!(pixel(&ppu, 0, 0), DMG_PALETTE[1]);
        assert_eq!(pixel(&ppu, 159, 143), DMG_PALETTE[1]);
    }

    #[test]
    fn renders_sprite_over_background() {
        let mut ppu = Ppu::new();
        ppu.write_reg(0xFF40, 0x93); // écran + fond + sprites
        ppu.write_reg(0xFF47, 0xE4);
        ppu.write_reg(0xFF48, 0xE4);
        // Tuile 1 : seul le pixel de gauche est opaque, en couleur 3.
        for row in 0..8 {
            ppu.vram[16 + row * 2] = 0x80;
            ppu.vram[16 + row * 2 + 1] = 0x80;
        }
        ppu.oam[..4].copy_from_slice(&[16, 8, 1, 0]); // sprite en (0,0), tuile 1
        run_dots(&mut ppu, 456 * 144);
        assert_eq!(pixel(&ppu, 0, 0), DMG_PALETTE[3]);
        assert_eq!(pixel(&ppu, 1, 0), DMG_PALETTE[0]);
        assert_eq!(pixel(&ppu, 0, 8), DMG_PALETTE[0]);
    }

    #[test]
    fn sprite_behind_background_when_flagged() {
        let mut ppu = Ppu::new();
        ppu.write_reg(0xFF40, 0x93);
        ppu.write_reg(0xFF47, 0xE4);
        ppu.write_reg(0xFF48, 0xE4);
        for row in 0..8 {
            ppu.vram[row * 2] = 0xFF; // fond en couleur 1
            ppu.vram[16 + row * 2 + 1] = 0xFF; // sprite en couleur 2
        }
        ppu.oam[..4].copy_from_slice(&[16, 8, 1, 0x80]);
        run_dots(&mut ppu, 456 * 144);
        assert_eq!(pixel(&ppu, 0, 0), DMG_PALETTE[1]);
    }
}
