//! Conversion of preprocessing tokens into C tokens.

use crate::diag::{Loc, Result, err, error};
use crate::pp::{PKind, PTok};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(Rc<str>),
    Kw(Kw),
    /// Integer constant: value and C type.
    Int(u64, IntLitTy),
    Float(f64, bool),
    Str(Vec<u8>),
    Punct(&'static str),
    Asm(String),
    Pragma(String),
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntLitTy {
    Int,
    UInt,
    Long,
    ULong,
    LongLong,
    ULongLong,
    /// Character constant (type int, value already converted).
    Char,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub loc: Loc,
}

macro_rules! keywords {
    ($($name:ident = $s:expr),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Kw { $($name),* }
        impl Kw {
            pub fn from_str(s: &str) -> Option<Kw> {
                match s { $($s => Some(Kw::$name),)* _ => None }
            }
            pub fn as_str(self) -> &'static str {
                match self { $(Kw::$name => $s),* }
            }
        }
    };
}

keywords! {
    Auto = "auto", Break = "break", Case = "case", Char = "char", Const = "const", Continue = "continue",
    Default = "default", Do = "do", Double = "double", Else = "else", Enum = "enum", Extern = "extern",
    Float = "float", For = "for", Goto = "goto", If = "if", Inline = "inline", Int = "int", Long = "long",
    Register = "register", Restrict = "restrict", Return = "return", Short = "short", Signed = "signed",
    Sizeof = "sizeof", Static = "static", Struct = "struct", Switch = "switch", Typedef = "typedef",
    Union = "union", Unsigned = "unsigned", Void = "void", Volatile = "volatile", While = "while",
    Bool = "_Bool", Alignas = "_Alignas", Alignof = "_Alignof", Noreturn = "_Noreturn",
    StaticAssert = "_Static_assert", Generic = "_Generic", ThreadLocal = "_Thread_local",
    Complex = "_Complex", Atomic = "_Atomic",
    // GNU
    Typeof = "typeof", Attribute = "__attribute__", Extension = "__extension__", BuiltinOffsetof = "__builtin_offsetof",
    BuiltinVaArg = "__builtin_va_arg",
    // SDCC
    Data = "__data", Idata = "__idata", Xdata = "__xdata", Pdata = "__pdata", Code = "__code",
    Bit = "__bit", Sbit = "__sbit", Sfr = "__sfr", Sfr16 = "__sfr16", Sfr32 = "__sfr32", At = "__at",
    Interrupt = "__interrupt", Using = "__using", Naked = "__naked", Critical = "__critical",
    Reentrant = "__reentrant", AsmBegin = "__asm", AsmEnd = "__endasm", AsmFn = "__asm__",
    Banked = "__banked", Nonbanked = "__nonbanked", Wparam = "__wparam", Shadowregs = "__shadowregs",
    PreservesRegs = "__preserves_regs", Smallc = "__smallc", Fastcall = "__z88dk_fastcall",
    Callee = "__z88dk_callee", Trap = "__trap", Sdcccall = "__sdcccall", Raisonance = "__raisonance",
    Iar = "__iar", Cosmic = "__cosmic", Hightide = "__z88dk_hightide",
}

fn canonical_keyword(s: &str) -> Option<Kw> {
    let s2 = match s {
        "__inline" | "__inline__" => "inline",
        "__restrict" | "__restrict__" => "restrict",
        "__const" | "__const__" => "const",
        "__volatile" | "__volatile__" => "volatile",
        "__signed" | "__signed__" => "signed",
        "__typeof" | "__typeof__" => "typeof",
        "__alignof" | "__alignof__" => "_Alignof",
        "_asm" => "__asm",
        "_endasm" => "__endasm",
        "__attribute" => "__attribute__",
        "bool" => "_Bool",
        "static_assert" => "_Static_assert",
        "alignof" => "_Alignof",
        "alignas" => "_Alignas",
        "thread_local" => "_Thread_local",
        "__typeof_unqual__" | "typeof_unqual" => "typeof",
        _ => s,
    };
    Kw::from_str(s2)
}

