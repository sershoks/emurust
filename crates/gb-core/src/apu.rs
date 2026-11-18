//! APU (Audio Processing Unit) : le son de la Game Boy.
//!
//! Quatre canaux :
//! 1. onde carrée avec balayage de fréquence (sweep)
//! 2. onde carrée
//! 3. onde libre, lue dans 32 échantillons de 4 bits (wave RAM)
//! 4. bruit, généré par un registre à décalage (LFSR)
//!
//! Un séquenceur à 512 Hz cadence la durée des notes (length),
//! les enveloppes de volume et le sweep. Les canaux sont ensuite mixés
//! en stéréo et rééchantillonnés à la fréquence de la carte son.

use serde::{Deserialize, Serialize};

const CPU_HZ: u64 = 4_194_304;
/// Le séquenceur avance tous les 8192 T-cycles (512 Hz).
const FRAME_SEQUENCER_PERIOD: u32 = 8192;

const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12,5 %
    [1, 0, 0, 0, 0, 0, 0, 1], // 25 %
    [1, 0, 0, 0, 0, 1, 1, 1], // 50 %
    [0, 1, 1, 1, 1, 1, 1, 0], // 75 %
];

/// Valeurs « OU » appliquées à la lecture des registres FF10-FF2F :
/// les bits non lisibles reviennent toujours à 1.
const READ_MASK: [u8; 0x20] = [
    0x80, 0x3F, 0x00, 0xFF, 0xBF, // NR10-NR14
    0xFF, 0x3F, 0x00, 0xFF, 0xBF, // ----, NR21-NR24
    0x7F, 0xFF, 0x9F, 0xFF, 0xBF, // NR30-NR34
    0xFF, 0xFF, 0x00, 0x00, 0xBF, // ----, NR41-NR44
    0x00, 0x00, 0x70, // NR50-NR52
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// Compteur de durée commun à tous les canaux.
#[derive(Default, Clone, Serialize, Deserialize)]
struct Length {
    counter: u16,
    enabled: bool,
}

impl Length {
    /// Renvoie `false` quand la note est terminée.
    fn clock(&mut self) -> bool {
        if self.enabled && self.counter > 0 {
            self.counter -= 1;
            return self.counter != 0;
        }
        true
    }

    fn trigger(&mut self, max: u16) {
        if self.counter == 0 {
            self.counter = max;
        }
    }
}

/// Enveloppe de volume des canaux 1, 2 et 4.
#[derive(Default, Clone, Serialize, Deserialize)]
struct Envelope {
    initial: u8,
    up: bool,
    period: u8,
    volume: u8,
    timer: u8,
}

impl Envelope {
    fn write(&mut self, value: u8) {
        self.initial = value >> 4;
        self.up = value & 0x08 != 0;
        self.period = value & 0x07;
    }

    /// Le DAC est éteint quand les 5 bits hauts de NRx2 valent 0.
    fn dac_on(&self) -> bool {
        self.initial != 0 || self.up
    }

    fn trigger(&mut self) {
        self.volume = self.initial;
        self.timer = self.period;
    }

    fn clock(&mut self) {
        if self.period == 0 {
            return;
        }
        self.timer = self.timer.saturating_sub(1);
        if self.timer == 0 {
            self.timer = self.period;
            if self.up && self.volume < 15 {
                self.volume += 1;
            } else if !self.up && self.volume > 0 {
                self.volume -= 1;
            }
        }
    }
}

/// Canaux 1 et 2 : onde carrée.
#[derive(Default, Clone, Serialize, Deserialize)]
struct Square {
    enabled: bool,
    duty: u8,
    duty_pos: u8,
    freq: u16,
    timer: i32,
    length: Length,
    envelope: Envelope,

    // Sweep (canal 1 uniquement)
    sweep_period: u8,
    sweep_negate: bool,
    sweep_shift: u8,
    sweep_timer: u8,
    sweep_enabled: bool,
    shadow_freq: u16,
}

impl Square {
    fn period(&self) -> i32 {
        (2048 - self.freq as i32) * 4
    }

    fn step(&mut self, cycles: i32) {
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period();
            self.duty_pos = (self.duty_pos + 1) & 7;
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && DUTY_TABLE[self.duty as usize][self.duty_pos as usize] == 1 {
            self.envelope.volume
        } else {
            0
        }
    }

    fn trigger(&mut self) {
        self.enabled = self.envelope.dac_on();
        self.length.trigger(64);
        self.timer = self.period();
        self.envelope.trigger();

        self.shadow_freq = self.freq;
        self.sweep_timer = if self.sweep_period == 0 {
            8
        } else {
            self.sweep_period
        };
        self.sweep_enabled = self.sweep_period != 0 || self.sweep_shift != 0;
        if self.sweep_shift != 0 {
            self.sweep_calc();
        }
    }

    /// Calcule la prochaine fréquence du sweep ; coupe le canal si elle déborde.
    fn sweep_calc(&mut self) -> u16 {
        let delta = self.shadow_freq >> self.sweep_shift;
        let freq = if self.sweep_negate {
            self.shadow_freq.wrapping_sub(delta)
        } else {
            self.shadow_freq + delta
        };
        if freq > 2047 {
            self.enabled = false;
        }
        freq
    }

    fn clock_sweep(&mut self) {
        self.sweep_timer = self.sweep_timer.saturating_sub(1);
        if self.sweep_timer != 0 {
            return;
        }
        self.sweep_timer = if self.sweep_period == 0 {
            8
        } else {
            self.sweep_period
        };
        if self.sweep_enabled && self.sweep_period != 0 {
            let freq = self.sweep_calc();
            if freq <= 2047 && self.sweep_shift != 0 {
                self.shadow_freq = freq;
                self.freq = freq;
                self.sweep_calc();
            }
        }
    }
}

/// Canal 3 : onde libre.
#[derive(Default, Clone, Serialize, Deserialize)]
struct Wave {
    enabled: bool,
    dac_on: bool,
    volume_code: u8,
    freq: u16,
    timer: i32,
    position: u8,
    length: Length,
    ram: [u8; 16],
}

impl Wave {
    fn period(&self) -> i32 {
        (2048 - self.freq as i32) * 2
    }

    fn step(&mut self, cycles: i32) {
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period();
            self.position = (self.position + 1) & 31;
        }
    }

    fn output(&self) -> u8 {
        if !self.enabled || self.volume_code == 0 {
            return 0;
        }
        let byte = self.ram[(self.position / 2) as usize];
        let sample = if self.position.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0x0F
        };
        sample >> (self.volume_code - 1)
    }

    fn trigger(&mut self) {
        self.enabled = self.dac_on;
        self.length.trigger(256);
        self.timer = self.period();
        self.position = 0;
    }
}

