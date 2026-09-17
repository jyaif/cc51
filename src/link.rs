//! Code layout, branch relaxation, encoding and output formats.

use crate::asm::{self, Expr, Insn, Item, Mn, Op, Reach};
use std::collections::HashMap;
use std::fmt::Write;
use std::rc::Rc;

pub struct Section {
    pub name: Rc<str>,
    pub items: Vec<Item>,
    /// Fixed origin.
    pub org: Option<u32>,
}

pub struct LinkOut {
    pub image: Vec<u8>,
    pub syms: HashMap<Rc<str>, i64>,
    pub code_end: u32,
    pub sections: Vec<(Rc<str>, u32, u32)>,
    pub listing: String,
}

fn relaxable(i: &Insn) -> bool {
    match i.mn {
        Mn::Jmp => matches!(i.ops.first(), Some(Op::Code(_))),
        Mn::Call | Mn::Jc | Mn::Jnc | Mn::Jz | Mn::Jnz | Mn::Jb | Mn::Jnb | Mn::Jbc | Mn::Cjne | Mn::Djnz => true,
        _ => false,
    }
}

fn item_size(it: &Item, reach: Reach) -> u32 {
    match it {
        Item::Insn(i) => {
            if relaxable(i) {
                asm::insn_size(i, reach)
            } else {
                asm::insn_size(i, Reach::Short)
            }
        }
        Item::Db(v) => v.len() as u32,
        Item::Dw(v) => 2 * v.len() as u32,
        Item::Ds(n) => *n,
        _ => 0,
    }
}