const PUNCTS: &[&str] = &[
    "<<=", ">>=", "...", "->", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "*=", "/=", "%=", "+=",
    "-=", "&=", "^=", "|=", "##", "[", "]", "(", ")", "{", "}", ".", "&", "*", "+", "-", "~", "!", "/", "%", "<",
    ">", "^", "|", "?", ":", ";", "=", ",", "#",
];

/// Parse an integer literal. Returns (value, has_unsigned_suffix). None if malformed.
pub fn parse_int_literal(s: &str) -> Option<(u64, bool)> {
    let (v, ty) = parse_int_literal_full(s)?;
    Some((v, matches!(ty, IntLitTy::UInt | IntLitTy::ULong | IntLitTy::ULongLong)))
}

pub fn parse_int_literal_full(s: &str) -> Option<(u64, IntLitTy)> {
    let s: String = s.chars().filter(|&c| c != '\'').collect();
    let lower = s.to_ascii_lowercase();
    let (radix, digits_start) = if lower.starts_with("0x") {
        (16, 2)
    } else if lower.starts_with("0b") {
        (2, 2)
    } else if lower.starts_with('0') && lower.len() > 1 {
        (8, 1)
    } else {
        (10, 0)
    };
    let body = &lower[digits_start..];
    let end = body.find(|c: char| !c.is_digit(radix)).unwrap_or(body.len());
    let (digits, suffix) = body.split_at(end);
    if digits.is_empty() && radix != 8 {
        return None;
    }
    let val = if digits.is_empty() { 0 } else { u128::from_str_radix(digits, radix).ok()? };
    if val > u64::MAX as u128 {
        return None;
    }
    let val = val as u64;
    let (mut u, mut l) = (false, 0);
    match suffix {
        "" => {}
        "u" => u = true,
        "l" => l = 1,
        "ul" | "lu" => {
            u = true;
            l = 1
        }
        "ll" => l = 2,
        "ull" | "llu" => {
            u = true;
            l = 2
        }
        _ => return None,
    }
    let decimal = radix == 10;
    // C rules with int=16, long=32, long long=64 bits.
    let fits = |bits: u32, signed: bool| -> bool {
        let max = if signed { (1u128 << (bits - 1)) - 1 } else { (1u128 << bits) - 1 };
        (val as u128) <= max
    };
    let cands: &[(IntLitTy, u32, bool)] = match (u, l, decimal) {
        (false, 0, true) => &[(IntLitTy::Int, 16, true), (IntLitTy::Long, 32, true), (IntLitTy::LongLong, 64, true)],
        (false, 0, false) => &[
            (IntLitTy::Int, 16, true),
            (IntLitTy::UInt, 16, false),
            (IntLitTy::Long, 32, true),
            (IntLitTy::ULong, 32, false),
            (IntLitTy::LongLong, 64, true),
            (IntLitTy::ULongLong, 64, false),
        ],
        (true, 0, _) => &[(IntLitTy::UInt, 16, false), (IntLitTy::ULong, 32, false), (IntLitTy::ULongLong, 64, false)],
        (false, 1, true) => &[(IntLitTy::Long, 32, true), (IntLitTy::LongLong, 64, true)],
        (false, 1, false) => &[
            (IntLitTy::Long, 32, true),
            (IntLitTy::ULong, 32, false),
            (IntLitTy::LongLong, 64, true),
            (IntLitTy::ULongLong, 64, false),
        ],
        (true, 1, _) => &[(IntLitTy::ULong, 32, false), (IntLitTy::ULongLong, 64, false)],
        (false, _, true) => &[(IntLitTy::LongLong, 64, true)],
        (false, _, false) => &[(IntLitTy::LongLong, 64, true), (IntLitTy::ULongLong, 64, false)],
        (true, _, _) => &[(IntLitTy::ULongLong, 64, false)],
    };
    for &(ty, bits, signed) in cands {
        if fits(bits, signed) {
            return Some((val, ty));
        }
    }
    Some((val, IntLitTy::ULongLong))
}