/// Canal 4 : bruit.
#[derive(Default, Clone, Serialize, Deserialize)]
struct Noise {
    enabled: bool,
    shift: u8,
    narrow: bool,
    divisor_code: u8,
    timer: i32,
    lfsr: u16,
    length: Length,
    envelope: Envelope,
}

impl Noise {
    fn period(&self) -> i32 {
        let divisor = if self.divisor_code == 0 {
            8
        } else {
            self.divisor_code as i32 * 16
        };
        divisor << self.shift
    }

    fn step(&mut self, cycles: i32) {
        self.timer -= cycles;
        while self.timer <= 0 {
            self.timer += self.period();
            let bit = (self.lfsr ^ (self.lfsr >> 1)) & 1;
            self.lfsr = (self.lfsr >> 1) | (bit << 14);
            if self.narrow {
                self.lfsr = (self.lfsr & !(1 << 6)) | (bit << 6);
            }
        }
    }

    fn output(&self) -> u8 {
        if self.enabled && self.lfsr & 1 == 0 {
            self.envelope.volume
        } else {
            0
        }
    }

    fn trigger(&mut self) {
        self.enabled = self.envelope.dac_on();
        self.length.trigger(64);
        self.timer = self.period();
        self.envelope.trigger();
        self.lfsr = 0x7FFF;
    }
}

#[derive(Serialize, Deserialize)]
pub struct Apu {
    /// Copie brute des registres FF10-FF2F, pour la relecture.
    regs: [u8; 0x20],
    powered: bool,
    ch1: Square,
    ch2: Square,
    ch3: Wave,
    ch4: Noise,

    sequencer_timer: u32,
    sequencer_step: u8,

    sample_rate: u64,
    /// Accumulateur pour savoir quand produire un échantillon.
    sample_clock: u64,
    /// Somme des sorties depuis le dernier échantillon (filtre moyenneur).
    sum_left: f32,
    sum_right: f32,
    sum_count: u32,
    /// Filtres passe-haut qui retirent la composante continue, comme le condensateur de la console.
    cap_left: f32,
    cap_right: f32,
    /// Échantillons stéréo entrelacés (gauche, droite) prêts à être joués.
    #[serde(skip)]
    samples: Vec<f32>,
}

