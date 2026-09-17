//! 8051 assembly representation and instruction encoder.

pub mod parse;

use std::fmt;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Val,
    Lo,
    Hi,
    B2,
}

/// Restricted relocatable expression: part(sym + off).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Expr {
    pub sym: Option<Rc<str>>,
    pub off: i64,
    pub part: Part,
}

impl Expr {
    pub fn num(v: i64) -> Expr {
        Expr { sym: None, off: v, part: Part::Val }
    }
    pub fn sym(s: &Rc<str>) -> Expr {
        Expr { sym: Some(s.clone()), off: 0, part: Part::Val }
    }
    pub fn sym_off(s: &Rc<str>, off: i64) -> Expr {
        Expr { sym: Some(s.clone()), off, part: Part::Val }
    }
    pub fn lo(mut self) -> Expr {
        if self.sym.is_none() {
            return Expr::num(self.eval_part(self.off));
        }
        self.part = Part::Lo;
        self
    }
    pub fn hi(mut self) -> Expr {
        if self.sym.is_none() {
            return Expr::num((self.off >> 8) & 0xff);
        }
        self.part = Part::Hi;
        self
    }
    pub fn b2(mut self) -> Expr {
        if self.sym.is_none() {
            return Expr::num((self.off >> 16) & 0xff);
        }
        self.part = Part::B2;
        self
    }
    pub fn add(mut self, k: i64) -> Expr {
        assert!(self.part == Part::Val || self.sym.is_none());
        self.off += k;
        self
    }
    pub fn const_val(&self) -> Option<i64> {
        if self.sym.is_none() { Some(self.eval_part(self.off)) } else { None }
    }
    fn eval_part(&self, v: i64) -> i64 {
        match self.part {
            Part::Val => v,
            Part::Lo => v & 0xff,
            Part::Hi => (v >> 8) & 0xff,
            Part::B2 => (v >> 16) & 0xff,
        }
    }
    pub fn resolve(&self, lookup: &dyn Fn(&str) -> Option<i64>) -> Option<i64> {
        let base = match &self.sym {
            Some(s) => lookup(s)?,
            None => 0,
        };
        Some(self.eval_part(base + self.off))
    }
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let inner = match (&self.sym, self.off) {
            (None, v) => {
                if self.part == Part::Val {
                    return if v < 0 || v > 9 { write!(f, "0x{:02x}", v) } else { write!(f, "{}", v) };
                }
                format!("0x{:x}", v)
            }
            (Some(s), 0) => s.to_string(),
            (Some(s), o) if o > 0 => format!("{} + {}", s, o),
            (Some(s), o) => format!("{} - {}", s, -o),
        };
        match self.part {
            Part::Val => write!(f, "{}", inner),
            Part::Lo => write!(f, "<({})", inner),
            Part::Hi => write!(f, ">({})", inner),
            Part::B2 => write!(f, "b2({})", inner),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Op {
    A,
    AB,
    C,
    Dptr,
    R(u8),
    AtR(u8),
    AtDptr,
    AtADptr,
    AtAPc,
    Imm(Expr),
    Dir(Expr),
    Bit(Expr),
    Code(Expr),
}

impl Op {
    pub fn dir(v: i64) -> Op {
        Op::Dir(Expr::num(v))
    }
    pub fn imm(v: i64) -> Op {
        Op::Imm(Expr::num(v & 0xff))
    }
    pub fn bit(v: i64) -> Op {
        Op::Bit(Expr::num(v))
    }
    pub fn label(s: &Rc<str>) -> Op {
        Op::Code(Expr::sym(s))
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Op::A => write!(f, "a"),
            Op::AB => write!(f, "ab"),
            Op::C => write!(f, "c"),
            Op::Dptr => write!(f, "dptr"),
            Op::R(n) => write!(f, "r{}", n),
            Op::AtR(n) => write!(f, "@r{}", n),
            Op::AtDptr => write!(f, "@dptr"),
            Op::AtADptr => write!(f, "@a+dptr"),
            Op::AtAPc => write!(f, "@a+pc"),
            Op::Imm(e) => write!(f, "#{}", e),
            Op::Dir(e) => match e.const_val() {
                Some(v) => write!(f, "{}", sfr_name(v).map(|s| s.to_string()).unwrap_or_else(|| format!("0x{:02x}", v))),
                None => write!(f, "{}", e),
            },
            Op::Bit(e) => match e.const_val() {
                Some(v) => write!(f, "{}", bit_name(v)),
                None => write!(f, "{}", e),
            },
            Op::Code(e) => write!(f, "{}", e),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mn {
    Acall,
    Add,
    Addc,
    Ajmp,
    Anl,
    Cjne,
    Clr,
    Cpl,
    Da,
    Dec,
    Div,
    Djnz,
    Inc,
    Jb,
    Jbc,
    Jc,
    /// `jmp @a+dptr`, or a relaxable jump when the operand is a code address.
    Jmp,
    Jnb,
    Jnc,
    Jnz,
    Jz,
    Lcall,
    Ljmp,
    Mov,
    Movc,
    Movx,
    Mul,
    Nop,
    Orl,
    Pop,
    Push,
    Ret,
    Reti,
    Rl,
    Rlc,
    Rr,
    Rrc,
    Setb,
    Sjmp,
    Subb,
    Swap,
    Xch,
    Xchd,
    Xrl,
    /// Relaxable call.
    Call,
}

impl Mn {
    pub fn name(self) -> &'static str {
        match self {
            Mn::Acall => "acall",
            Mn::Add => "add",
            Mn::Addc => "addc",
            Mn::Ajmp => "ajmp",
            Mn::Anl => "anl",
            Mn::Cjne => "cjne",
            Mn::Clr => "clr",
            Mn::Cpl => "cpl",
            Mn::Da => "da",
            Mn::Dec => "dec",
            Mn::Div => "div",
            Mn::Djnz => "djnz",
            Mn::Inc => "inc",
            Mn::Jb => "jb",
            Mn::Jbc => "jbc",
            Mn::Jc => "jc",
            Mn::Jmp => "jmp",
            Mn::Jnb => "jnb",
            Mn::Jnc => "jnc",
            Mn::Jnz => "jnz",
            Mn::Jz => "jz",
            Mn::Lcall => "lcall",
            Mn::Ljmp => "ljmp",
            Mn::Mov => "mov",
            Mn::Movc => "movc",
            Mn::Movx => "movx",
            Mn::Mul => "mul",
            Mn::Nop => "nop",
            Mn::Orl => "orl",
            Mn::Pop => "pop",
            Mn::Push => "push",
            Mn::Ret => "ret",
            Mn::Reti => "reti",
            Mn::Rl => "rl",
            Mn::Rlc => "rlc",
            Mn::Rr => "rr",
            Mn::Rrc => "rrc",
            Mn::Setb => "setb",
            Mn::Sjmp => "sjmp",
            Mn::Subb => "subb",
            Mn::Swap => "swap",
            Mn::Xch => "xch",
            Mn::Xchd => "xchd",
            Mn::Xrl => "xrl",
            Mn::Call => "call",
        }
    }
    pub fn from_name(s: &str) -> Option<Mn> {
        use Mn::*;
        Some(match s {
            "acall" => Acall,
            "add" => Add,
            "addc" | "adc" => Addc,
            "ajmp" => Ajmp,
            "anl" | "and" => Anl,
            "cjne" => Cjne,
            "clr" => Clr,
            "cpl" => Cpl,
            "da" => Da,
            "dec" => Dec,
            "div" => Div,
            "djnz" => Djnz,
            "inc" => Inc,
            "jb" => Jb,
            "jbc" => Jbc,
            "jc" => Jc,
            "jmp" => Jmp,
            "jnb" => Jnb,
            "jnc" => Jnc,
            "jnz" => Jnz,
            "jz" => Jz,
            "lcall" => Lcall,
            "ljmp" => Ljmp,
            "mov" => Mov,
            "movc" => Movc,
            "movx" => Movx,
            "mul" => Mul,
            "nop" => Nop,
            "orl" | "or" => Orl,
            "pop" => Pop,
            "push" => Push,
            "ret" => Ret,
            "reti" => Reti,
            "rl" => Rl,
            "rlc" => Rlc,
            "rr" => Rr,
            "rrc" => Rrc,
            "setb" => Setb,
            "sjmp" => Sjmp,
            "subb" | "sbc" => Subb,
            "swap" => Swap,
            "xch" => Xch,
            "xchd" => Xchd,
            "xrl" | "xor" => Xrl,
            "call" => Call,
            _ => return None,
        })
    }
    pub fn is_cond_jump(self) -> bool {
        matches!(self, Mn::Jb | Mn::Jbc | Mn::Jnb | Mn::Jc | Mn::Jnc | Mn::Jz | Mn::Jnz | Mn::Cjne | Mn::Djnz)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Insn {
    pub mn: Mn,
    pub ops: Vec<Op>,
}

impl Insn {
    pub fn new(mn: Mn, ops: Vec<Op>) -> Insn {
        Insn { mn, ops }
    }
    /// The code-address target operand, if any.
    pub fn target(&self) -> Option<&Expr> {
        match self.ops.last() {
            Some(Op::Code(e)) => Some(e),
            _ => None,
        }
    }
    pub fn target_mut(&mut self) -> Option<&mut Expr> {
        match self.ops.last_mut() {
            Some(Op::Code(e)) => Some(e),
            _ => None,
        }
    }
}

impl fmt::Display for Insn {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.mn.name())?;
        for (i, o) in self.ops.iter().enumerate() {
            write!(f, "{}{}", if i == 0 { "\t" } else { ", " }, o)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Label(Rc<str>),
    Insn(Insn),
    Db(Vec<Expr>),
    Dw(Vec<Expr>),
    /// Reserve bytes (zero-filled in code).
    Ds(u32),
    Comment(String),
}

pub fn sfr_name(addr: i64) -> Option<&'static str> {
    SFRS.iter().find(|(_, a)| *a == addr).map(|(n, _)| *n)
}

pub fn bit_name(addr: i64) -> String {
    if let Some((n, _)) = SBITS.iter().find(|(_, a)| *a == addr) {
        return n.to_string();
    }
    if addr >= 0x80 {
        let byte = addr & !7;
        if let Some(n) = sfr_name(byte) {
            return format!("{}.{}", n, addr & 7);
        }
    }
    format!("0x{:02x}", addr)
}

pub const SFRS: &[(&str, i64)] = &[
    ("P0", 0x80),
    ("SP", 0x81),
    ("DPL", 0x82),
    ("DPH", 0x83),
    ("PCON", 0x87),
    ("TCON", 0x88),
    ("TMOD", 0x89),
    ("TL0", 0x8A),
    ("TL1", 0x8B),
    ("TH0", 0x8C),
    ("TH1", 0x8D),
    ("P1", 0x90),
    ("SCON", 0x98),
    ("SBUF", 0x99),
    ("P2", 0xA0),
    ("IE", 0xA8),
    ("P3", 0xB0),
    ("IP", 0xB8),
    ("T2CON", 0xC8),
    ("RCAP2L", 0xCA),
    ("RCAP2H", 0xCB),
    ("TL2", 0xCC),
    ("TH2", 0xCD),
    ("PSW", 0xD0),
    ("ACC", 0xE0),
    ("B", 0xF0),
];

pub const SBITS: &[(&str, i64)] = &[
    ("IT0", 0x88),
    ("IE0", 0x89),
    ("IT1", 0x8A),
    ("IE1", 0x8B),
    ("TR0", 0x8C),
    ("TF0", 0x8D),
    ("TR1", 0x8E),
    ("TF1", 0x8F),
    ("RI", 0x98),
    ("TI", 0x99),
    ("RB8", 0x9A),
    ("TB8", 0x9B),
    ("REN", 0x9C),
    ("SM2", 0x9D),
    ("SM1", 0x9E),
    ("SM0", 0x9F),
    ("EX0", 0xA8),
    ("ET0", 0xA9),
    ("EX1", 0xAA),
    ("ET1", 0xAB),
    ("ES", 0xAC),
    ("ET2", 0xAD),
    ("EA", 0xAF),
    ("RXD", 0xB0),
    ("TXD", 0xB1),
    ("INT0", 0xB2),
    ("INT1", 0xB3),
    ("T0", 0xB4),
    ("T1", 0xB5),
    ("WR", 0xB6),
    ("RD", 0xB7),
    ("PX0", 0xB8),
    ("PT0", 0xB9),
    ("PX1", 0xBA),
    ("PT1", 0xBB),
    ("PS", 0xBC),
    ("PT2", 0xBD),
    ("CP_RL2", 0xC8),
    ("C_T2", 0xC9),
    ("TR2", 0xCA),
    ("EXEN2", 0xCB),
    ("TCLK", 0xCC),
    ("RCLK", 0xCD),
    ("EXF2", 0xCE),
    ("TF2", 0xCF),
    ("P", 0xD0),
    ("F1", 0xD1),
    ("OV", 0xD2),
    ("RS0", 0xD3),
    ("RS1", 0xD4),
    ("F0", 0xD5),
    ("AC", 0xD6),
    ("CY", 0xD7),
];

pub const A_DIR: i64 = 0xE0;
pub const B_DIR: i64 = 0xF0;
pub const PSW_DIR: i64 = 0xD0;
pub const DPL_DIR: i64 = 0x82;
pub const DPH_DIR: i64 = 0x83;
pub const SP_DIR: i64 = 0x81;
pub const OV_BIT: i64 = 0xD2;
pub const EA_BIT: i64 = 0xAF;

#[derive(Debug)]
pub struct EncodeError(pub String);

/// Relaxation state for relaxable jumps/calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// sjmp / ajmp-able / short conditional
    Short,
    /// ajmp / acall (same 2K page)
    Abs11,
    /// ljmp / lcall, or conditional inverted over an ljmp
    Long,
}

fn reg_n(o: &Op) -> Option<u8> {
    match o {
        Op::R(n) => Some(*n),
        _ => None,
    }
}
fn at_r(o: &Op) -> Option<u8> {
    match o {
        Op::AtR(n) => Some(*n),
        _ => None,
    }
}

/// Instruction size in bytes for a given relaxation state.
pub fn insn_size(i: &Insn, reach: Reach) -> u32 {
    match i.mn {
        Mn::Jmp if matches!(i.ops.first(), Some(Op::Code(_))) => match reach {
            Reach::Short | Reach::Abs11 => 2,
            Reach::Long => 3,
        },
        Mn::Call => match reach {
            Reach::Short | Reach::Abs11 => 2,
            Reach::Long => 3,
        },
        Mn::Jc | Mn::Jnc | Mn::Jz | Mn::Jnz => match reach {
            Reach::Long => 5,
            _ => 2,
        },
        Mn::Jb | Mn::Jnb | Mn::Jbc => match reach {
            Reach::Long => 6,
            _ => 3,
        },
        Mn::Cjne => match reach {
            Reach::Long => 6,
            _ => 3,
        },
        Mn::Djnz => {
            let base = if matches!(i.ops[0], Op::R(_)) { 2 } else { 3 };
            match reach {
                Reach::Long => base + 2 + 3,
                _ => base,
            }
        }
        _ => encode_fixed(i, 0, &|_| Some(0)).map(|v| v.len() as u32).unwrap_or(3),
    }
}

fn val8(e: &Expr, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<u8, EncodeError> {
    let v = e.resolve(lookup).ok_or_else(|| EncodeError(format!("undefined symbol in '{}'", e)))?;
    if v < -128 || v > 255 {
        return Err(EncodeError(format!("value {} out of 8-bit range in '{}'", v, e)));
    }
    Ok(v as u8)
}

fn val16(e: &Expr, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<u16, EncodeError> {
    let v = e.resolve(lookup).ok_or_else(|| EncodeError(format!("undefined symbol in '{}'", e)))?;
    Ok(v as u16)
}

fn dir8(e: &Expr, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<u8, EncodeError> {
    let v = e.resolve(lookup).ok_or_else(|| EncodeError(format!("undefined symbol in '{}'", e)))?;
    if !(0..=255).contains(&v) {
        return Err(EncodeError(format!("direct address {:#x} out of range in '{}'", v, e)));
    }
    Ok(v as u8)
}

fn bit8(e: &Expr, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<u8, EncodeError> {
    dir8(e, lookup)
}

fn rel8(target: &Expr, pc_after: u32, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<u8, EncodeError> {
    let t = target.resolve(lookup).ok_or_else(|| EncodeError(format!("undefined label '{}'", target)))?;
    let d = t - pc_after as i64;
    if !(-128..=127).contains(&d) {
        return Err(EncodeError(format!("relative jump out of range ({}) to '{}'", d, target)));
    }
    Ok(d as i8 as u8)
}

pub fn rel_fits(target: i64, pc_after: u32) -> bool {
    let d = target - pc_after as i64;
    (-128..=127).contains(&d)
}

/// Encode a non-relaxable instruction (or relaxable one in its given form).
pub fn encode(i: &Insn, pc: u32, reach: Reach, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<Vec<u8>, EncodeError> {
    match i.mn {
        Mn::Jmp if matches!(i.ops.first(), Some(Op::Code(_))) => {
            let t = i.target().unwrap();
            match reach {
                Reach::Short => Ok(vec![0x80, rel8(t, pc + 2, lookup)?]),
                Reach::Abs11 => encode_fixed(&Insn::new(Mn::Ajmp, i.ops.clone()), pc, lookup),
                Reach::Long => encode_fixed(&Insn::new(Mn::Ljmp, i.ops.clone()), pc, lookup),
            }
        }
        Mn::Call => match reach {
            Reach::Short | Reach::Abs11 => encode_fixed(&Insn::new(Mn::Acall, i.ops.clone()), pc, lookup),
            Reach::Long => encode_fixed(&Insn::new(Mn::Lcall, i.ops.clone()), pc, lookup),
        },
        Mn::Jc | Mn::Jnc | Mn::Jz | Mn::Jnz | Mn::Jb | Mn::Jnb | Mn::Jbc | Mn::Cjne | Mn::Djnz if reach == Reach::Long => {
            // Inverted/short-circuit form: <cond> +3 over an ljmp, e.g. jnc $+5; ljmp target
            let t = i.target().unwrap().clone();
            let mut short = i.clone();
            let skip_len = match i.mn {
                Mn::Cjne | Mn::Djnz | Mn::Jbc => 2 + 3, // cond jumps to taken-stub; sjmp over; ljmp
                _ => 3,
            };
            match i.mn {
                Mn::Jc | Mn::Jnc | Mn::Jz | Mn::Jnz | Mn::Jb | Mn::Jnb => {
                    short.mn = match i.mn {
                        Mn::Jc => Mn::Jnc,
                        Mn::Jnc => Mn::Jc,
                        Mn::Jz => Mn::Jnz,
                        Mn::Jnz => Mn::Jz,
                        Mn::Jb => Mn::Jnb,
                        _ => Mn::Jb,
                    };
                    let size = insn_size(&short, Reach::Short);
                    let after = pc + size + 3;
                    *short.target_mut().unwrap() = Expr::num(after as i64);
                    let mut v = encode(&short, pc, Reach::Short, lookup)?;
                    v.extend(encode_fixed(&Insn::new(Mn::Ljmp, vec![Op::Code(t)]), pc + size, lookup)?);
                    Ok(v)
                }
                _ => {
                    // cjne/djnz/jbc: cond jumps to ljmp; otherwise sjmp over it.
                    let size = insn_size(&short, Reach::Short);
                    let ljmp_at = pc + size + 2;
                    *short.target_mut().unwrap() = Expr::num(ljmp_at as i64);
                    let mut v = encode(&short, pc, Reach::Short, lookup)?;
                    v.extend([0x80, 3]);
                    v.extend(encode_fixed(&Insn::new(Mn::Ljmp, vec![Op::Code(t)]), ljmp_at, lookup)?);
                    let _ = skip_len;
                    Ok(v)
                }
            }
        }
        _ => encode_fixed(i, pc, lookup),
    }
}

pub fn encode_fixed(i: &Insn, pc: u32, lookup: &dyn Fn(&str) -> Option<i64>) -> Result<Vec<u8>, EncodeError> {
    use Op::*;
    let o = &i.ops;
    let bad = || EncodeError(format!("invalid operands for '{}'", i));
    let n = o.len();
    let o0 = o.first();
    let o1 = o.get(1);
    // Helper for the arithmetic/logic families: base opcode for A,#imm; then dir, @Ri, Rn.
    let alu = |base: u8| -> Result<Vec<u8>, EncodeError> {
        match o1 {
            Some(Imm(e)) => Ok(vec![base, val8(e, lookup)?]),
            Some(Dir(e)) => Ok(vec![base + 1, dir8(e, lookup)?]),
            Some(AtR(r)) => Ok(vec![base + 2 + r]),
            Some(R(r)) => Ok(vec![base + 4 + r]),
            _ => Err(bad()),
        }
    };
    let bytes = match i.mn {
        Mn::Nop => vec![0x00],
        Mn::Ret => vec![0x22],
        Mn::Reti => vec![0x32],
        Mn::Ajmp | Mn::Acall => {
            let Some(Code(t)) = o0 else { return Err(bad()) };
            let tv = val16(t, lookup)? as u32;
            let next = pc + 2;
            if (tv & 0xf800) != (next & 0xf800) {
                return Err(EncodeError(format!("{} target {:#x} not in same 2K page as {:#x}", i.mn.name(), tv, next)));
            }
            let op = (((tv >> 8) & 7) << 5) as u8 | if i.mn == Mn::Ajmp { 0x01 } else { 0x11 };
            vec![op, tv as u8]
        }
        Mn::Ljmp | Mn::Lcall => {
            let Some(Code(t)) = o0 else { return Err(bad()) };
            let tv = val16(t, lookup)?;
            vec![if i.mn == Mn::Ljmp { 0x02 } else { 0x12 }, (tv >> 8) as u8, tv as u8]
        }
        Mn::Sjmp => {
            let Some(Code(t)) = o0 else { return Err(bad()) };
            vec![0x80, rel8(t, pc + 2, lookup)?]
        }
        Mn::Jmp => match o0 {
            Some(AtADptr) => vec![0x73],
            _ => return Err(bad()),
        },
        Mn::Rr => vec![0x03],
        Mn::Rrc => vec![0x13],
        Mn::Rl => vec![0x23],
        Mn::Rlc => vec![0x33],
        Mn::Inc | Mn::Dec => {
            let b = if i.mn == Mn::Inc { 0x00 } else { 0x10 };
            match o0 {
                Some(A) => vec![b + 0x04],
                Some(Dir(e)) => vec![b + 0x05, dir8(e, lookup)?],
                Some(AtR(r)) => vec![b + 0x06 + r],
                Some(R(r)) => vec![b + 0x08 + r],
                Some(Dptr) if i.mn == Mn::Inc => vec![0xA3],
                _ => return Err(bad()),
            }
        }
        Mn::Jbc | Mn::Jb | Mn::Jnb => {
            let (Some(Bit(b)), Some(Code(t))) = (o0, o1) else { return Err(bad()) };
            let op = match i.mn {
                Mn::Jbc => 0x10,
                Mn::Jb => 0x20,
                _ => 0x30,
            };
            vec![op, bit8(b, lookup)?, rel8(t, pc + 3, lookup)?]
        }
        Mn::Jc | Mn::Jnc | Mn::Jz | Mn::Jnz => {
            let Some(Code(t)) = o0 else { return Err(bad()) };
            let op = match i.mn {
                Mn::Jc => 0x40,
                Mn::Jnc => 0x50,
                Mn::Jz => 0x60,
                _ => 0x70,
            };
            vec![op, rel8(t, pc + 2, lookup)?]
        }
        Mn::Add | Mn::Addc | Mn::Subb => {
            if o0 != Some(&A) {
                return Err(bad());
            }
            alu(match i.mn {
                Mn::Add => 0x24,
                Mn::Addc => 0x34,
                _ => 0x94,
            })?
        }
        Mn::Orl | Mn::Anl | Mn::Xrl => {
            let (b, cb, cnb) = match i.mn {
                Mn::Orl => (0x40, 0x72, 0xA0),
                Mn::Anl => (0x50, 0x82, 0xB0),
                _ => (0x60, 0, 0),
            };
            match (o0, o1) {
                (Some(Dir(d)), Some(A)) => vec![b + 2, dir8(d, lookup)?],
                (Some(Dir(d)), Some(Imm(e))) => vec![b + 3, dir8(d, lookup)?, val8(e, lookup)?],
                (Some(A), _) => alu(b + 4)?,
                (Some(C), Some(Bit(e))) if cb != 0 => vec![cb, bit8(e, lookup)?],
                _ => {
                    let _ = cnb;
                    return Err(bad());
                }
            }
        }
        Mn::Mov => match (o0, o1) {
            (Some(A), Some(Imm(e))) => vec![0x74, val8(e, lookup)?],
            (Some(A), Some(Dir(e))) => vec![0xE5, dir8(e, lookup)?],
            (Some(A), Some(AtR(r))) => vec![0xE6 + r],
            (Some(A), Some(R(r))) => vec![0xE8 + r],
            (Some(Dir(d)), Some(Imm(e))) => vec![0x75, dir8(d, lookup)?, val8(e, lookup)?],
            (Some(Dir(d)), Some(Dir(s))) => vec![0x85, dir8(s, lookup)?, dir8(d, lookup)?],
            (Some(Dir(d)), Some(AtR(r))) => vec![0x86 + r, dir8(d, lookup)?],
            (Some(Dir(d)), Some(R(r))) => vec![0x88 + r, dir8(d, lookup)?],
            (Some(Dir(d)), Some(A)) => vec![0xF5, dir8(d, lookup)?],
            (Some(AtR(r)), Some(Imm(e))) => vec![0x76 + r, val8(e, lookup)?],
            (Some(AtR(r)), Some(Dir(e))) => vec![0xA6 + r, dir8(e, lookup)?],
            (Some(AtR(r)), Some(A)) => vec![0xF6 + r],
            (Some(R(r)), Some(Imm(e))) => vec![0x78 + r, val8(e, lookup)?],
            (Some(R(r)), Some(Dir(e))) => vec![0xA8 + r, dir8(e, lookup)?],
            (Some(R(r)), Some(A)) => vec![0xF8 + r],
            (Some(Dptr), Some(Imm(e))) => {
                let v = val16(e, lookup)?;
                vec![0x90, (v >> 8) as u8, v as u8]
            }
            (Some(Bit(b)), Some(C)) => vec![0x92, bit8(b, lookup)?],
            (Some(C), Some(Bit(b))) => vec![0xA2, bit8(b, lookup)?],
            _ => return Err(bad()),
        },
        Mn::Movc => match (o0, o1) {
            (Some(A), Some(AtAPc)) => vec![0x83],
            (Some(A), Some(AtADptr)) => vec![0x93],
            _ => return Err(bad()),
        },
        Mn::Movx => match (o0, o1) {
            (Some(A), Some(AtDptr)) => vec![0xE0],
            (Some(A), Some(AtR(r))) => vec![0xE2 + r],
            (Some(AtDptr), Some(A)) => vec![0xF0],
            (Some(AtR(r)), Some(A)) => vec![0xF2 + r],
            _ => return Err(bad()),
        },
        Mn::Div => vec![0x84],
        Mn::Mul => vec![0xA4],
        Mn::Cpl => match o0 {
            Some(A) => vec![0xF4],
            Some(C) => vec![0xB3],
            Some(Bit(b)) => vec![0xB2, bit8(b, lookup)?],
            _ => return Err(bad()),
        },
        Mn::Clr => match o0 {
            Some(A) => vec![0xE4],
            Some(C) => vec![0xC3],
            Some(Bit(b)) => vec![0xC2, bit8(b, lookup)?],
            _ => return Err(bad()),
        },
        Mn::Setb => match o0 {
            Some(C) => vec![0xD3],
            Some(Bit(b)) => vec![0xD2, bit8(b, lookup)?],
            _ => return Err(bad()),
        },
        Mn::Cjne => {
            let Some(Code(t)) = o.get(2) else { return Err(bad()) };
            let rel = rel8(t, pc + 3, lookup)?;
            match (o0, o1) {
                (Some(A), Some(Imm(e))) => vec![0xB4, val8(e, lookup)?, rel],
                (Some(A), Some(Dir(e))) => vec![0xB5, dir8(e, lookup)?, rel],
                (Some(AtR(r)), Some(Imm(e))) => vec![0xB6 + r, val8(e, lookup)?, rel],
                (Some(R(r)), Some(Imm(e))) => vec![0xB8 + r, val8(e, lookup)?, rel],
                _ => return Err(bad()),
            }
        }
        Mn::Push | Mn::Pop => {
            let Some(Dir(d)) = o0 else { return Err(bad()) };
            vec![if i.mn == Mn::Push { 0xC0 } else { 0xD0 }, dir8(d, lookup)?]
        }
        Mn::Swap => vec![0xC4],
        Mn::Da => vec![0xD4],
        Mn::Xch => match o1 {
            Some(Dir(e)) => vec![0xC5, dir8(e, lookup)?],
            Some(AtR(r)) => vec![0xC6 + r],
            Some(R(r)) => vec![0xC8 + r],
            _ => return Err(bad()),
        },
        Mn::Xchd => match o1 {
            Some(AtR(r)) => vec![0xD6 + r],
            _ => return Err(bad()),
        },
        Mn::Djnz => {
            let Some(Code(t)) = o1 else { return Err(bad()) };
            match o0 {
                Some(R(r)) => vec![0xD8 + r, rel8(t, pc + 2, lookup)?],
                Some(Dir(d)) => vec![0xD5, dir8(d, lookup)?, rel8(t, pc + 3, lookup)?],
                _ => return Err(bad()),
            }
        }
        Mn::Call => return Err(bad()),
    };
    let _ = (n, reg_n, at_r);
    Ok(bytes)
}