pub fn link(sections: Vec<Section>, mut syms: HashMap<Rc<str>, i64>, code_start: u32, code_end: u32) -> Result<LinkOut, String> {
    // Flatten.
    let mut items: Vec<(usize, Item)> = Vec::new();
    for (si, s) in sections.iter().enumerate() {
        for it in &s.items {
            items.push((si, it.clone()));
        }
    }
    let n = items.len();
    let mut reach = vec![Reach::Short; n];
    for (i, (_, it)) in items.iter().enumerate() {
        if let Item::Insn(ins) = it {
            if ins.mn == Mn::Call {
                reach[i] = Reach::Abs11;
            }
            if matches!(ins.mn, Mn::Ljmp | Mn::Lcall) {
                reach[i] = Reach::Long;
            }
        }
    }
    let mut addr = vec![0u32; n];
    let mut sec_bounds: Vec<(u32, u32)> = vec![(0, 0); sections.len()];
    let mut iterations = 0;
    loop {
        iterations += 1;
        // Assign addresses.
        let mut pc = code_start;
        let mut cur_sec = usize::MAX;
        for i in 0..n {
            let (si, it) = &items[i];
            if *si != cur_sec {
                if cur_sec != usize::MAX {
                    sec_bounds[cur_sec].1 = pc;
                }
                cur_sec = *si;
                if let Some(org) = sections[*si].org {
                    pc = org;
                }
                sec_bounds[*si].0 = pc;
            }
            addr[i] = pc;
            if let Item::Label(l) = it {
                syms.insert(l.clone(), pc as i64);
            }
            pc += item_size(it, reach[i]);
        }
        if cur_sec != usize::MAX {
            sec_bounds[cur_sec].1 = pc;
        }
        // Sections without items.
        for (si, s) in sections.iter().enumerate() {
            if s.items.is_empty() {
                sec_bounds[si] = (0, 0);
            }
        }
        // Check reach.
        let mut changed = false;
        for i in 0..n {
            let Item::Insn(ins) = &items[i].1 else { continue };
            if !relaxable(ins) || reach[i] == Reach::Long {
                continue;
            }
            let is_cond = !matches!(ins.mn, Mn::Call | Mn::Jmp);
            let Some(t) = ins.target() else { continue };
            let Some(tv) = t.resolve(&|s| syms.get(s).copied()) else {
                return Err(format!("undefined symbol '{}'", t));
            };
            let size = item_size(&items[i].1, reach[i]);
            let next = addr[i] + size;
            let same_page = |after: u32| (tv as u32 & 0xf800) == (after & 0xf800);
            let ok = match (ins.mn, reach[i]) {
                (Mn::Call, _) => same_page(next),
                (Mn::Jmp, _) => asm::rel_fits(tv, next) || same_page(next),
                (_, Reach::CondAbs) => same_page(next),
                _ => asm::rel_fits(tv, next),
            };
            if !ok {
                if is_cond && reach[i] == Reach::Short && same_page(next + 2) {
                    reach[i] = Reach::CondAbs;
                } else {
                    reach[i] = Reach::Long;
                }
                changed = true;
            }
        }
        if !changed || iterations > 100 {
            break;
        }
    }
    // Check overlaps and bounds.
    let mut spans: Vec<(u32, u32, Rc<str>)> = sections.iter().enumerate().filter(|(_, s)| !s.items.is_empty()).map(|(i, s)| (sec_bounds[i].0, sec_bounds[i].1, s.name.clone())).collect();
    spans.sort();
    for w in spans.windows(2) {
        if w[0].1 > w[1].0 {
            return Err(format!("code sections '{}' and '{}' overlap", w[0].2, w[1].2));
        }
    }
    let end = spans.iter().map(|s| s.1).max().unwrap_or(code_start);
    if end > code_end {
        return Err(format!("program too large: code ends at {:#x}, limit {:#x}", end, code_end));
    }
    // Encode.
    let mut image = vec![0xffu8; end as usize];
    let mut listing = String::new();
    let lookup = |s: &str| syms.get(s).copied();
    for i in 0..n {
        let (_, it) = &items[i];
        let a = addr[i];
        let bytes: Vec<u8> = match it {
            Item::Insn(ins) => {
                let r = if ins.mn == Mn::Jmp && relaxable(ins) && reach[i] != Reach::Long {
                    let t = ins.target().unwrap().resolve(&lookup).unwrap();
                    if asm::rel_fits(t, a + 2) {
                        asm::encode(ins, a, Reach::Short, &lookup)
                    } else {
                        asm::encode(ins, a, Reach::Abs11, &lookup)
                    }
                } else if relaxable(ins) {
                    asm::encode(ins, a, reach[i], &lookup)
                } else {
                    asm::encode(ins, a, Reach::Short, &lookup)
                };
                r.map_err(|e| format!("{} at {:#06x}: {}", ins, a, e.0))?
            }
            Item::Db(v) => {
                let mut out = Vec::new();
                for e in v {
                    let x = e.resolve(&lookup).ok_or_else(|| format!("undefined symbol in .db: {}", e))?;
                    out.push(x as u8);
                }
                out
            }
            Item::Dw(v) => {
                let mut out = Vec::new();
                for e in v {
                    let x = e.resolve(&lookup).ok_or_else(|| format!("undefined symbol in .dw: {}", e))?;
                    out.push((x >> 8) as u8);
                    out.push(x as u8);
                }
                out
            }
            Item::Ds(k) => vec![0; *k as usize],
            Item::Label(l) => {
                let _ = writeln!(listing, "{:04x}          {}:", a, l);
                continue;
            }
            Item::Comment(c) => {
                let _ = writeln!(listing, "                ; {}", c);
                continue;
            }
        };
        if bytes.len() as u32 != item_size(it, reach[i]) {
            return Err(format!("internal error: size mismatch for {:?} at {:#06x} ({} vs {})", it, a, bytes.len(), item_size(it, reach[i])));
        }
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect::<Vec<_>>().join(" ");
        let text = match it {
            Item::Insn(ins) => format!("{}", ins),
            Item::Db(_) => ".db".to_string(),
            Item::Dw(_) => ".dw".to_string(),
            Item::Ds(k) => format!(".ds {}", k),
            _ => String::new(),
        };
        let _ = writeln!(listing, "{:04x}  {:<12}\t{}", a, hex, text);
        for (k, b) in bytes.iter().enumerate() {
            image[a as usize + k] = *b;
        }
    }
    let secs = sections.iter().enumerate().map(|(i, s)| (s.name.clone(), sec_bounds[i].0, sec_bounds[i].1 - sec_bounds[i].0)).collect();
    Ok(LinkOut { image, syms, code_end: end, sections: secs, listing })
}

pub fn intel_hex(image: &[u8], used: &[(u32, u32)]) -> String {
    let mut out = String::new();
    let mut ranges: Vec<(u32, u32)> = used.iter().copied().filter(|r| r.1 > r.0).collect();
    ranges.sort();
    // Merge adjacent ranges.
    let mut merged: Vec<(u32, u32)> = Vec::new();
    for r in ranges {
        if let Some(last) = merged.last_mut() {
            if r.0 <= last.1 {
                last.1 = last.1.max(r.1);
                continue;
            }
        }
        merged.push(r);
    }
    for (s, e) in merged {
        let mut a = s;
        while a < e {
            let len = (e - a).min(16);
            let mut sum: u32 = len + (a >> 8) + (a & 0xff);
            let _ = write!(out, ":{:02X}{:04X}00", len, a);
            for k in 0..len {
                let b = image[(a + k) as usize];
                sum += b as u32;
                let _ = write!(out, "{:02X}", b);
            }
            let _ = writeln!(out, "{:02X}", (0x100 - (sum & 0xff)) & 0xff);
            a += len;
        }
    }
    out.push_str(":00000001FF\n");
    out
}

pub fn expr_sym(s: &str) -> Expr {
    Expr::sym(&s.into())
}
