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
    pub vram: Vec<u8>,
    pub oam: Vec<u8>,

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
        let blank = vec![DMG_PALETTE[0]; SCREEN_WIDTH * SCREEN_HEIGHT];
        Ppu {
            vram: vec![0; 0x2000],
            oam: vec![0; 0xA0],
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
                    self.front.fill(DMG_PALETTE[0]);
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
            _ => {}
        }
    }

    /// Avance d'un M-cycle (4 dots). Renvoie les interruptions à demander.
    pub fn tick(&mut self) -> u8 {
        if !self.lcd_on() {
            return 0;
        }
        let mut irq = 0;
        for _ in 0..4 {
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
            irq |= ppu.tick();
        }
        irq
    }

    fn pixel(ppu: &Ppu, x: usize, y: usize) -> u32 {
        ppu.frame()[y * SCREEN_WIDTH + x]
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
