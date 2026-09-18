//! Preprocessing-token lexer.

use crate::diag::{Loc, Result, err};
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PKind {
    Ident,
    Number,
    Char,
    Str,
    Punct,
    Other,
    /// Raw text between `__asm` and `__endasm`.
    AsmBlock,
    /// A `#pragma` line that the preprocessor passes through (text = the pragma body).
    Pragma,
    Eof,
}

#[derive(Clone, Debug)]
pub struct PTok {
    pub kind: PKind,
    pub text: Rc<str>,
    pub loc: Loc,
    /// Preceded by whitespace.
    pub space: bool,
    /// First token on a line.
    pub bol: bool,
    pub hideset: Option<Rc<HashSet<Rc<str>>>>,
    /// Set for identifiers that must never be expanded again (painted blue).
    pub noexpand: bool,
}

impl PTok {
    pub fn is(&self, s: &str) -> bool {
        (self.kind == PKind::Punct || self.kind == PKind::Ident) && &*self.text == s
    }
    pub fn in_hideset(&self, name: &str) -> bool {
        self.hideset.as_ref().map_or(false, |h| h.contains(name))
    }
}

const PUNCTS: &[&str] = &[
    "<<=", ">>=", "...", "%:%:", "<%", "%>", "<:", ":>", "%:", "->", "++", "--", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "*=", "/=", "%=", "+=",
    "-=", "&=", "^=", "|=", "##", "[", "]", "(", ")", "{", "}", ".", "&", "*", "+", "-", "~", "!", "/", "%", "<",
    ">", "^", "|", "?", ":", ";", "=", ",", "#",
];

/// The primary spelling of a digraph, if `s` is one.
fn digraph(s: &str) -> Option<&'static str> {
    Some(match s {
        "<%" => "{",
        "%>" => "}",
        "<:" => "[",
        ":>" => "]",
        "%:" => "#",
        "%:%:" => "##",
        _ => return None,
    })
}

/// The character a trigraph `??x` stands for.
fn trigraph(c: char) -> Option<char> {
    Some(match c {
        '=' => '#',
        '(' => '[',
        '/' => '\\',
        ')' => ']',
        '\'' => '^',
        '<' => '{',
        '!' => '|',
        '>' => '}',
        '-' => '~',
        _ => return None,
    })
}

/// Remove backslash-newline sequences, keeping track of the original line of each char.
fn splice(src: &str) -> (Vec<char>, Vec<(u32, u32)>) {
    let mut chars: Vec<char> = src.chars().collect();
    // Translation phase 1: trigraph sequences.
    if src.contains("??") {
        let mut out = Vec::with_capacity(chars.len());
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '?' && i + 2 < chars.len() && chars[i + 1] == '?' {
                if let Some(c) = trigraph(chars[i + 2]) {
                    out.push(c);
                    // Keep the column count of the line stable by padding the replacement.
                    i += 3;
                    continue;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        chars = out;
    }
    let mut out = Vec::with_capacity(chars.len());
    let mut pos = Vec::with_capacity(chars.len());
    let (mut line, mut col) = (1u32, 1u32);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            let mut j = i + 1;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '\n' || chars[j] == '\r') {
                if chars[j] == '\r' && j + 1 < chars.len() && chars[j + 1] == '\n' {
                    j += 1;
                }
                i = j + 1;
                line += 1;
                col = 1;
                continue;
            }
        }
        if c == '\r' {
            if i + 1 < chars.len() && chars[i + 1] == '\n' {
                i += 1;
                continue;
            }
            out.push('\n');
            pos.push((line, col));
            line += 1;
            col = 1;
            i += 1;
            continue;
        }
        out.push(c);
        pos.push((line, col));
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        i += 1;
    }
    (out, pos)
}

fn is_ident_start(c: char) -> bool {
    // Characters outside ASCII are identifier characters (C23 6.4.2), except the private-use
    // block that holds bytes recovered from a non-UTF-8 source.
    c.is_ascii_alphabetic() || c == '_' || c == '$' || (c as u32 >= 0x80 && !('\u{E000}'..='\u{E0FF}').contains(&c))
}
fn is_ident_char(c: char) -> bool {
    c.is_ascii_digit() || is_ident_start(c)
}

/// A universal character name (`\uXXXX` / `\UXXXXXXXX`) at `i`, and its length in characters.
fn ucn_at(s: &[char], i: usize) -> Option<(char, usize)> {
    if s.get(i) != Some(&'\\') {
        return None;
    }
    let ndigits = match s.get(i + 1) {
        Some('u') => 4,
        Some('U') => 8,
        _ => return None,
    };
    let mut v: u32 = 0;
    for k in 0..ndigits {
        let d = s.get(i + 2 + k)?.to_digit(16)?;
        v = v * 16 + d;
    }
    Some((char::from_u32(v)?, 2 + ndigits))
}