impl Apu {
    pub fn new() -> Self {
        let mut apu = Apu {
            regs: [0; 0x20],
            powered: true,
            ch1: Square::default(),
            ch2: Square::default(),
            ch3: Wave::default(),
            ch4: Noise::default(),
            sequencer_timer: FRAME_SEQUENCER_PERIOD,
            sequencer_step: 0,
            sample_rate: 48_000,
            sample_clock: 0,
            sum_left: 0.0,
            sum_right: 0.0,
            sum_count: 0,
            cap_left: 0.0,
            cap_right: 0.0,
            samples: Vec::new(),
        };
        // Valeurs laissées par la boot ROM.
        apu.write(0xFF24, 0x77);
        apu.write(0xFF25, 0xF3);
        apu
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate.max(1) as u64;
    }

    /// Récupère les échantillons produits depuis le dernier appel.
    pub fn drain_samples(&mut self, out: &mut Vec<f32>) {
        out.append(&mut self.samples);
    }

    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            0xFF26 => {
                (self.powered as u8) << 7
                    | 0x70
                    | (self.ch4.enabled as u8) << 3
                    | (self.ch3.enabled as u8) << 2
                    | (self.ch2.enabled as u8) << 1
                    | self.ch1.enabled as u8
            }
            0xFF10..=0xFF2F => {
                let i = (addr - 0xFF10) as usize;
                self.regs[i] | READ_MASK[i]
            }
            0xFF30..=0xFF3F => self.ch3.ram[(addr - 0xFF30) as usize],
            _ => 0xFF,
        }
    }

    pub fn write(&mut self, addr: u16, value: u8) {
        if let 0xFF30..=0xFF3F = addr {
            self.ch3.ram[(addr - 0xFF30) as usize] = value;
            return;
        }
        if addr == 0xFF26 {
            let on = value & 0x80 != 0;
            if self.powered && !on {
                self.power_off();
            } else if !self.powered && on {
                self.sequencer_step = 0;
            }
            self.powered = on;
            return;
        }
        if !self.powered || !(0xFF10..=0xFF25).contains(&addr) {
            return;
        }
        self.regs[(addr - 0xFF10) as usize] = value;

        match addr {
            // Canal 1
            0xFF10 => {
                self.ch1.sweep_period = (value >> 4) & 0x07;
                self.ch1.sweep_negate = value & 0x08 != 0;
                self.ch1.sweep_shift = value & 0x07;
            }
            0xFF11 => {
                self.ch1.duty = value >> 6;
                self.ch1.length.counter = 64 - (value & 0x3F) as u16;
            }
            0xFF12 => {
                self.ch1.envelope.write(value);
                if !self.ch1.envelope.dac_on() {
                    self.ch1.enabled = false;
                }
            }
            0xFF13 => self.ch1.freq = (self.ch1.freq & 0x700) | value as u16,
            0xFF14 => {
                self.ch1.freq = (self.ch1.freq & 0xFF) | ((value as u16 & 0x07) << 8);
                self.ch1.length.enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch1.trigger();
                }
            }

            // Canal 2
            0xFF16 => {
                self.ch2.duty = value >> 6;
                self.ch2.length.counter = 64 - (value & 0x3F) as u16;
            }
            0xFF17 => {
                self.ch2.envelope.write(value);
                if !self.ch2.envelope.dac_on() {
                    self.ch2.enabled = false;
                }
            }
            0xFF18 => self.ch2.freq = (self.ch2.freq & 0x700) | value as u16,
            0xFF19 => {
                self.ch2.freq = (self.ch2.freq & 0xFF) | ((value as u16 & 0x07) << 8);
                self.ch2.length.enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch2.trigger();
                }
            }

            // Canal 3
            0xFF1A => {
                self.ch3.dac_on = value & 0x80 != 0;
                if !self.ch3.dac_on {
                    self.ch3.enabled = false;
                }
            }
            0xFF1B => self.ch3.length.counter = 256 - value as u16,
            0xFF1C => self.ch3.volume_code = (value >> 5) & 0x03,
            0xFF1D => self.ch3.freq = (self.ch3.freq & 0x700) | value as u16,
            0xFF1E => {
                self.ch3.freq = (self.ch3.freq & 0xFF) | ((value as u16 & 0x07) << 8);
                self.ch3.length.enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch3.trigger();
                }
            }

            // Canal 4
            0xFF20 => self.ch4.length.counter = 64 - (value & 0x3F) as u16,
            0xFF21 => {
                self.ch4.envelope.write(value);
                if !self.ch4.envelope.dac_on() {
                    self.ch4.enabled = false;
                }
            }
            0xFF22 => {
                self.ch4.shift = value >> 4;
                self.ch4.narrow = value & 0x08 != 0;
                self.ch4.divisor_code = value & 0x07;
            }
            0xFF23 => {
                self.ch4.length.enabled = value & 0x40 != 0;
                if value & 0x80 != 0 {
                    self.ch4.trigger();
                }
            }

            _ => {}
        }
    }

    /// Couper l'APU efface tous ses registres (sauf la wave RAM).
    fn power_off(&mut self) {
        let wave_ram = self.ch3.ram;
        self.regs = [0; 0x20];
        self.ch1 = Square::default();
        self.ch2 = Square::default();
        self.ch3 = Wave {
            ram: wave_ram,
            ..Wave::default()
        };
        self.ch4 = Noise::default();
    }

    fn clock_sequencer(&mut self) {
        let step = self.sequencer_step;
        if step.is_multiple_of(2) {
            self.ch1.enabled &= self.ch1.length.clock();
            self.ch2.enabled &= self.ch2.length.clock();
            self.ch3.enabled &= self.ch3.length.clock();
            self.ch4.enabled &= self.ch4.length.clock();
        }
        if step == 2 || step == 6 {
            self.ch1.clock_sweep();
        }
        if step == 7 {
            self.ch1.envelope.clock();
            self.ch2.envelope.clock();
            self.ch4.envelope.clock();
        }
        self.sequencer_step = (step + 1) & 7;
    }

    /// Avance d'un M-cycle (4 T-cycles).
    pub fn tick(&mut self) {
        if self.powered {
            self.ch1.step(4);
            self.ch2.step(4);
            self.ch3.step(4);
            self.ch4.step(4);

            self.sequencer_timer -= 4;
            if self.sequencer_timer == 0 {
                self.sequencer_timer = FRAME_SEQUENCER_PERIOD;
                self.clock_sequencer();
            }
        }

        let (left, right) = self.mix();
        self.sum_left += left;
        self.sum_right += right;
        self.sum_count += 1;

        self.sample_clock += 4 * self.sample_rate;
        if self.sample_clock >= CPU_HZ {
            self.sample_clock -= CPU_HZ;
            self.push_sample();
        }
    }

    /// Sortie analogique d'un canal : de -1,0 à 1,0, ou 0 si son DAC est éteint.
    fn dac(dac_on: bool, digital: u8) -> f32 {
        if dac_on {
            1.0 - digital as f32 / 7.5
        } else {
            0.0
        }
    }

    fn mix(&self) -> (f32, f32) {
        if !self.powered {
            return (0.0, 0.0);
        }
        let channels = [
            Self::dac(self.ch1.envelope.dac_on(), self.ch1.output()),
            Self::dac(self.ch2.envelope.dac_on(), self.ch2.output()),
            Self::dac(self.ch3.dac_on, self.ch3.output()),
            Self::dac(self.ch4.envelope.dac_on(), self.ch4.output()),
        ];
        let panning = self.regs[0x15]; // NR51
        let volume = self.regs[0x14]; // NR50
        let (mut left, mut right) = (0.0, 0.0);
        for (i, &sample) in channels.iter().enumerate() {
            if panning & (0x10 << i) != 0 {
                left += sample;
            }
            if panning & (0x01 << i) != 0 {
                right += sample;
            }
        }
        let left_volume = ((volume >> 4) & 0x07) as f32 + 1.0;
        let right_volume = (volume & 0x07) as f32 + 1.0;
        (left * left_volume / 32.0, right * right_volume / 32.0)
    }

    fn push_sample(&mut self) {
        let count = self.sum_count.max(1) as f32;
        let left = self.sum_left / count;
        let right = self.sum_right / count;
        self.sum_left = 0.0;
        self.sum_right = 0.0;
        self.sum_count = 0;

        // Passe-haut : la constante dépend de la fréquence d'échantillonnage.
        let charge = 0.999_958_f32.powf(CPU_HZ as f32 / self.sample_rate as f32);
        let out_left = left - self.cap_left;
        self.cap_left = left - out_left * charge;
        let out_right = right - self.cap_right;
        self.cap_right = right - out_right * charge;

        // Sans personne pour les lire, on évite d'accumuler sans fin (≈ 1 s max).
        if self.samples.len() > self.sample_rate as usize * 2 {
            self.samples.clear();
        }
        self.samples.push(out_left);
        self.samples.push(out_right);
    }
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_cycles(apu: &mut Apu, t_cycles: u32) {
        for _ in 0..t_cycles / 4 {
            apu.tick();
        }
    }

    #[test]
    fn produces_samples_at_requested_rate() {
        let mut apu = Apu::new();
        apu.set_sample_rate(48_000);
        run_cycles(&mut apu, CPU_HZ as u32);
        let mut out = Vec::new();
        apu.drain_samples(&mut out);
        // Une seconde émulée = 48 000 échantillons stéréo.
        assert!((out.len() as i32 / 2 - 48_000).abs() <= 1, "{}", out.len());
    }

    #[test]
    fn trigger_enables_channel_and_nr52_reports_it() {
        let mut apu = Apu::new();
        apu.write(0xFF12, 0xF0); // volume max, DAC allumé
        apu.write(0xFF14, 0x80); // déclenchement
        assert_eq!(apu.read(0xFF26) & 0x01, 0x01);
    }

    #[test]
    fn dac_off_prevents_trigger() {
        let mut apu = Apu::new();
        apu.write(0xFF17, 0x00);
        apu.write(0xFF19, 0x80);
        assert_eq!(apu.read(0xFF26) & 0x02, 0);
    }

    #[test]
    fn length_counter_stops_channel() {
        let mut apu = Apu::new();
        apu.write(0xFF17, 0xF0);
        apu.write(0xFF16, 0x3F); // durée = 1 pas
        apu.write(0xFF19, 0xC0); // déclenchement + durée activée
        assert_eq!(apu.read(0xFF26) & 0x02, 0x02);
        run_cycles(&mut apu, FRAME_SEQUENCER_PERIOD * 2);
        assert_eq!(apu.read(0xFF26) & 0x02, 0);
    }

    #[test]
    fn envelope_decreases_volume() {
        let mut apu = Apu::new();
        apu.write(0xFF12, 0xF1); // volume 15, descendant, période 1
        apu.write(0xFF14, 0x80);
        run_cycles(&mut apu, FRAME_SEQUENCER_PERIOD * 8);
        assert_eq!(apu.ch1.envelope.volume, 14);
    }

    #[test]
    fn sweep_overflow_disables_channel() {
        let mut apu = Apu::new();
        apu.write(0xFF10, 0x11); // période 1, addition, décalage 1
        apu.write(0xFF12, 0xF0);
        apu.write(0xFF13, 0xFF);
        apu.write(0xFF14, 0x87); // fréquence 0x7FF : le premier calcul déborde
        assert_eq!(apu.read(0xFF26) & 0x01, 0);
    }

    #[test]
    fn read_masks_unused_bits() {
        let apu = Apu::new();
        assert_eq!(apu.read(0xFF11), 0x3F | apu.regs[1]);
        assert_eq!(apu.read(0xFF13), 0xFF);
        assert_eq!(apu.read(0xFF26) & 0x70, 0x70);
    }

    #[test]
    fn power_off_clears_registers_but_keeps_wave_ram() {
        let mut apu = Apu::new();
        apu.write(0xFF30, 0xAB);
        apu.write(0xFF12, 0xF0);
        apu.write(0xFF14, 0x80);
        apu.write(0xFF26, 0x00);
        assert_eq!(apu.read(0xFF26), 0x70);
        assert_eq!(apu.read(0xFF12), 0x00);
        assert_eq!(apu.read(0xFF30), 0xAB);
        // Les écritures sont ignorées tant que l'APU est éteint.
        apu.write(0xFF12, 0xF0);
        assert_eq!(apu.read(0xFF12), 0x00);
    }

    #[test]
    fn square_wave_is_audible() {
        let mut apu = Apu::new();
        apu.write(0xFF25, 0x22); // canal 2 à gauche et à droite
        apu.write(0xFF16, 0x80); // rapport cyclique 50 %
        apu.write(0xFF17, 0xF0);
        apu.write(0xFF18, 0x00);
        apu.write(0xFF19, 0x87); // ≈ 1 kHz
        run_cycles(&mut apu, CPU_HZ as u32 / 10);
        let mut out = Vec::new();
        apu.drain_samples(&mut out);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.1, "pic = {peak}");
    }
}
