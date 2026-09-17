//! 8051 instruction-set simulator (for testing).

pub trait Io {
    /// Read external data memory.
    fn xread(&mut self, _cpu: &CpuState, _addr: u16) -> Option<u8> {
        None
    }
    /// Write external data memory. Return true if handled (not stored in XRAM).
    fn xwrite(&mut self, _cpu: &CpuState, _addr: u16, _v: u8) -> bool {
        false
    }
    /// Read a port SFR (P0-P3). Return None for the latch value.
    fn port_read(&mut self, _cpu: &CpuState, _port: u8) -> Option<u8> {
        None
    }
    fn port_write(&mut self, _cpu: &CpuState, _port: u8, _v: u8) {}
    fn serial_out(&mut self, _v: u8) {}
}

pub struct NullIo;
impl Io for NullIo {}

#[derive(Clone)]
pub struct CpuState {
    pub pc: u16,
    pub iram: [u8; 256],
    pub sfr: [u8; 128],
    pub cycles: u64,
}

pub struct Cpu {
    pub st: CpuState,
    pub rom: Vec<u8>,
    pub xram: Vec<u8>,
    pub halted: bool,
    pub halt_reason: String,
    pub serial: Vec<u8>,
    /// Interrupt in progress (priority levels: bit0 low, bit1 high).
    in_isr: u8,
    pub trace: bool,
    pub stop_on_self_loop: bool,
    pub max_sp: u8,
}

const SP: u8 = 0x81;
const DPL: u8 = 0x82;
const DPH: u8 = 0x83;
const PSW: u8 = 0xD0;
const ACC: u8 = 0xE0;
const B: u8 = 0xF0;

impl Cpu {
    pub fn new(rom: Vec<u8>) -> Cpu {
        let mut rom = rom;
        rom.resize(0x10000, 0xff);
        let mut st = CpuState { pc: 0, iram: [0; 256], sfr: [0; 128], cycles: 0 };
        st.sfr[(SP - 0x80) as usize] = 7;
        for p in [0x80u8, 0x90, 0xA0, 0xB0] {
            st.sfr[(p - 0x80) as usize] = 0xff;
        }
        Cpu { st, rom, xram: vec![0; 0x10000], halted: false, halt_reason: String::new(), serial: Vec::new(), in_isr: 0, trace: false, stop_on_self_loop: true, max_sp: 7 }
    }

    pub fn load_hex(text: &str) -> Result<Vec<u8>, String> {
        let mut rom = vec![0xffu8; 0x10000];
        for line in text.lines() {
            let l = line.trim();
            if l.is_empty() {
                continue;
            }
            let l = l.strip_prefix(':').ok_or("bad hex line")?;
            let bytes: Vec<u8> = (0..l.len() / 2).map(|i| u8::from_str_radix(&l[2 * i..2 * i + 2], 16).unwrap_or(0)).collect();
            if bytes.len() < 5 {
                continue;
            }
            let n = bytes[0] as usize;
            let addr = ((bytes[1] as usize) << 8) | bytes[2] as usize;
            match bytes[3] {
                0 => {
                    for k in 0..n {
                        rom[addr + k] = bytes[4 + k];
                    }
                }
                1 => break,
                _ => {}
            }
        }
        Ok(rom)
    }

    #[inline]
    fn a(&self) -> u8 {
        self.st.sfr[(ACC - 0x80) as usize]
    }
    #[inline]
    fn set_a(&mut self, v: u8) {
        self.st.sfr[(ACC - 0x80) as usize] = v;
    }
    #[inline]
    fn psw(&self) -> u8 {
        self.st.sfr[(PSW - 0x80) as usize]
    }
    #[inline]
    fn set_psw(&mut self, v: u8) {
        self.st.sfr[(PSW - 0x80) as usize] = v;
    }
    fn cy(&self) -> bool {
        self.psw() & 0x80 != 0
    }
    fn set_cy(&mut self, c: bool) {
        let p = self.psw();
        self.set_psw(if c { p | 0x80 } else { p & 0x7f });
    }
    fn bank(&self) -> u8 {
        (self.psw() >> 3) & 3
    }
    fn reg(&self, n: u8) -> u8 {
        self.st.iram[(self.bank() * 8 + n) as usize]
    }
    fn set_reg(&mut self, n: u8, v: u8) {
        let b = self.bank();
        self.st.iram[(b * 8 + n) as usize] = v;
    }
    fn dptr(&self) -> u16 {
        ((self.st.sfr[(DPH - 0x80) as usize] as u16) << 8) | self.st.sfr[(DPL - 0x80) as usize] as u16
    }
    fn set_dptr(&mut self, v: u16) {
        self.st.sfr[(DPH - 0x80) as usize] = (v >> 8) as u8;
        self.st.sfr[(DPL - 0x80) as usize] = v as u8;
    }

