//! CPU Sharp SM83, le processeur de la Game Boy.
//!
//! Le timing est calculé accès par accès : chaque lecture ou écriture mémoire
//! coûte un M-cycle (4 T-cycles), et les cycles internes sont ajoutés
//! explicitement avec `tick`. Le reste de la machine (timer, écran...) avance
//! donc en même temps que le CPU, au cycle près.

pub const FLAG_Z: u8 = 0x80;
pub const FLAG_N: u8 = 0x40;
pub const FLAG_H: u8 = 0x20;
pub const FLAG_C: u8 = 0x10;

const IE_ADDR: u16 = 0xFFFF;
const IF_ADDR: u16 = 0xFF0F;

/// Ce que le CPU voit du reste de la machine.
pub trait Bus {
    fn read(&mut self, addr: u16) -> u8;
    fn write(&mut self, addr: u16, value: u8);
    /// Fait avancer le reste de la machine d'un M-cycle (4 T-cycles).
    fn tick(&mut self);
}

/// Registres du SM83.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registers {
    pub a: u8,
    pub f: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
}

impl Registers {
    pub fn af(&self) -> u16 {
        u16::from_be_bytes([self.a, self.f])
    }
    pub fn bc(&self) -> u16 {
        u16::from_be_bytes([self.b, self.c])
    }
    pub fn de(&self) -> u16 {
        u16::from_be_bytes([self.d, self.e])
    }
    pub fn hl(&self) -> u16 {
        u16::from_be_bytes([self.h, self.l])
    }
    pub fn set_af(&mut self, v: u16) {
        let [hi, lo] = v.to_be_bytes();
        self.a = hi;
        // Les 4 bits bas de F n'existent pas : ils valent toujours 0.
        self.f = lo & 0xF0;
    }
    pub fn set_bc(&mut self, v: u16) {
        [self.b, self.c] = v.to_be_bytes();
    }
    pub fn set_de(&mut self, v: u16) {
        [self.d, self.e] = v.to_be_bytes();
    }
    pub fn set_hl(&mut self, v: u16) {
        [self.h, self.l] = v.to_be_bytes();
    }
}

#[derive(Debug, Clone, Default)]
pub struct Cpu {
    pub regs: Registers,
    /// Interrupt Master Enable.
    pub ime: bool,
    /// `EI` n'active les interruptions qu'après l'instruction suivante.
    ime_delay: u8,
    pub halted: bool,
    /// Bug matériel : après un HALT raté, l'octet suivant est lu deux fois.
    halt_bug: bool,
    /// Le CPU est bloqué après un opcode illégal.
    pub locked: bool,
    /// T-cycles écoulés depuis le démarrage.
    pub cycles: u64,
}

impl Cpu {
    pub fn new() -> Self {
        Self::default()
    }

    /// État des registres juste après la boot ROM d'une Game Boy (DMG).
    /// On démarre directement ici : aucune boot ROM n'est nécessaire.
    pub fn post_boot_dmg() -> Self {
        let mut cpu = Self::new();
        cpu.regs = Registers {
            a: 0x01,
            f: 0xB0,
            b: 0x00,
            c: 0x13,
            d: 0x00,
            e: 0xD8,
            h: 0x01,
            l: 0x4D,
            sp: 0xFFFE,
            pc: 0x0100,
        };
        cpu
    }

    /// Exécute une instruction (ou traite une interruption) et renvoie
    /// le nombre de T-cycles consommés.
    pub fn step<B: Bus>(&mut self, bus: &mut B) -> u32 {
        let start = self.cycles;

        if self.locked {
            self.tick(bus);
        } else if !self.service_interrupt(bus) {
            if self.halted {
                self.tick(bus);
            } else {
                let op = self.fetch(bus);
                self.execute(bus, op);
            }
        }

        if self.ime_delay > 0 {
            self.ime_delay -= 1;
            if self.ime_delay == 0 {
                self.ime = true;
            }
        }

        (self.cycles - start) as u32
    }

    // ----- Accès mémoire et cycles -----

    fn tick<B: Bus>(&mut self, bus: &mut B) {
        bus.tick();
        self.cycles += 4;
    }

    fn read<B: Bus>(&mut self, bus: &mut B, addr: u16) -> u8 {
        self.tick(bus);
        bus.read(addr)
    }

    fn write<B: Bus>(&mut self, bus: &mut B, addr: u16, value: u8) {
        self.tick(bus);
        bus.write(addr, value);
    }

