//! Internal RAM layout: globals, overlaid function frames and bit variables.

use std::collections::HashMap;
use std::rc::Rc;

/// A request for contiguous RAM.
#[derive(Clone, Debug)]
pub struct RamObj {
    pub sym: Rc<str>,
    pub size: u32,
    /// May (and should) be placed in the indirectly addressed upper RAM.
    pub upper: bool,
}

/// Frame of one function.
#[derive(Clone, Debug, Default)]
pub struct FrameReq {
    /// Contiguous objects (address-taken locals, aggregates).
    pub objects: Vec<RamObj>,
    /// Individually addressed byte slots.
    pub slots: Vec<Rc<str>>,
    /// Bit slots.
    pub bits: Vec<Rc<str>>,
}

pub struct LayoutInput {
    pub iram_size: u32,
    /// Registers banks in use (bank 0 always).
    pub banks: u8,
    pub global_bits: Vec<Rc<str>>,
    pub globals: Vec<RamObj>,
    /// Frames in caller-before-callee order with their parent (caller) indices.
    pub frames: Vec<FrameReq>,
    pub callers: Vec<Vec<usize>>,
    /// Frames that are roots of interrupt contexts (placed above all main-context frames).
    pub isr_roots: Vec<usize>,
    /// Internal RAM ranges taken by objects at fixed addresses.
    pub reserved: Vec<(u32, u32)>,
}

pub struct LayoutResult {
    pub syms: HashMap<Rc<str>, i64>,
    /// First free byte after all RAM data (the stack starts here).
    pub ram_end: u32,
    /// End of zero-initialized globals (for clearing).
    pub globals_end: u32,
    pub bit_bytes: u32,
    pub used: u32,
    /// Lowest address used by upper-RAM objects (iram_size if none).
    pub upper_start: u32,
}

/// Map from a virtual contiguous offset to a physical address, skipping the bit area.
struct Phys {
    segs: Vec<(u32, u32)>,
}

impl Phys {
    fn addr(&self, v: u32) -> Option<u32> {
        let mut off = v;
        for &(s, e) in &self.segs {
            let len = e - s;
            if off < len {
                return Some(s + off);
            }
            off -= len;
        }
        None
    }
    /// Smallest v' >= v such that [v', v'+len) lies inside one segment.
    fn fit(&self, v: u32, len: u32) -> u32 {
        let mut base = 0;
        for &(s, e) in &self.segs {
            let seglen = e - s;
            let start = v.max(base);
            if start + len <= base + seglen {
                return start;
            }
            base += seglen;
        }
        v.max(base)
    }
    fn capacity(&self) -> u32 {
        self.segs.iter().map(|(s, e)| e - s).sum()
    }
}