    pub fn read_dir(&mut self, io: &mut dyn Io, a: u8) -> u8 {
        if a < 0x80 {
            return self.st.iram[a as usize];
        }
        if matches!(a, 0x80 | 0x90 | 0xA0 | 0xB0) {
            let st = self.st.clone();
            if let Some(v) = io.port_read(&st, a) {
                return v;
            }
        }
        self.st.sfr[(a - 0x80) as usize]
    }

    pub fn write_dir(&mut self, io: &mut dyn Io, a: u8, v: u8) {
        if a < 0x80 {
            self.st.iram[a as usize] = v;
            return;
        }
        self.st.sfr[(a - 0x80) as usize] = v;
        match a {
            0x99 => {
                // SBUF
                self.serial.push(v);
                io.serial_out(v);
                self.st.sfr[(0x98 - 0x80) as usize] |= 0x02; // TI
            }
            0x80 | 0x90 | 0xA0 | 0xB0 => {
                let st = self.st.clone();
                io.port_write(&st, a, v);
            }
            _ => {}
        }
    }

    fn bit_addr(b: u8) -> (u8, u8) {
        if b < 0x80 {
            (0x20 + b / 8, b % 8)
        } else {
            (b & 0xf8, b & 7)
        }
    }

    pub fn read_bit(&mut self, io: &mut dyn Io, b: u8) -> bool {
        let (a, n) = Self::bit_addr(b);
        (self.read_dir(io, a) >> n) & 1 != 0
    }

    pub fn write_bit(&mut self, io: &mut dyn Io, b: u8, v: bool) {
        let (a, n) = Self::bit_addr(b);
        let cur = if a >= 0x80 { self.st.sfr[(a - 0x80) as usize] } else { self.st.iram[a as usize] };
        let nv = if v { cur | (1 << n) } else { cur & !(1 << n) };
        self.write_dir(io, a, nv);
    }

    fn fetch(&mut self) -> u8 {
        let v = self.rom[self.st.pc as usize];
        self.st.pc = self.st.pc.wrapping_add(1);
        v
    }

    fn push(&mut self, v: u8) {
        let sp = self.st.sfr[(SP - 0x80) as usize].wrapping_add(1);
        self.st.sfr[(SP - 0x80) as usize] = sp;
        self.st.iram[sp as usize] = v;
        if sp > self.max_sp {
            self.max_sp = sp;
        }
    }

    fn pop(&mut self) -> u8 {
        let sp = self.st.sfr[(SP - 0x80) as usize];
        let v = self.st.iram[sp as usize];
        self.st.sfr[(SP - 0x80) as usize] = sp.wrapping_sub(1);
        v
    }

    fn update_parity(&mut self) {
        let p = self.a().count_ones() & 1;
        let psw = self.psw();
        self.set_psw((psw & !1) | p as u8);
    }

    fn add(&mut self, v: u8, carry: bool) {
        let a = self.a();
        let c = carry as u16;
        let r = a as u16 + v as u16 + c;
        let ac = ((a & 0xf) + (v & 0xf) + c as u8) > 0xf;
        let ov = ((a ^ v) & 0x80 == 0) && ((a ^ r as u8) & 0x80 != 0);
        let mut psw = self.psw() & !(0x80 | 0x40 | 0x04);
        if r > 0xff {
            psw |= 0x80;
        }
        if ac {
            psw |= 0x40;
        }
        if ov {
            psw |= 0x04;
        }
        self.set_psw(psw);
        self.set_a(r as u8);
    }