fn parse_escape(chars: &[char], i: &mut usize, loc: Loc) -> Result<u32> {
    // chars[*i] is the char after the backslash.
    let c = chars[*i];
    *i += 1;
    Ok(match c {
        'n' => 10,
        't' => 9,
        'r' => 13,
        'a' => 7,
        'b' => 8,
        'f' => 12,
        'v' => 11,
        'e' => 27,
        '\\' => 92,
        '\'' => 39,
        '"' => 34,
        '?' => 63,
        'x' => {
            let mut v: u32 = 0;
            let start = *i;
            while *i < chars.len() && chars[*i].is_ascii_hexdigit() {
                v = v.wrapping_mul(16).wrapping_add(chars[*i].to_digit(16).unwrap());
                *i += 1;
            }
            if *i == start {
                return err(loc, "\\x used with no following hex digits");
            }
            v
        }
        '0'..='7' => {
            let mut v = c.to_digit(8).unwrap();
            let mut n = 1;
            while n < 3 && *i < chars.len() && chars[*i].is_digit(8) {
                v = v * 8 + chars[*i].to_digit(8).unwrap();
                *i += 1;
                n += 1;
            }
            v
        }
        'u' | 'U' => {
            let n = if c == 'u' { 4 } else { 8 };
            let mut v: u32 = 0;
            for _ in 0..n {
                if *i >= chars.len() || !chars[*i].is_ascii_hexdigit() {
                    return err(loc, "incomplete universal character name");
                }
                v = v * 16 + chars[*i].to_digit(16).unwrap();
                *i += 1;
            }
            v
        }
        other => other as u32,
    })
}

/// Parse a character literal; returns (value as int, wide).
pub fn parse_char_literal(s: &str, loc: Loc) -> Result<(i64, bool)> {
    let wide = !s.starts_with('\'');
    let start = s.find('\'').unwrap() + 1;
    let chars: Vec<char> = s[start..s.len() - 1].chars().collect();
    let mut i = 0;
    let mut vals = Vec::new();
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 1;
            vals.push(parse_escape(&chars, &mut i, loc)?);
        } else {
            let c = chars[i];
            i += 1;
            if wide {
                vals.push(c as u32);
            } else if ('\u{E080}'..='\u{E0FF}').contains(&c) {
                // A byte from a non-UTF-8 source.
                vals.push(c as u32 & 0xff);
            } else {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    vals.push(b as u32);
                }
            }
        }
    }
    if vals.is_empty() {
        return err(loc, "empty character constant");
    }
    if wide {
        return Ok((vals[0] as i64, true));
    }
    if vals.len() == 1 {
        // char is unsigned in this implementation.
        return Ok(((vals[0] & 0xff) as i64, false));
    }
    let mut v: i64 = 0;
    for x in vals {
        v = (v << 8) | (x & 0xff) as i64;
    }
    Ok(((v as i16) as i64, false))
}

fn parse_string_literal(s: &str, loc: Loc, out: &mut Vec<u8>) -> Result<()> {
    let start = s.find('"').unwrap() + 1;
    let chars: Vec<char> = s[start..s.len() - 1].chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 1;
            let v = parse_escape(&chars, &mut i, loc)?;
            if chars[i - 1] == 'u' || (i >= 5 && chars.get(i.wrapping_sub(5)) == Some(&'u')) {
                // best effort: encode as UTF-8 if > 0xff
            }
            if v > 0xff {
                if let Some(c) = char::from_u32(v) {
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                } else {
                    out.push(v as u8);
                }
            } else {
                out.push(v as u8);
            }
        } else if ('\u{E080}'..='\u{E0FF}').contains(&chars[i]) {
            // A byte from a non-UTF-8 source.
            out.push(chars[i] as u32 as u8);
            i += 1;
        } else {
            let mut buf = [0u8; 4];
            out.extend_from_slice(chars[i].encode_utf8(&mut buf).as_bytes());
            i += 1;
        }
    }
    Ok(())
}