pub fn layout(inp: &LayoutInput) -> Result<LayoutResult, String> {
    let mut syms: HashMap<Rc<str>, i64> = HashMap::new();
    // ---- Bits ----
    let n = inp.frames.len();
    let order = topo(n, &inp.callers);
    let mut isr_ctx = vec![false; n];
    for &r in &inp.isr_roots {
        isr_ctx[r] = true;
    }
    for &f in &order {
        if inp.callers[f].iter().any(|c| isr_ctx[*c]) {
            isr_ctx[f] = true;
        }
    }
    // Physical bit numbers, skipping bytes taken by fixed-address objects.
    let reserved_byte = |a: u32| inp.reserved.iter().any(|&(s, e)| a >= s && a < e);
    let bit_nums: Vec<u32> = (0x20u32..0x30).filter(|b| !reserved_byte(*b)).flat_map(|b| (0..8).map(move |k| (b - 0x20) * 8 + k)).collect();
    let bit_at = |i: u32| -> u32 { bit_nums.get(i as usize).copied().unwrap_or(u32::MAX) };
    let gbits = inp.global_bits.len() as u32;
    for (i, b) in inp.global_bits.iter().enumerate() {
        syms.insert(b.clone(), bit_at(i as u32) as i64);
    }
    let mut bit_end_of = vec![0u32; n];
    let mut main_bits_end = gbits;
    for &f in &order {
        if isr_ctx[f] {
            continue;
        }
        let mut b = gbits;
        for &c in &inp.callers[f] {
            b = b.max(bit_end_of[c]);
        }
        for (i, name) in inp.frames[f].bits.iter().enumerate() {
            syms.insert(name.clone(), bit_at(b + i as u32) as i64);
        }
        bit_end_of[f] = b + inp.frames[f].bits.len() as u32;
        main_bits_end = main_bits_end.max(bit_end_of[f]);
    }
    let mut total_bits = main_bits_end;
    for &f in &order {
        if !isr_ctx[f] {
            continue;
        }
        let mut b = main_bits_end;
        for &c in &inp.callers[f] {
            if isr_ctx[c] {
                b = b.max(bit_end_of[c]);
            }
        }
        for (i, name) in inp.frames[f].bits.iter().enumerate() {
            syms.insert(name.clone(), bit_at(b + i as u32) as i64);
        }
        bit_end_of[f] = b + inp.frames[f].bits.len() as u32;
        total_bits = total_bits.max(bit_end_of[f]);
    }
    if total_bits as usize > bit_nums.len() {
        return Err(format!("too many bit variables ({})", total_bits));
    }
    // Bytes of the bit area actually used (fixed-address bytes inside it stay reserved).
    let bit_bytes = if total_bits == 0 { 0 } else { bit_at(total_bits - 1) / 8 + 1 };

    // ---- Bytes ----
    let reg_end = 8 * inp.banks as u32;
    let mut segs = Vec::new();
    let top = inp.iram_size.min(0x80);
    if bit_bytes > 0 {
        if reg_end < 0x20 {
            segs.push((reg_end, 0x20));
        }
        segs.push(((0x20 + bit_bytes).max(reg_end), top));
    } else {
        segs.push((reg_end, top));
    }
    // Exclude bytes taken by fixed-address objects.
    let mut segs2: Vec<(u32, u32)> = Vec::new();
    for (s, e) in segs {
        let mut cur = s;
        let mut cuts: Vec<(u32, u32)> = inp.reserved.iter().copied().filter(|&(rs, re)| re > s && rs < e).collect();
        cuts.sort();
        for (rs, re) in cuts {
            if rs > cur {
                segs2.push((cur, rs.min(e)));
            }
            cur = cur.max(re);
        }
        if cur < e {
            segs2.push((cur, e));
        }
    }
    let phys = Phys { segs: segs2 };
    let ram_top = inp.iram_size.min(0x100);
    // Upper RAM: globals first (top-down), then frames overlaid by call depth.
    let mut upper_used = 0u32;
    for g in inp.globals.iter().filter(|g| g.upper) {
        upper_used += g.size;
        syms.insert(g.sym.clone(), (ram_top - upper_used) as i64);
    }
    let upper_globals = upper_used;
    {
        let mut uend = vec![0u32; inp.frames.len()];
        let usize_of = |f: &FrameReq| -> u32 { f.objects.iter().filter(|o| o.upper).map(|o| o.size).sum() };
        let mut main_top = upper_globals;
        for &f in &order {
            if isr_ctx[f] {
                continue;
            }
            let mut b = upper_globals;
            for &c in &inp.callers[f] {
                b = b.max(uend[c]);
            }
            let mut cur = b;
            for o in inp.frames[f].objects.iter().filter(|o| o.upper) {
                cur += o.size;
                syms.insert(o.sym.clone(), ram_top as i64 - cur as i64);
            }
            uend[f] = cur;
            main_top = main_top.max(cur);
            let _ = usize_of;
        }
        let mut top = main_top;
        for &f in &order {
            if !isr_ctx[f] {
                continue;
            }
            let mut b = main_top;
            for &c in &inp.callers[f] {
                if isr_ctx[c] {
                    b = b.max(uend[c]);
                }
            }
            let mut cur = b;
            for o in inp.frames[f].objects.iter().filter(|o| o.upper) {
                cur += o.size;
                syms.insert(o.sym.clone(), ram_top as i64 - cur as i64);
            }
            uend[f] = cur;
            top = top.max(cur);
        }
        upper_used = top;
    }
    if upper_used > ram_top.saturating_sub(0x80) {
        return Err(format!("internal RAM exhausted: {} bytes of indirectly addressed objects", upper_used));
    }
    let upper_start = ram_top - upper_used;
    let mut v = 0u32;
    // Globals: contiguous each.
    for g in inp.globals.iter().filter(|g| !g.upper) {
        let at = phys.fit(v, g.size.max(1));
        let a = phys.addr(at).ok_or_else(|| "internal RAM exhausted by global variables".to_string())?;
        syms.insert(g.sym.clone(), a as i64);
        v = at + g.size;
    }
    let globals_v_end = v;
    let globals_end = if v == 0 { reg_end } else { phys.addr(v - 1).map(|a| a + 1).unwrap_or(top) };
    // Frames: objects first, then slots.
    let fsize = |f: &FrameReq| -> u32 { f.objects.iter().map(|o| o.size).sum::<u32>() + f.slots.len() as u32 };
    let mut base = vec![0u32; n];
    let mut main_end = globals_v_end;
    let mut place = |f: usize, start: u32, syms: &mut HashMap<Rc<str>, i64>| -> Result<u32, String> {
        let fr = &inp.frames[f];
        let mut cur = start;
        for o in fr.objects.iter().filter(|o| !o.upper) {
            let at = phys.fit(cur, o.size.max(1));
            let a = phys.addr(at).ok_or_else(|| format!("internal RAM exhausted placing frame object {}", o.sym))?;
            syms.insert(o.sym.clone(), a as i64);
            cur = at + o.size;
        }
        for s in &fr.slots {
            let a = phys.addr(cur).ok_or_else(|| format!("internal RAM exhausted placing {}", s))?;
            syms.insert(s.clone(), a as i64);
            cur += 1;
        }
        Ok(cur)
    };
    let mut end_of = vec![0u32; n];
    // Main context.
    for &f in &order {
        if isr_ctx[f] {
            continue;
        }
        let mut b = globals_v_end;
        for &c in &inp.callers[f] {
            b = b.max(end_of[c]);
        }
        base[f] = b;
        end_of[f] = place(f, b, &mut syms)?;
        main_end = main_end.max(end_of[f]);
    }
    // Interrupt contexts on top.
    let mut isr_end = main_end;
    for &f in &order {
        if !isr_ctx[f] {
            continue;
        }
        let mut b = main_end;
        for &c in &inp.callers[f] {
            if isr_ctx[c] {
                b = b.max(end_of[c]);
            }
        }
        base[f] = b;
        end_of[f] = place(f, b, &mut syms)?;
        isr_end = isr_end.max(end_of[f]);
    }
    let _ = fsize;
    let used = isr_end;
    if used > phys.capacity() {
        return Err(format!("internal RAM exhausted: need {} bytes, have {}", used, phys.capacity()));
    }
    let ram_end = if used == 0 { reg_end.max(if bit_bytes > 0 { 0x20 + bit_bytes } else { 0 }) } else { phys.addr(used - 1).unwrap() + 1 };
    let ram_end = ram_end.max(if bit_bytes > 0 { 0x20 + bit_bytes } else { reg_end });
    Ok(LayoutResult { syms, ram_end, globals_end, bit_bytes, used, upper_start })
}

/// Topological order (callers before callees). Cycles are broken arbitrarily.
fn topo(n: usize, callers: &[Vec<usize>]) -> Vec<usize> {
    let mut indeg: Vec<usize> = callers.iter().map(|c| c.len()).collect();
    let mut callees: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (f, cs) in callers.iter().enumerate() {
        for &c in cs {
            callees[c].push(f);
        }
    }
    let mut q: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    q.reverse();
    let mut out = Vec::new();
    let mut seen = vec![false; n];
    while out.len() < n {
        if let Some(f) = q.pop() {
            if seen[f] {
                continue;
            }
            seen[f] = true;
            out.push(f);
            for &g in &callees[f] {
                indeg[g] = indeg[g].saturating_sub(1);
                if indeg[g] == 0 {
                    q.push(g);
                }
            }
        } else {
            // cycle: pick any unseen
            let f = (0..n).find(|&i| !seen[i]).unwrap();
            indeg[f] = 0;
            q.push(f);
        }
    }
    out
}