    fn subb(&mut self, v: u8) {
        let a = self.a();
        let c = self.cy() as i16;
        let r = a as i16 - v as i16 - c;
        let ac = ((a & 0xf) as i16 - (v & 0xf) as i16 - c) < 0;
        let res = r as u8;
        let ov = ((a ^ v) & 0x80 != 0) && ((a ^ res) & 0x80 != 0);
        let mut psw = self.psw() & !(0x80 | 0x40 | 0x04);
        if r < 0 {
            psw |= 0x80;
        }
        if ac {
            psw |= 0x40;
        }
        if ov {
            psw |= 0x04;
        }
        self.set_psw(psw);
        self.set_a(res);
    }

    fn rel_jump(&mut self, rel: u8) {
        self.st.pc = (self.st.pc as i32 + rel as i8 as i32) as u16;
    }

    fn timers(&mut self, n: u64) {
        let tmod = self.st.sfr[(0x89 - 0x80) as usize];
        let tcon = self.st.sfr[(0x88 - 0x80) as usize];
        for t in 0..2u8 {
            let run = tcon & (if t == 0 { 0x10 } else { 0x40 }) != 0;
            if !run {
                continue;
            }
            let mode = (tmod >> (4 * t)) & 3;
            let (tl, th) = if t == 0 { (0x8A, 0x8C) } else { (0x8B, 0x8D) };
            let mut lo = self.st.sfr[(tl - 0x80) as usize] as u32;
            let mut hi = self.st.sfr[(th - 0x80) as usize] as u32;
            let mut overflow = false;
            for _ in 0..n {
                match mode {
                    1 => {
                        let v = ((hi << 8) | lo) + 1;
                        if v > 0xffff {
                            overflow = true;
                        }
                        lo = v & 0xff;
                        hi = (v >> 8) & 0xff;
                    }
                    2 => {
                        lo += 1;
                        if lo > 0xff {
                            lo = hi;
                            overflow = true;
                        }
                    }
                    _ => {
                        lo += 1;
                        if lo > 0xff {
                            lo = 0;
                            hi = (hi + 1) & 0xff;
                        }
                    }
                }
            }
            self.st.sfr[(tl - 0x80) as usize] = lo as u8;
            self.st.sfr[(th - 0x80) as usize] = hi as u8;
            if overflow {
                self.st.sfr[(0x88 - 0x80) as usize] |= if t == 0 { 0x20 } else { 0x80 };
            }
        }
    }

    fn interrupts(&mut self) {
        let ie = self.st.sfr[(0xA8 - 0x80) as usize];
        if ie & 0x80 == 0 || self.in_isr != 0 {
            return;
        }
        let tcon = self.st.sfr[(0x88 - 0x80) as usize];
        let scon = self.st.sfr[(0x98 - 0x80) as usize];
        let mut vec = None;
        if ie & 0x02 != 0 && tcon & 0x20 != 0 {
            self.st.sfr[(0x88 - 0x80) as usize] &= !0x20;
            vec = Some(0x0B);
        } else if ie & 0x08 != 0 && tcon & 0x80 != 0 {
            self.st.sfr[(0x88 - 0x80) as usize] &= !0x80;
            vec = Some(0x1B);
        } else if ie & 0x10 != 0 && scon & 0x03 != 0 {
            vec = Some(0x23);
        }
        if let Some(v) = vec {
            let pc = self.st.pc;
            self.push(pc as u8);
            self.push((pc >> 8) as u8);
            self.st.pc = v;
            self.in_isr = 1;
        }
    }

    pub fn run(&mut self, io: &mut dyn Io, max_cycles: u64) {
        while !self.halted && self.st.cycles < max_cycles {
            self.step(io);
        }
        if !self.halted {
            self.halt_reason = "cycle limit".into();
        }
    }