pub fn convert(ptoks: Vec<PTok>) -> Result<Vec<Token>> {
    let mut out: Vec<Token> = Vec::with_capacity(ptoks.len() + 1);
    let mut last_loc = Loc::default();
    let mut i = 0;
    while i < ptoks.len() {
        let t = &ptoks[i];
        last_loc = t.loc;
        let tok = match t.kind {
            PKind::Ident => {
                if &*t.text == "true" || &*t.text == "false" {
                    Tok::Int((&*t.text == "true") as u64, IntLitTy::Int)
                } else if &*t.text == "nullptr" {
                    Tok::Int(0, IntLitTy::Int)
                } else if let Some(k) = canonical_keyword(&t.text) {
                    Tok::Kw(k)
                } else {
                    Tok::Ident(t.text.clone())
                }
            }
            PKind::Number => {
                if let Some((v, ty)) = parse_int_literal_full(&t.text) {
                    Tok::Int(v, ty)
                } else {
                    let s = t.text.to_ascii_lowercase();
                    let (body, is_f) = if s.ends_with('f') && !s.starts_with("0x") {
                        (&s[..s.len() - 1], true)
                    } else if s.ends_with('l') {
                        (&s[..s.len() - 1], false)
                    } else if s.starts_with("0x") && (s.ends_with('f')) && s.contains('p') {
                        (&s[..s.len() - 1], true)
                    } else {
                        (&s[..], false)
                    };
                    let v = if body.starts_with("0x") { parse_hex_float(&body[2..]) } else { body.parse::<f64>().ok() };
                    match v {
                        Some(v) => Tok::Float(v, is_f),
                        None => return err(t.loc, format!("invalid numeric constant '{}'", t.text)),
                    }
                }
            }
            PKind::Char => {
                let (v, _) = parse_char_literal(&t.text, t.loc)?;
                Tok::Int(v as u64, IntLitTy::Char)
            }
            PKind::Str => {
                let mut bytes = Vec::new();
                parse_string_literal(&t.text, t.loc, &mut bytes)?;
                // Adjacent string concatenation.
                while i + 1 < ptoks.len() && ptoks[i + 1].kind == PKind::Str {
                    i += 1;
                    parse_string_literal(&ptoks[i].text, ptoks[i].loc, &mut bytes)?;
                }
                Tok::Str(bytes)
            }
            PKind::Punct => {
                let p = PUNCTS.iter().find(|p| **p == &*t.text).copied().ok_or_else(|| error(t.loc, "bad punctuator"))?;
                Tok::Punct(p)
            }
            PKind::AsmBlock => Tok::Asm(t.text.to_string()),
            PKind::Pragma => Tok::Pragma(t.text.to_string()),
            PKind::Other => {
                if &*t.text == "\\" || &*t.text == "@" || &*t.text == "`" {
                    return err(t.loc, format!("stray '{}' in program", t.text));
                }
                return err(t.loc, format!("stray '{}' in program", t.text));
            }
            PKind::Eof => {
                i += 1;
                continue;
            }
        };
        out.push(Token { tok, loc: t.loc });
        i += 1;
    }
    out.push(Token { tok: Tok::Eof, loc: last_loc });
    Ok(out)
}

fn parse_hex_float(s: &str) -> Option<f64> {
    let (mant, exp) = match s.find('p') {
        Some(p) => (&s[..p], s[p + 1..].parse::<i32>().ok()?),
        None => (s, 0),
    };
    let (ip, fp) = match mant.find('.') {
        Some(d) => (&mant[..d], &mant[d + 1..]),
        None => (mant, ""),
    };
    let mut v: f64 = 0.0;
    for c in ip.chars() {
        v = v * 16.0 + c.to_digit(16)? as f64;
    }
    let mut scale = 1.0 / 16.0;
    for c in fp.chars() {
        v += c.to_digit(16)? as f64 * scale;
        scale /= 16.0;
    }
    Some(v * 2f64.powi(exp))
}