pub fn tokenize(src: &str, file: u32) -> Result<Vec<PTok>> {
    let (s, pos) = splice(src);
    let mut toks: Vec<PTok> = Vec::new();
    let mut i = 0;
    let mut bol = true;
    let mut space = false;
    let n = s.len();
    let loc_at = |i: usize| -> Loc {
        let (line, col) = if i < pos.len() { pos[i] } else { pos.last().map(|p| (p.0 + 1, 1)).unwrap_or((1, 1)) };
        Loc { file, line, col }
    };
    // Tracks whether we're inside an #include line to lex <...> as a string.
    let mut line_toks_start = 0usize;
    while i < n {
        let c = s[i];
        if c == '\n' {
            i += 1;
            bol = true;
            space = false;
            line_toks_start = toks.len();
            continue;
        }
        if c == ' ' || c == '\t' || c == '\x0c' || c == '\x0b' {
            i += 1;
            space = true;
            continue;
        }
        if c == '/' && i + 1 < n && s[i + 1] == '/' {
            while i < n && s[i] != '\n' {
                i += 1;
            }
            space = true;
            continue;
        }
        if c == '/' && i + 1 < n && s[i + 1] == '*' {
            let start = i;
            i += 2;
            loop {
                if i + 1 >= n {
                    return err(loc_at(start), "unterminated comment");
                }
                if s[i] == '*' && s[i + 1] == '/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            space = true;
            continue;
        }
        let start = i;
        let loc = loc_at(i);
        let kind;
        // #include <...>
        let in_include = toks.len() == line_toks_start + 2
            && toks[line_toks_start].is("#")
            && toks[line_toks_start].bol
            && (&*toks[line_toks_start + 1].text == "include" || &*toks[line_toks_start + 1].text == "include_next");
        if c == '<' && in_include {
            let mut j = i + 1;
            while j < n && s[j] != '>' && s[j] != '\n' {
                j += 1;
            }
            if j < n && s[j] == '>' {
                i = j + 1;
                let text: String = s[start..i].iter().collect();
                toks.push(PTok {
                    kind: PKind::Str,
                    text: text.into(),
                    loc,
                    space,
                    bol,
                    hideset: None,
                    noexpand: false,
                });
                bol = false;
                space = false;
                continue;
            }
        }
        if c.is_ascii_digit() || (c == '.' && i + 1 < n && s[i + 1].is_ascii_digit()) {
            i += 1;
            while i < n {
                let d = s[i];
                if (d == 'e' || d == 'E' || d == 'p' || d == 'P') && i + 1 < n && (s[i + 1] == '+' || s[i + 1] == '-') {
                    i += 2;
                } else if is_ident_char(d) || d == '.' {
                    i += 1;
                } else if d == '\'' && i + 1 < n && s[i + 1].is_ascii_alphanumeric() {
                    // C23 digit separator
                    i += 1;
                } else {
                    break;
                }
            }
            kind = PKind::Number;
        } else if c == '"' || c == '\'' || ((c == 'L' || c == 'u' || c == 'U') && i + 1 < n && (s[i + 1] == '"' || s[i + 1] == '\''))
            || (c == 'u' && i + 2 < n && s[i + 1] == '8' && (s[i + 2] == '"' || s[i + 2] == '\''))
        {
            while s[i] != '"' && s[i] != '\'' {
                i += 1;
            }
            let q = s[i];
            i += 1;
            loop {
                if i >= n || s[i] == '\n' {
                    return err(loc, "unterminated literal");
                }
                if s[i] == '\\' {
                    i += 2;
                    continue;
                }
                if s[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            kind = if q == '"' { PKind::Str } else { PKind::Char };
        } else if is_ident_start(c) || ucn_at(&s, i).is_some() {
            // Identifiers may be spelled with universal character names; keep the character.
            let mut word = String::new();
            while i < n {
                if let Some((u, len)) = ucn_at(&s, i) {
                    word.push(u);
                    i += len;
                    continue;
                }
                if !is_ident_char(s[i]) {
                    break;
                }
                word.push(s[i]);
                i += 1;
            }
            if word == "__asm" {
                toks.push(PTok { kind: PKind::Ident, text: word.into(), loc, space, bol, hideset: None, noexpand: false });
                // Capture raw assembly text up to __endasm / _endasm.
                let raw_start = i;
                let mut j = i;
                let mut found = None;
                while j < n {
                    if (s[j] == '_') && (j == 0 || !is_ident_char(s[j - 1])) {
                        let mut k = j;
                        while k < n && is_ident_char(s[k]) {
                            k += 1;
                        }
                        let w: String = s[j..k].iter().collect();
                        if w == "__endasm" {
                            found = Some(j);
                            break;
                        }
                        j = k;
                        continue;
                    }
                    j += 1;
                }
                let Some(end) = found else {
                    return err(loc, "unterminated __asm block");
                };
                let raw: String = s[raw_start..end].iter().collect();
                toks.push(PTok {
                    kind: PKind::AsmBlock,
                    text: raw.into(),
                    loc: loc_at(raw_start),
                    space: true,
                    bol: false,
                    hideset: None,
                    noexpand: false,
                });
                i = end;
                bol = false;
                space = true;
                continue;
            }
            toks.push(PTok { kind: PKind::Ident, text: word.into(), loc, space, bol, hideset: None, noexpand: false });
            bol = false;
            space = false;
            continue;
        } else {
            let mut matched = None;
            for p in PUNCTS {
                let pc: Vec<char> = p.chars().collect();
                if i + pc.len() <= n && s[i..i + pc.len()] == pc[..] {
                    matched = Some(pc.len());
                    break;
                }
            }
            if let Some(l) = matched {
                i += l;
                kind = PKind::Punct;
                // Digraphs stand for their primary spelling.
                let text: String = s[start..i].iter().collect();
                if let Some(prim) = digraph(&text) {
                    toks.push(PTok { kind, text: prim.into(), loc, space, bol, hideset: None, noexpand: false });
                    bol = false;
                    space = false;
                    continue;
                }
            } else {
                i += 1;
                kind = PKind::Other;
            }
        }
        let text: String = s[start..i].iter().collect();
        toks.push(PTok { kind, text: text.into(), loc, space, bol, hideset: None, noexpand: false });
        bol = false;
        space = false;
    }
    Ok(toks)
}