    pub fn step(&mut self, io: &mut dyn Io) {
        let pc0 = self.st.pc;
        let op = self.fetch();
        let mut cyc = 1u64;
        macro_rules! rn {
            () => {
                op & 7
            };
        }
        match op {
            0x00 => {}
            0x01 | 0x21 | 0x41 | 0x61 | 0x81 | 0xA1 | 0xC1 | 0xE1 => {
                let lo = self.fetch();
                let t = (self.st.pc & 0xf800) | (((op >> 5) as u16) << 8) | lo as u16;
                if t == pc0 && self.stop_on_self_loop && self.st.sfr[(0xA8 - 0x80) as usize] & 0x80 == 0 {
                    self.halted = true;
                    self.halt_reason = "self loop".into();
                }
                self.st.pc = t;
                cyc = 2;
            }
            0x11 | 0x31 | 0x51 | 0x71 | 0x91 | 0xB1 | 0xD1 | 0xF1 => {
                let lo = self.fetch();
                let t = (self.st.pc & 0xf800) | (((op >> 5) as u16) << 8) | lo as u16;
                let pc = self.st.pc;
                self.push(pc as u8);
                self.push((pc >> 8) as u8);
                self.st.pc = t;
                cyc = 2;
            }
            0x02 => {
                let hi = self.fetch();
                let lo = self.fetch();
                let t = ((hi as u16) << 8) | lo as u16;
                if t == pc0 && self.stop_on_self_loop && self.st.sfr[(0xA8 - 0x80) as usize] & 0x80 == 0 {
                    self.halted = true;
                    self.halt_reason = "self loop".into();
                }
                self.st.pc = t;
                cyc = 2;
            }
            0x12 => {
                let hi = self.fetch();
                let lo = self.fetch();
                let pc = self.st.pc;
                self.push(pc as u8);
                self.push((pc >> 8) as u8);
                self.st.pc = ((hi as u16) << 8) | lo as u16;
                cyc = 2;
            }
            0x03 => {
                let a = self.a();
                self.set_a(a.rotate_right(1));
            }
            0x13 => {
                let a = self.a();
                let c = self.cy();
                self.set_cy(a & 1 != 0);
                self.set_a((a >> 1) | ((c as u8) << 7));
            }
            0x23 => {
                let a = self.a();
                self.set_a(a.rotate_left(1));
            }
            0x33 => {
                let a = self.a();
                let c = self.cy();
                self.set_cy(a & 0x80 != 0);
                self.set_a((a << 1) | c as u8);
            }
            0x04 => {
                let a = self.a();
                self.set_a(a.wrapping_add(1));
            }
            0x05 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                self.write_dir(io, d, v.wrapping_add(1));
            }
            0x06 | 0x07 => {
                let r = self.reg(op & 1);
                self.st.iram[r as usize] = self.st.iram[r as usize].wrapping_add(1);
            }
            0x08..=0x0F => {
                let v = self.reg(rn!());
                self.set_reg(rn!(), v.wrapping_add(1));
            }
            0x14 => {
                let a = self.a();
                self.set_a(a.wrapping_sub(1));
            }
            0x15 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                self.write_dir(io, d, v.wrapping_sub(1));
            }
            0x16 | 0x17 => {
                let r = self.reg(op & 1);
                self.st.iram[r as usize] = self.st.iram[r as usize].wrapping_sub(1);
            }
            0x18..=0x1F => {
                let v = self.reg(rn!());
                self.set_reg(rn!(), v.wrapping_sub(1));
            }
            0x10 | 0x20 | 0x30 => {
                let b = self.fetch();
                let rel = self.fetch();
                let v = self.read_bit(io, b);
                let take = match op {
                    0x10 => v,
                    0x20 => v,
                    _ => !v,
                };
                if op == 0x10 && v {
                    self.write_bit(io, b, false);
                }
                if take {
                    self.rel_jump(rel);
                }
                cyc = 2;
            }
            0x22 => {
                let hi = self.pop();
                let lo = self.pop();
                self.st.pc = ((hi as u16) << 8) | lo as u16;
                cyc = 2;
            }
            0x32 => {
                let hi = self.pop();
                let lo = self.pop();
                self.st.pc = ((hi as u16) << 8) | lo as u16;
                self.in_isr = 0;
                cyc = 2;
            }
            0x24 | 0x34 | 0x94 => {
                let v = self.fetch();
                self.arith(op, v);
            }
            0x25 | 0x35 | 0x95 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                self.arith(op, v);
            }
            0x26 | 0x27 | 0x36 | 0x37 | 0x96 | 0x97 => {
                let r = self.reg(op & 1);
                let v = self.st.iram[r as usize];
                self.arith(op, v);
            }
            0x28..=0x2F | 0x38..=0x3F | 0x98..=0x9F => {
                let v = self.reg(rn!());
                self.arith(op, v);
            }
            0x40 | 0x50 | 0x60 | 0x70 | 0x80 => {
                let rel = self.fetch();
                let take = match op {
                    0x40 => self.cy(),
                    0x50 => !self.cy(),
                    0x60 => self.a() == 0,
                    0x70 => self.a() != 0,
                    _ => true,
                };
                if op == 0x80 && rel == 0xfe && self.stop_on_self_loop && self.st.sfr[(0xA8 - 0x80) as usize] & 0x80 == 0 {
                    self.halted = true;
                    self.halt_reason = "self loop".into();
                }
                if take {
                    self.rel_jump(rel);
                }
                cyc = 2;
            }
            0x42 | 0x52 | 0x62 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                let a = self.a();
                let r = match op {
                    0x42 => v | a,
                    0x52 => v & a,
                    _ => v ^ a,
                };
                self.write_dir(io, d, r);
            }
            0x43 | 0x53 | 0x63 => {
                let d = self.fetch();
                let k = self.fetch();
                let v = self.read_dir(io, d);
                let r = match op {
                    0x43 => v | k,
                    0x53 => v & k,
                    _ => v ^ k,
                };
                self.write_dir(io, d, r);
                cyc = 2;
            }
            0x44 | 0x54 | 0x64 => {
                let k = self.fetch();
                self.logic(op, k);
            }
            0x45 | 0x55 | 0x65 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                self.logic(op, v);
            }
            0x46 | 0x47 | 0x56 | 0x57 | 0x66 | 0x67 => {
                let r = self.reg(op & 1);
                let v = self.st.iram[r as usize];
                self.logic(op, v);
            }
            0x48..=0x4F | 0x58..=0x5F | 0x68..=0x6F => {
                let v = self.reg(rn!());
                self.logic(op, v);
            }
            0x72 | 0x82 | 0xA0 | 0xB0 => {
                let b = self.fetch();
                let v = self.read_bit(io, b);
                let c = self.cy();
                let r = match op {
                    0x72 => c | v,
                    0x82 => c & v,
                    0xA0 => c | !v,
                    _ => c & !v,
                };
                self.set_cy(r);
                cyc = 2;
            }
            0x73 => {
                self.st.pc = self.dptr().wrapping_add(self.a() as u16);
                cyc = 2;
            }
            0x74 => {
                let k = self.fetch();
                self.set_a(k);
            }
            0x75 => {
                let d = self.fetch();
                let k = self.fetch();
                self.write_dir(io, d, k);
                cyc = 2;
            }
            0x76 | 0x77 => {
                let k = self.fetch();
                let r = self.reg(op & 1);
                self.st.iram[r as usize] = k;
            }
            0x78..=0x7F => {
                let k = self.fetch();
                self.set_reg(rn!(), k);
            }
            0x83 => {
                let v = self.rom[self.st.pc.wrapping_add(self.a() as u16) as usize];
                self.set_a(v);
                cyc = 2;
            }
            0x84 => {
                let a = self.a();
                let b = self.st.sfr[(B - 0x80) as usize];
                let mut psw = self.psw() & !(0x80 | 0x04);
                if b == 0 {
                    psw |= 0x04;
                } else {
                    self.set_a(a / b);
                    self.st.sfr[(B - 0x80) as usize] = a % b;
                }
                self.set_psw(psw);
                cyc = 4;
            }
            0x85 => {
                let s = self.fetch();
                let d = self.fetch();
                let v = self.read_dir(io, s);
                self.write_dir(io, d, v);
                cyc = 2;
            }
            0x86 | 0x87 => {
                let d = self.fetch();
                let r = self.reg(op & 1);
                let v = self.st.iram[r as usize];
                self.write_dir(io, d, v);
                cyc = 2;
            }
            0x88..=0x8F => {
                let d = self.fetch();
                let v = self.reg(rn!());
                self.write_dir(io, d, v);
                cyc = 2;
            }
            0x90 => {
                let hi = self.fetch();
                let lo = self.fetch();
                self.set_dptr(((hi as u16) << 8) | lo as u16);
                cyc = 2;
            }
            0x92 => {
                let b = self.fetch();
                let c = self.cy();
                self.write_bit(io, b, c);
                cyc = 2;
            }
            0x93 => {
                let v = self.rom[self.dptr().wrapping_add(self.a() as u16) as usize];
                self.set_a(v);
                cyc = 2;
            }
            0xA2 => {
                let b = self.fetch();
                let v = self.read_bit(io, b);
                self.set_cy(v);
            }
            0xA3 => {
                let d = self.dptr().wrapping_add(1);
                self.set_dptr(d);
                cyc = 2;
            }
            0xA4 => {
                let r = self.a() as u16 * self.st.sfr[(B - 0x80) as usize] as u16;
                self.set_a(r as u8);
                self.st.sfr[(B - 0x80) as usize] = (r >> 8) as u8;
                let mut psw = self.psw() & !(0x80 | 0x04);
                if r > 0xff {
                    psw |= 0x04;
                }
                self.set_psw(psw);
                cyc = 4;
            }
            0xA5 => {
                self.halted = true;
                self.halt_reason = "A5 opcode".into();
            }
            0xA6 | 0xA7 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                let r = self.reg(op & 1);
                self.st.iram[r as usize] = v;
                cyc = 2;
            }
            0xA8..=0xAF => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                self.set_reg(rn!(), v);
                cyc = 2;
            }
            0xB2 => {
                let b = self.fetch();
                let v = self.read_bit(io, b);
                self.write_bit(io, b, !v);
            }
            0xB3 => {
                let c = self.cy();
                self.set_cy(!c);
            }
            0xB4..=0xBF => {
                let (lhs, rhs) = match op {
                    0xB4 => {
                        let k = self.fetch();
                        (self.a(), k)
                    }
                    0xB5 => {
                        let d = self.fetch();
                        let v = self.read_dir(io, d);
                        (self.a(), v)
                    }
                    0xB6 | 0xB7 => {
                        let k = self.fetch();
                        let r = self.reg(op & 1);
                        (self.st.iram[r as usize], k)
                    }
                    _ => {
                        let k = self.fetch();
                        (self.reg(rn!()), k)
                    }
                };
                let rel = self.fetch();
                self.set_cy(lhs < rhs);
                if lhs != rhs {
                    self.rel_jump(rel);
                }
                cyc = 2;
            }
            0xC0 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                self.push(v);
                cyc = 2;
            }
            0xC2 => {
                let b = self.fetch();
                self.write_bit(io, b, false);
            }
            0xC3 => self.set_cy(false),
            0xC4 => {
                let a = self.a();
                self.set_a(a.rotate_left(4));
            }
            0xC5 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                let a = self.a();
                self.write_dir(io, d, a);
                self.set_a(v);
            }
            0xC6 | 0xC7 => {
                let r = self.reg(op & 1);
                let v = self.st.iram[r as usize];
                let a = self.a();
                self.st.iram[r as usize] = a;
                self.set_a(v);
            }
            0xC8..=0xCF => {
                let v = self.reg(rn!());
                let a = self.a();
                self.set_reg(rn!(), a);
                self.set_a(v);
            }
            0xD0 => {
                let d = self.fetch();
                let v = self.pop();
                self.write_dir(io, d, v);
                cyc = 2;
            }
            0xD2 => {
                let b = self.fetch();
                self.write_bit(io, b, true);
            }
            0xD3 => self.set_cy(true),
            0xD4 => {
                let mut a = self.a() as u16;
                let psw = self.psw();
                if (a & 0xf) > 9 || psw & 0x40 != 0 {
                    a += 6;
                }
                let mut c = psw & 0x80 != 0 || a > 0xff;
                if ((a >> 4) & 0xf) > 9 || c {
                    a += 0x60;
                }
                if a > 0xff {
                    c = true;
                }
                self.set_a(a as u8);
                self.set_cy(c);
            }
            0xD5 => {
                let d = self.fetch();
                let rel = self.fetch();
                let v = self.read_dir(io, d).wrapping_sub(1);
                self.write_dir(io, d, v);
                if v != 0 {
                    self.rel_jump(rel);
                }
                cyc = 2;
            }
            0xD6 | 0xD7 => {
                let r = self.reg(op & 1);
                let v = self.st.iram[r as usize];
                let a = self.a();
                self.st.iram[r as usize] = (v & 0xf0) | (a & 0x0f);
                self.set_a((a & 0xf0) | (v & 0x0f));
            }
            0xD8..=0xDF => {
                let rel = self.fetch();
                let v = self.reg(rn!()).wrapping_sub(1);
                self.set_reg(rn!(), v);
                if v != 0 {
                    self.rel_jump(rel);
                }
                cyc = 2;
            }
            0xE0 => {
                let st = self.st.clone();
                let d = self.dptr();
                let v = io.xread(&st, d).unwrap_or(self.xram[d as usize]);
                self.set_a(v);
                cyc = 2;
            }
            0xE2 | 0xE3 => {
                let r = self.reg(op & 1) as u16 | ((self.st.sfr[(0xA0 - 0x80) as usize] as u16) << 8);
                let st = self.st.clone();
                let v = io.xread(&st, r).unwrap_or(self.xram[r as usize]);
                self.set_a(v);
                cyc = 2;
            }
            0xE4 => self.set_a(0),
            0xE5 => {
                let d = self.fetch();
                let v = self.read_dir(io, d);
                self.set_a(v);
            }
            0xE6 | 0xE7 => {
                let r = self.reg(op & 1);
                let v = self.st.iram[r as usize];
                self.set_a(v);
            }
            0xE8..=0xEF => {
                let v = self.reg(rn!());
                self.set_a(v);
            }
            0xF0 => {
                let st = self.st.clone();
                let d = self.dptr();
                let a = self.a();
                if !io.xwrite(&st, d, a) {
                    self.xram[d as usize] = a;
                }
                cyc = 2;
            }
            0xF2 | 0xF3 => {
                let r = self.reg(op & 1) as u16 | ((self.st.sfr[(0xA0 - 0x80) as usize] as u16) << 8);
                let st = self.st.clone();
                let a = self.a();
                if !io.xwrite(&st, r, a) {
                    self.xram[r as usize] = a;
                }
                cyc = 2;
            }
            0xF4 => {
                let a = self.a();
                self.set_a(!a);
            }
            0xF5 => {
                let d = self.fetch();
                let a = self.a();
                self.write_dir(io, d, a);
            }
            0xF6 | 0xF7 => {
                let r = self.reg(op & 1);
                let a = self.a();
                self.st.iram[r as usize] = a;
            }
            0xF8..=0xFF => {
                let a = self.a();
                self.set_reg(rn!(), a);
            }
            _ => {
                self.halted = true;
                self.halt_reason = format!("invalid opcode {:02x} at {:04x}", op, pc0);
            }
        }
        self.update_parity();
        self.st.cycles += cyc;
        self.timers(cyc);
        self.interrupts();
        if self.trace {
            eprintln!("{:04x}: {:02x}  a={:02x} psw={:02x} sp={:02x} b={:02x} dptr={:04x} r={:02x?}", pc0, op, self.a(), self.psw(), self.st.sfr[(SP - 0x80) as usize], self.st.sfr[(B - 0x80) as usize], self.dptr(), &self.st.iram[0..8]);
        }
    }

    fn arith(&mut self, op: u8, v: u8) {
        match op >> 4 {
            0x2 => self.add(v, false),
            0x3 => {
                let c = self.cy();
                self.add(v, c)
            }
            _ => self.subb(v),
        }
    }

    fn logic(&mut self, op: u8, v: u8) {
        let a = self.a();
        let r = match op >> 4 {
            0x4 => a | v,
            0x5 => a & v,
            _ => a ^ v,
        };
        self.set_a(r);
    }
}