    fn fetch<B: Bus>(&mut self, bus: &mut B) -> u8 {
        let v = self.read(bus, self.regs.pc);
        if self.halt_bug {
            self.halt_bug = false;
        } else {
            self.regs.pc = self.regs.pc.wrapping_add(1);
        }
        v
    }

    fn fetch16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.fetch(bus);
        let hi = self.fetch(bus);
        u16::from_le_bytes([lo, hi])
    }

    fn push16<B: Bus>(&mut self, bus: &mut B, v: u16) {
        let [hi, lo] = v.to_be_bytes();
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write(bus, self.regs.sp, hi);
        self.regs.sp = self.regs.sp.wrapping_sub(1);
        self.write(bus, self.regs.sp, lo);
    }

    fn pop16<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let lo = self.read(bus, self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        let hi = self.read(bus, self.regs.sp);
        self.regs.sp = self.regs.sp.wrapping_add(1);
        u16::from_le_bytes([lo, hi])
    }

    // ----- Interruptions -----

    fn service_interrupt<B: Bus>(&mut self, bus: &mut B) -> bool {
        let pending = bus.read(IE_ADDR) & bus.read(IF_ADDR) & 0x1F;
        if pending == 0 {
            return false;
        }
        // Une interruption en attente réveille le CPU, même si IME = 0.
        self.halted = false;
        if !self.ime {
            return false;
        }

        self.ime = false;
        self.tick(bus);
        self.tick(bus);
        self.push16(bus, self.regs.pc);
        let bit = pending.trailing_zeros() as u16;
        let iflag = bus.read(IF_ADDR);
        bus.write(IF_ADDR, iflag & !(1 << bit));
        self.regs.pc = 0x0040 + 8 * bit;
        self.tick(bus);
        true
    }

    // ----- Drapeaux -----

    fn flag(&self, mask: u8) -> bool {
        self.regs.f & mask != 0
    }

    fn set_flags(&mut self, z: bool, n: bool, h: bool, c: bool) {
        self.regs.f = (z as u8) << 7 | (n as u8) << 6 | (h as u8) << 5 | (c as u8) << 4;
    }

    fn condition(&self, idx: u8) -> bool {
        match idx {
            0 => !self.flag(FLAG_Z),
            1 => self.flag(FLAG_Z),
            2 => !self.flag(FLAG_C),
            _ => self.flag(FLAG_C),
        }
    }

    // ----- Opérandes -----

    /// Registre 8 bits encodé sur 3 bits : B C D E H L (HL) A.
    fn get_r8<B: Bus>(&mut self, bus: &mut B, idx: u8) -> u8 {
        match idx {
            0 => self.regs.b,
            1 => self.regs.c,
            2 => self.regs.d,
            3 => self.regs.e,
            4 => self.regs.h,
            5 => self.regs.l,
            6 => self.read(bus, self.regs.hl()),
            _ => self.regs.a,
        }
    }

    fn set_r8<B: Bus>(&mut self, bus: &mut B, idx: u8, v: u8) {
        match idx {
            0 => self.regs.b = v,
            1 => self.regs.c = v,
            2 => self.regs.d = v,
            3 => self.regs.e = v,
            4 => self.regs.h = v,
            5 => self.regs.l = v,
            6 => self.write(bus, self.regs.hl(), v),
            _ => self.regs.a = v,
        }
    }

    /// Registre 16 bits encodé sur 2 bits : BC DE HL SP.
    fn get_r16(&self, idx: u8) -> u16 {
        match idx {
            0 => self.regs.bc(),
            1 => self.regs.de(),
            2 => self.regs.hl(),
            _ => self.regs.sp,
        }
    }

    fn set_r16(&mut self, idx: u8, v: u16) {
        match idx {
            0 => self.regs.set_bc(v),
            1 => self.regs.set_de(v),
            2 => self.regs.set_hl(v),
            _ => self.regs.sp = v,
        }
    }

    // ----- Opérations arithmétiques -----

    fn add(&mut self, v: u8, carry: u8) {
        let a = self.regs.a;
        let r = a as u16 + v as u16 + carry as u16;
        let h = (a & 0x0F) + (v & 0x0F) + carry > 0x0F;
        self.regs.a = r as u8;
        self.set_flags(r as u8 == 0, false, h, r > 0xFF);
    }

    fn sub(&mut self, v: u8, carry: u8) -> u8 {
        let a = self.regs.a;
        let r = a as i16 - v as i16 - carry as i16;
        let h = ((a & 0x0F) as i16 - (v & 0x0F) as i16 - carry as i16) < 0;
        self.set_flags(r as u8 == 0, true, h, r < 0);
        r as u8
    }

    /// Les 8 opérations ALU sur A : ADD ADC SUB SBC AND XOR OR CP.
    fn alu(&mut self, op: u8, v: u8) {
        let carry = self.flag(FLAG_C) as u8;
        match op {
            0 => self.add(v, 0),
            1 => self.add(v, carry),
            2 => self.regs.a = self.sub(v, 0),
            3 => self.regs.a = self.sub(v, carry),
            4 => {
                self.regs.a &= v;
                self.set_flags(self.regs.a == 0, false, true, false);
            }
            5 => {
                self.regs.a ^= v;
                self.set_flags(self.regs.a == 0, false, false, false);
            }
            6 => {
                self.regs.a |= v;
                self.set_flags(self.regs.a == 0, false, false, false);
            }
            _ => {
                self.sub(v, 0);
            }
        }
    }

    /// SP + e signé, utilisé par `ADD SP,e` et `LD HL,SP+e`.
    fn sp_plus_e<B: Bus>(&mut self, bus: &mut B) -> u16 {
        let e = self.fetch(bus) as i8 as i16 as u16;
        let sp = self.regs.sp;
        let h = (sp & 0x0F) + (e & 0x0F) > 0x0F;
        let c = (sp & 0xFF) + (e & 0xFF) > 0xFF;
        self.set_flags(false, false, h, c);
        sp.wrapping_add(e)
    }

    fn daa(&mut self) {
        let mut a = self.regs.a;
        let mut carry = self.flag(FLAG_C);
        if !self.flag(FLAG_N) {
            if carry || a > 0x99 {
                a = a.wrapping_add(0x60);
                carry = true;
            }
            if self.flag(FLAG_H) || (a & 0x0F) > 0x09 {
                a = a.wrapping_add(0x06);
            }
        } else {
            if carry {
                a = a.wrapping_sub(0x60);
            }
            if self.flag(FLAG_H) {
                a = a.wrapping_sub(0x06);
            }
        }
        self.regs.a = a;
        self.set_flags(a == 0, self.flag(FLAG_N), false, carry);
    }

    // ----- Décodage -----

    fn execute<B: Bus>(&mut self, bus: &mut B, op: u8) {
        match op {
            0x00 => {}

            // LD rr,nn
            0x01 | 0x11 | 0x21 | 0x31 => {
                let v = self.fetch16(bus);
                self.set_r16(op >> 4, v);
            }

            // LD (rr),A
            0x02 => self.write(bus, self.regs.bc(), self.regs.a),
            0x12 => self.write(bus, self.regs.de(), self.regs.a),
            0x22 | 0x32 => {
                let hl = self.regs.hl();
                self.write(bus, hl, self.regs.a);
                let next = if op == 0x22 {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                };
                self.regs.set_hl(next);
            }

            // LD A,(rr)
            0x0A => self.regs.a = self.read(bus, self.regs.bc()),
            0x1A => self.regs.a = self.read(bus, self.regs.de()),
            0x2A | 0x3A => {
                let hl = self.regs.hl();
                self.regs.a = self.read(bus, hl);
                let next = if op == 0x2A {
                    hl.wrapping_add(1)
                } else {
                    hl.wrapping_sub(1)
                };
                self.regs.set_hl(next);
            }

            // INC rr / DEC rr
            0x03 | 0x13 | 0x23 | 0x33 => {
                let r = op >> 4;
                self.set_r16(r, self.get_r16(r).wrapping_add(1));
                self.tick(bus);
            }
            0x0B | 0x1B | 0x2B | 0x3B => {
                let r = op >> 4;
                self.set_r16(r, self.get_r16(r).wrapping_sub(1));
                self.tick(bus);
            }

            // ADD HL,rr
            0x09 | 0x19 | 0x29 | 0x39 => {
                let hl = self.regs.hl();
                let v = self.get_r16(op >> 4);
                let r = hl as u32 + v as u32;
                let h = (hl & 0x0FFF) + (v & 0x0FFF) > 0x0FFF;
                self.set_flags(self.flag(FLAG_Z), false, h, r > 0xFFFF);
                self.regs.set_hl(r as u16);
                self.tick(bus);
            }

            // INC r / DEC r / LD r,n
            op if op & 0xC7 == 0x04 => {
                let r = (op >> 3) & 7;
                let v = self.get_r8(bus, r);
                let res = v.wrapping_add(1);
                self.set_r8(bus, r, res);
                self.set_flags(res == 0, false, v & 0x0F == 0x0F, self.flag(FLAG_C));
            }
            op if op & 0xC7 == 0x05 => {
                let r = (op >> 3) & 7;
                let v = self.get_r8(bus, r);
                let res = v.wrapping_sub(1);
                self.set_r8(bus, r, res);
                self.set_flags(res == 0, true, v & 0x0F == 0, self.flag(FLAG_C));
            }
            op if op & 0xC7 == 0x06 => {
                let v = self.fetch(bus);
                self.set_r8(bus, (op >> 3) & 7, v);
            }

            // Rotations de A
            0x07 => {
                let a = self.regs.a;
                self.regs.a = a.rotate_left(1);
                self.set_flags(false, false, false, a & 0x80 != 0);
            }
            0x0F => {
                let a = self.regs.a;
                self.regs.a = a.rotate_right(1);
                self.set_flags(false, false, false, a & 0x01 != 0);
            }
            0x17 => {
                let a = self.regs.a;
                self.regs.a = (a << 1) | self.flag(FLAG_C) as u8;
                self.set_flags(false, false, false, a & 0x80 != 0);
            }
            0x1F => {
                let a = self.regs.a;
                self.regs.a = (a >> 1) | (self.flag(FLAG_C) as u8) << 7;
                self.set_flags(false, false, false, a & 0x01 != 0);
            }

            // LD (nn),SP
            0x08 => {
                let addr = self.fetch16(bus);
                let [hi, lo] = self.regs.sp.to_be_bytes();
                self.write(bus, addr, lo);
                self.write(bus, addr.wrapping_add(1), hi);
            }

            // STOP : l'octet suivant est ignoré.
            0x10 => {
                self.fetch(bus);
            }

            // JR e / JR cc,e
            0x18 => {
                let e = self.fetch(bus) as i8;
                self.regs.pc = self.regs.pc.wrapping_add_signed(e as i16);
                self.tick(bus);
            }
            0x20 | 0x28 | 0x30 | 0x38 => {
                let e = self.fetch(bus) as i8;
                if self.condition((op >> 3) & 3) {
                    self.regs.pc = self.regs.pc.wrapping_add_signed(e as i16);
                    self.tick(bus);
                }
            }

            0x27 => self.daa(),
            0x2F => {
                self.regs.a = !self.regs.a;
                self.regs.f |= FLAG_N | FLAG_H;
            }
            0x37 => self.set_flags(self.flag(FLAG_Z), false, false, true),
            0x3F => self.set_flags(self.flag(FLAG_Z), false, false, !self.flag(FLAG_C)),

            // HALT
            0x76 => {
                let pending = bus.read(IE_ADDR) & bus.read(IF_ADDR) & 0x1F;
                if !self.ime && pending != 0 {
                    self.halt_bug = true;
                } else {
                    self.halted = true;
                }
            }

            // LD r,r'
            0x40..=0x7F => {
                let v = self.get_r8(bus, op & 7);
                self.set_r8(bus, (op >> 3) & 7, v);
            }

            // ALU A,r / ALU A,n
            0x80..=0xBF => {
                let v = self.get_r8(bus, op & 7);
                self.alu((op >> 3) & 7, v);
            }
            op if op & 0xC7 == 0xC6 => {
                let v = self.fetch(bus);
                self.alu((op >> 3) & 7, v);
            }

            // RET cc / RET / RETI
            0xC0 | 0xC8 | 0xD0 | 0xD8 => {
                self.tick(bus);
                if self.condition((op >> 3) & 3) {
                    self.regs.pc = self.pop16(bus);
                    self.tick(bus);
                }
            }
            0xC9 | 0xD9 => {
                self.regs.pc = self.pop16(bus);
                self.tick(bus);
                if op == 0xD9 {
                    self.ime = true;
                }
            }

            // POP rr / PUSH rr (AF à la place de SP)
            0xC1 | 0xD1 | 0xE1 | 0xF1 => {
                let v = self.pop16(bus);
                match (op >> 4) & 3 {
                    3 => self.regs.set_af(v),
                    r => self.set_r16(r, v),
                }
            }
            0xC5 | 0xD5 | 0xE5 | 0xF5 => {
                let v = match (op >> 4) & 3 {
                    3 => self.regs.af(),
                    r => self.get_r16(r),
                };
                self.tick(bus);
                self.push16(bus, v);
            }

            // JP / JP cc / JP HL
            0xC3 => {
                self.regs.pc = self.fetch16(bus);
                self.tick(bus);
            }
            0xC2 | 0xCA | 0xD2 | 0xDA => {
                let addr = self.fetch16(bus);
                if self.condition((op >> 3) & 3) {
                    self.regs.pc = addr;
                    self.tick(bus);
                }
            }
            0xE9 => self.regs.pc = self.regs.hl(),

            // CALL / CALL cc / RST
            0xCD => {
                let addr = self.fetch16(bus);
                self.tick(bus);
                self.push16(bus, self.regs.pc);
                self.regs.pc = addr;
            }
            0xC4 | 0xCC | 0xD4 | 0xDC => {
                let addr = self.fetch16(bus);
                if self.condition((op >> 3) & 3) {
                    self.tick(bus);
                    self.push16(bus, self.regs.pc);
                    self.regs.pc = addr;
                }
            }
            op if op & 0xC7 == 0xC7 => {
                self.tick(bus);
                self.push16(bus, self.regs.pc);
                self.regs.pc = (op & 0x38) as u16;
            }

            0xCB => {
                let cb = self.fetch(bus);
                self.execute_cb(bus, cb);
            }

            // Accès à la zone 0xFF00 (registres matériels)
            0xE0 => {
                let n = self.fetch(bus);
                self.write(bus, 0xFF00 | n as u16, self.regs.a);
            }
            0xF0 => {
                let n = self.fetch(bus);
                self.regs.a = self.read(bus, 0xFF00 | n as u16);
            }
            0xE2 => self.write(bus, 0xFF00 | self.regs.c as u16, self.regs.a),
            0xF2 => self.regs.a = self.read(bus, 0xFF00 | self.regs.c as u16),
            0xEA => {
                let addr = self.fetch16(bus);
                self.write(bus, addr, self.regs.a);
            }
            0xFA => {
                let addr = self.fetch16(bus);
                self.regs.a = self.read(bus, addr);
            }

            // Pile
            0xE8 => {
                self.regs.sp = self.sp_plus_e(bus);
                self.tick(bus);
                self.tick(bus);
            }
            0xF8 => {
                let v = self.sp_plus_e(bus);
                self.regs.set_hl(v);
                self.tick(bus);
            }
            0xF9 => {
                self.regs.sp = self.regs.hl();
                self.tick(bus);
            }

            0xF3 => {
                self.ime = false;
                self.ime_delay = 0;
            }
            0xFB => self.ime_delay = 2,

            // D3 DB DD E3 E4 EB EC ED F4 FC FD : opcodes illégaux, le CPU se bloque.
            _ => self.locked = true,
        }
    }

    fn execute_cb<B: Bus>(&mut self, bus: &mut B, op: u8) {
        let r = op & 7;
        let bit = (op >> 3) & 7;
        let v = self.get_r8(bus, r);
        match op >> 6 {
            // RLC RRC RL RR SLA SRA SWAP SRL
            0 => {
                let c_in = self.flag(FLAG_C) as u8;
                let (res, c_out) = match bit {
                    0 => (v.rotate_left(1), v >> 7),
                    1 => (v.rotate_right(1), v & 1),
                    2 => ((v << 1) | c_in, v >> 7),
                    3 => ((v >> 1) | (c_in << 7), v & 1),
                    4 => (v << 1, v >> 7),
                    5 => ((v >> 1) | (v & 0x80), v & 1),
                    6 => (v.rotate_left(4), 0),
                    _ => (v >> 1, v & 1),
                };
                self.set_r8(bus, r, res);
                self.set_flags(res == 0, false, false, c_out != 0);
            }
            // BIT
            1 => self.set_flags(v & (1 << bit) == 0, false, true, self.flag(FLAG_C)),
            // RES / SET
            2 => self.set_r8(bus, r, v & !(1 << bit)),
            _ => self.set_r8(bus, r, v | (1 << bit)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 64 Kio de RAM plate, sans aucun matériel : idéal pour tester le CPU seul.
    struct FlatBus {
        mem: Vec<u8>,
    }

    impl FlatBus {
        fn with_program(at: u16, program: &[u8]) -> Self {
            let mut mem = vec![0u8; 0x10000];
            mem[at as usize..at as usize + program.len()].copy_from_slice(program);
            FlatBus { mem }
        }
    }

    impl Bus for FlatBus {
        fn read(&mut self, addr: u16) -> u8 {
            self.mem[addr as usize]
        }
        fn write(&mut self, addr: u16, value: u8) {
            self.mem[addr as usize] = value;
        }
        fn tick(&mut self) {}
    }

    fn run(program: &[u8], steps: usize) -> (Cpu, FlatBus) {
        let mut bus = FlatBus::with_program(0, program);
        let mut cpu = Cpu::new();
        cpu.regs.sp = 0xFFFE;
        for _ in 0..steps {
            cpu.step(&mut bus);
        }
        (cpu, bus)
    }

    /// Durée en M-cycles de chaque opcode, avec F = 0xF0 :
    /// les conditions NZ/NC sont fausses, Z/C sont vraies. 0 = non testé ici.
    #[rustfmt::skip]
    const TIMINGS: [[u8; 16]; 8] = [
        [1,3,2,2,1,1,2,1,5,2,2,2,1,1,2,1], // 0x00
        [0,3,2,2,1,1,2,1,3,2,2,2,1,1,2,1], // 0x10
        [2,3,2,2,1,1,2,1,3,2,2,2,1,1,2,1], // 0x20
        [2,3,2,2,3,3,3,1,3,2,2,2,1,1,2,1], // 0x30
        [2,3,3,4,3,4,2,4,5,4,4,0,6,6,2,4], // 0xC0
        [2,3,3,0,3,4,2,4,5,4,4,0,6,0,2,4], // 0xD0
        [3,3,2,0,0,4,2,4,4,1,4,0,0,0,2,4], // 0xE0
        [3,3,2,1,0,4,2,4,3,2,4,1,0,0,2,4], // 0xF0
    ];

    fn expected_cycles(op: u8) -> u8 {
        match op {
            0x00..=0x3F => TIMINGS[(op >> 4) as usize][(op & 0xF) as usize],
            0x76 => 1,
            0x40..=0xBF if op & 7 == 6 || op & 0xF8 == 0x70 => 2,
            0x40..=0xBF => 1,
            _ => TIMINGS[(op >> 4) as usize - 8][(op & 0xF) as usize],
        }
    }

    fn cycles_of(program: &[u8]) -> u32 {
        let mut bus = FlatBus::with_program(0x0100, program);
        let mut cpu = Cpu::new();
        cpu.regs.pc = 0x0100;
        cpu.regs.sp = 0xD000;
        cpu.regs.f = 0xF0;
        cpu.regs.set_hl(0xC000);
        cpu.step(&mut bus) / 4
    }

    #[test]
    fn every_opcode_has_correct_timing() {
        for op in 0..=0xFFu8 {
            let expected = expected_cycles(op);
            if expected != 0 {
                assert_eq!(cycles_of(&[op]), expected as u32, "opcode {op:#04X}");
            }
        }
    }

    #[test]
    fn every_cb_opcode_has_correct_timing() {
        for op in 0..=0xFFu8 {
            let expected = match (op & 7, op >> 6) {
                (6, 1) => 3,
                (6, _) => 4,
                _ => 2,
            };
            assert_eq!(cycles_of(&[0xCB, op]), expected, "opcode CB {op:#04X}");
        }
    }

    #[test]
    fn illegal_opcode_locks_cpu() {
        let (cpu, _) = run(&[0xD3, 0x3C], 3);
        assert!(cpu.locked);
        assert_eq!(cpu.regs.a, 0);
    }

    #[test]
    fn daa_after_bcd_addition() {
        // LD A,0x15 ; ADD A,0x27 ; DAA  →  15 + 27 = 42 en BCD
        let (cpu, _) = run(&[0x3E, 0x15, 0xC6, 0x27, 0x27], 3);
        assert_eq!(cpu.regs.a, 0x42);
        assert!(!cpu.flag(FLAG_C));
    }

    #[test]
    fn sub_and_cp_flags() {
        // LD A,0x10 ; SUB 0x01
        let (cpu, _) = run(&[0x3E, 0x10, 0xD6, 0x01], 2);
        assert_eq!(cpu.regs.a, 0x0F);
        assert_eq!(cpu.regs.f, FLAG_N | FLAG_H);

        // LD A,0x10 ; CP 0x20  →  A inchangé, retenue
        let (cpu, _) = run(&[0x3E, 0x10, 0xFE, 0x20], 2);
        assert_eq!(cpu.regs.a, 0x10);
        assert_eq!(cpu.regs.f, FLAG_N | FLAG_C);
    }

    #[test]
    fn pop_af_clears_low_flag_bits() {
        // LD BC,0x12FF ; PUSH BC ; POP AF
        let (cpu, _) = run(&[0x01, 0xFF, 0x12, 0xC5, 0xF1], 3);
        assert_eq!(cpu.regs.a, 0x12);
        assert_eq!(cpu.regs.f, 0xF0);
        assert_eq!(cpu.regs.sp, 0xFFFE);
    }

    #[test]
    fn call_and_ret() {
        let mut program = vec![0u8; 0x11];
        program[..3].copy_from_slice(&[0xCD, 0x10, 0x00]); // CALL 0x0010
        program[0x10] = 0xC9; // RET
        let (cpu, bus) = run(&program, 2);
        assert_eq!(cpu.regs.pc, 0x0003);
        assert_eq!(cpu.regs.sp, 0xFFFE);
        assert_eq!(&bus.mem[0xFFFC..0xFFFE], &[0x03, 0x00]);
    }

    #[test]
    fn ld_hl_sp_plus_e_flags() {
        // LD SP,0x00FF ; LD HL,SP+1
        let (cpu, _) = run(&[0x31, 0xFF, 0x00, 0xF8, 0x01], 2);
        assert_eq!(cpu.regs.hl(), 0x0100);
        assert_eq!(cpu.regs.f, FLAG_H | FLAG_C);
    }

    #[test]
    fn cb_swap_and_bit() {
        // LD A,0xF1 ; SWAP A ; BIT 7,A
        let (cpu, _) = run(&[0x3E, 0xF1, 0xCB, 0x37, 0xCB, 0x7F], 3);
        assert_eq!(cpu.regs.a, 0x1F);
        assert_eq!(cpu.regs.f, FLAG_Z | FLAG_H);
    }

    #[test]
    fn interrupt_dispatch() {
        let mut bus = FlatBus::with_program(0x0200, &[0x00]);
        bus.mem[IE_ADDR as usize] = 0x04;
        bus.mem[IF_ADDR as usize] = 0x04;
        let mut cpu = Cpu::new();
        cpu.regs.pc = 0x0200;
        cpu.regs.sp = 0xD000;
        cpu.ime = true;

        assert_eq!(cpu.step(&mut bus), 20);
        assert_eq!(cpu.regs.pc, 0x0050);
        assert!(!cpu.ime);
        assert_eq!(bus.mem[IF_ADDR as usize], 0x00);
        assert_eq!(&bus.mem[0xCFFE..0xD000], &[0x00, 0x02]);
    }

    #[test]
    fn ei_is_delayed_by_one_instruction() {
        // EI ; NOP ; NOP  avec une interruption timer en attente
        let mut bus = FlatBus::with_program(0, &[0xFB, 0x00, 0x00]);
        bus.mem[IE_ADDR as usize] = 0x04;
        bus.mem[IF_ADDR as usize] = 0x04;
        let mut cpu = Cpu::new();
        cpu.regs.sp = 0xD000;

        cpu.step(&mut bus); // EI
        cpu.step(&mut bus); // NOP, exécuté avant que IME ne s'active
        assert_eq!(cpu.regs.pc, 0x0002);
        cpu.step(&mut bus); // l'interruption est prise
        assert_eq!(cpu.regs.pc, 0x0050);
    }

    #[test]
    fn halt_wakes_on_interrupt() {
        // HALT ; INC A  avec IME = 0
        let mut bus = FlatBus::with_program(0, &[0x76, 0x3C]);
        bus.mem[IE_ADDR as usize] = 0x01;
        let mut cpu = Cpu::new();

        cpu.step(&mut bus);
        assert!(cpu.halted);
        cpu.step(&mut bus);
        assert!(cpu.halted);

        bus.mem[IF_ADDR as usize] = 0x01;
        cpu.step(&mut bus);
        assert!(!cpu.halted);
        assert_eq!(cpu.regs.a, 1);
    }
}
