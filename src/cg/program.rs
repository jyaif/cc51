//! Whole-program backend driver.

use super::alloc::{self, AllocCtx};
use super::fold;
use super::select::{self as sel, Gen, GenCtx};
use super::layout::{self, FrameReq, LayoutInput, RamObj};
use super::*;
use crate::asm::parse::{AsmParser, SymRes};
use crate::asm::{Expr, Item, Mn};
use crate::ast::{Linkage, Program, RelocTarget};
use crate::ir::build::build_func;
use crate::ir::{self, Callee, Func, Inst, Mem, ParamLoc, Sym, Term, Ty, Val};
use crate::link::{self, Section};
use crate::types::Space;
use std::collections::{HashMap, HashSet};

pub struct Options {
    pub opt: u32,
    pub code_start: u32,
    pub code_size: u32,
    pub iram_size: u32,
    pub xram_start: u32,
    pub xram_size: u32,
    pub dump_ir: bool,
    pub dump_ir_raw: bool,
    pub verbose: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { opt: 2, code_start: 0, code_size: 0x10000, iram_size: 256, xram_start: 0, xram_size: 0x10000, dump_ir: false, dump_ir_raw: false, verbose: false }
    }
}

pub struct Output {
    pub image: Vec<u8>,
    pub hex: String,
    pub listing: String,
    pub map: String,
    pub code_size: u32,
    pub ram_used: u32,
}

const RUNTIME: &str = include_str!("../../runtime/rt.s");

struct RtModule {
    labels: Vec<String>,
    text: String,
}

fn runtime_modules() -> Vec<RtModule> {
    let mut mods = Vec::new();
    let mut cur: Option<RtModule> = None;
    for line in RUNTIME.lines() {
        if let Some(_name) = line.strip_prefix(";;; module ") {
            if let Some(m) = cur.take() {
                mods.push(m);
            }
            cur = Some(RtModule { labels: vec![], text: String::new() });
            continue;
        }
        if let Some(m) = cur.as_mut() {
            let t = line.trim();
            if let Some(l) = t.strip_suffix(':') {
                if !l.contains(char::is_whitespace) {
                    m.labels.push(l.to_string());
                }
            }
            m.text.push_str(line);
            m.text.push('\n');
        }
    }
    if let Some(m) = cur {
        mods.push(m);
    }
    mods
}

fn helper_summary(name: &str) -> Summary {
    let r = |v: &[u8]| v.iter().map(|x| Loc::R(*x)).collect::<Vec<_>>();
    let (params, ret) = match name {
        "__mulint" | "__divuint" | "__divsint" | "__moduint" | "__modsint" => (vec![r(&[7, 6]), r(&[3, 2])], r(&[7, 6])),
        n if n.starts_with("__fs") && (n.ends_with("2sl") || n.ends_with("2ul")) => (vec![r(&[7, 6, 5, 4])], r(&[7, 6, 5, 4])),
        "__sl2fs" | "__ul2fs" => (vec![r(&[7, 6, 5, 4])], r(&[7, 6, 5, 4])),
        "__fseq" | "__fslt" => (vec![r(&[7, 6, 5, 4]), r(&[3, 2, 1, 0])], vec![ACC]),
        _ => (vec![r(&[7, 6, 5, 4]), r(&[3, 2, 1, 0])], r(&[7, 6, 5, 4])),
    };
    Summary { params, ret, clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false }
}

/// Fixed calling convention (address-taken functions).
fn fixed_convention(f: &Func) -> (Vec<Vec<Loc>>, Vec<Loc>) {
    let order = [7u8, 6, 5, 4, 3, 2];
    let mut k = 0usize;
    let mut slot = 0u16;
    let mut params = Vec::new();
    for (i, p) in f.params.iter().enumerate() {
        match p {
            ParamLoc::Reg(_) => {
                let n = if f.param_tys[i] == Ty::Bit { 1 } else { f.param_tys[i].bytes() };
                let mut v = Vec::new();
                for _ in 0..n {
                    if f.param_tys[i] == Ty::Bit {
                        v.push(Loc::BitSlot(f.id, slot));
                        slot += 1;
                    } else if k < order.len() {
                        v.push(Loc::R(order[k]));
                        k += 1;
                    } else {
                        v.push(Loc::Slot(f.id, slot));
                        slot += 1;
                    }
                }
                params.push(v);
            }
            ParamLoc::Frame(_) => params.push(vec![]),
        }
    }
    (params, ret_convention(f.ret))
}

fn ret_convention(ret: Option<Ty>) -> Vec<Loc> {
    match ret {
        None => vec![],
        Some(Ty::Bit) => vec![CARRY],
        Some(Ty::I8) => vec![ACC],
        Some(t) => [7u8, 6, 5, 4, 3, 2, 1, 0].iter().take(t.bytes() as usize).map(|r| Loc::R(*r)).collect(),
    }
}

fn sdcc_convention(f: &Func) -> (Vec<Vec<Loc>>, Vec<Loc>) {
    let dpl = [Loc::Dir(0x82), Loc::Dir(0x83), Loc::Dir(0xF0), ACC];
    let mut params = Vec::new();
    for (i, t) in f.param_tys.iter().enumerate() {
        if i == 0 {
            params.push(dpl[..t.bytes().min(4) as usize].to_vec());
        } else {
            params.push((0..t.bytes()).map(|k| Loc::Obj(f.id, 1000 + i as u32, k as u16)).collect());
        }
    }
    let ret = match f.ret {
        None => vec![],
        Some(Ty::Bit) => vec![CARRY],
        Some(t) => dpl[..t.bytes().min(4) as usize].to_vec(),
    };
    (params, ret)
}

pub fn compile(prog: &Program, opts: &Options) -> Result<Output, String> {
    match compile_with(prog, opts, false) {
        Err(e) if e.contains("internal RAM exhausted") && opts.iram_size > 128 => compile_with(prog, opts, true),
        r => r,
    }
}

fn compile_with(prog: &Program, opts: &Options, upper_objects: bool) -> Result<Output, String> {
    ir::set_volatile_syms(
        prog.globals
            .iter()
            .enumerate()
            .filter(|(_, g)| g.volatile || matches!(g.space, Space::Sfr | Space::Sbit))
            .map(|(i, _)| Sym::Global(i))
            .collect(),
    );
    // ---- Symbol names ----
    let mut name_count: HashMap<String, usize> = HashMap::new();
    for g in &prog.globals {
        *name_count.entry(g.name.to_string()).or_default() += 1;
    }
    for f in &prog.funcs {
        *name_count.entry(f.name.to_string()).or_default() += 1;
    }
    let gnames: Vec<Rc<str>> = prog
        .globals
        .iter()
        .enumerate()
        .map(|(i, g)| {
            if g.linkage == Linkage::External || name_count[&*g.name] == 1 {
                format!("_{}", g.name).into()
            } else {
                format!("_{}${}", g.name, i).into()
            }
        })
        .collect();
    let fnames: Vec<Rc<str>> = prog
        .funcs
        .iter()
        .enumerate()
        .map(|(i, f)| {
            if f.linkage == Linkage::External || name_count[&*f.name] == 1 {
                format!("_{}", f.name).into()
            } else {
                format!("_{}${}", f.name, i).into()
            }
        })
        .collect();
    set_global_names(gnames.clone());

    // ---- Build IR ----
    ir::build::set_vararg_sizes(prog.vararg_sizes());
    let mut funcs: Vec<Option<Func>> = Vec::with_capacity(prog.funcs.len());
    for (fid, af) in prog.funcs.iter().enumerate() {
        if af.body.is_none() {
            funcs.push(None);
            continue;
        }
        let f = build_func(prog, fid).map_err(|e| e.to_string())?;
        funcs.push(Some(f));
    }

    // Symbol resolution for inline assembly (per translation unit).
    let asm_resolver = |tu: usize, name: &str| -> Option<SymRes> {
        let base = name.strip_prefix('_')?;
        // Static in the same TU first.
        for (i, g) in prog.globals.iter().enumerate() {
            if &*g.name == base && g.tu == tu && g.linkage == Linkage::Internal {
                return Some(glob_res(prog, i, &gnames));
            }
        }
        for (i, f) in prog.funcs.iter().enumerate() {
            if &*f.name == base && f.tu == tu && f.linkage == Linkage::Internal && f.body.is_some() {
                return Some(SymRes::Sym(fnames[i].clone()));
            }
        }
        match prog.externs.get(base) {
            Some(crate::ast::Sym::Global(g)) => Some(glob_res(prog, *g, &gnames)),
            Some(crate::ast::Sym::Func(f)) => Some(SymRes::Sym(fnames[*f].clone())),
            None => {
                // SDCC naked function parameter area: _func_PARM_n
                if let Some(pos) = base.rfind("_PARM_") {
                    let fname = &base[..pos];
                    if let Ok(n) = base[pos + 6..].parse::<u32>() {
                        if let Some(crate::ast::Sym::Func(f)) = prog.externs.get(fname) {
                            return Some(SymRes::Sym(frame_obj_sym(*f, 1000 + n - 1)));
                        }
                    }
                }
                None
            }
        }
    };

    // ---- Reachability ----
    let mut roots: Vec<usize> = Vec::new();
    for (fid, f) in prog.funcs.iter().enumerate() {
        if funcs[fid].is_none() {
            continue;
        }
        let ft = f.ftype();
        if ft.attrs.interrupt.is_some() || f.name.starts_with("__asm_block_") || (&*f.name == "main" && f.linkage == Linkage::External) {
            roots.push(fid);
        }
    }
    if !roots.iter().any(|&f| &*prog.funcs[f].name == "main") {
        return Err("no definition of 'main'".into());
    }
    let asm_syms = |text: &str, tu: usize| -> Vec<Rc<str>> {
        let resolve = |n: &str| asm_resolver(tu, n);
        let mut p = AsmParser::new(&resolve, 0);
        let mut out = Vec::new();
        if let Ok(items) = p.parse(text) {
            for it in items {
                if let Item::Insn(i) = it {
                    for o in &i.ops {
                        let e = match o {
                            crate::asm::Op::Imm(e) | crate::asm::Op::Dir(e) | crate::asm::Op::Bit(e) | crate::asm::Op::Code(e) => e,
                            _ => continue,
                        };
                        if let Some(s) = &e.sym {
                            out.push(s.clone());
                        }
                    }
                } else if let Item::Db(v) | Item::Dw(v) = it {
                    for e in v {
                        if let Some(s) = &e.sym {
                            out.push(s.clone());
                        }
                    }
                }
            }
        }
        out
    };
    let name_to_sym: HashMap<Rc<str>, crate::ast::Sym> = gnames
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), crate::ast::Sym::Global(i)))
        .chain(fnames.iter().enumerate().map(|(i, n)| (n.clone(), crate::ast::Sym::Func(i))))
        .collect();

    // IR parameter types of every function.
    let ptys: Vec<Vec<Ty>> = prog
        .funcs
        .iter()
        .map(|af| af.ftype().params.iter().map(|t| if t.is_scalar() { ir::build::ir_ty(prog, t) } else { Ty::I16 }).collect())
        .collect();
    let param_tys = |c: &Callee| -> Option<Vec<Ty>> {
        match c {
            Callee::Direct(f) => Some(ptys[*f].clone()),
            _ => None,
        }
    };
    let ocx = crate::opt::OptCtx { level: opts.opt, param_tys: &param_tys };
    // Optimize every function body once.
    for f in funcs.iter_mut().flatten() {
        if opts.dump_ir_raw {
            eprintln!("{}", ir::print::func(f));
        }
        crate::opt::optimize_func(f, &ocx);
    }
    let compute_reach = |funcs: &Vec<Option<Func>>| -> (HashSet<usize>, HashSet<usize>, HashSet<String>) {
        let mut reach_f: HashSet<usize> = HashSet::new();
        let mut reach_g: HashSet<usize> = HashSet::new();
        let mut runtime_used: HashSet<String> = HashSet::new();
        let mut work: Vec<crate::ast::Sym> = roots.iter().map(|f| crate::ast::Sym::Func(*f)).collect();
        while let Some(s) = work.pop() {
            match s {
                crate::ast::Sym::Global(g) => {
                    if !reach_g.insert(g) {
                        continue;
                    }
                    if let Some(init) = &prog.globals[g].init {
                        for r in &init.relocs {
                            work.push(match r.target {
                                RelocTarget::Global(x) => crate::ast::Sym::Global(x),
                                RelocTarget::Func(x) => crate::ast::Sym::Func(x),
                            });
                        }
                    }
                }
                crate::ast::Sym::Func(fid) => {
                    if !reach_f.insert(fid) {
                        continue;
                    }
                    let Some(f) = funcs[fid].as_ref() else { continue };
                    let tu = prog.funcs[fid].tu;
                    let add_sym = |s: &Sym, work: &mut Vec<crate::ast::Sym>| match s {
                        Sym::Global(g) => work.push(crate::ast::Sym::Global(*g)),
                        Sym::Func(x) => work.push(crate::ast::Sym::Func(*x)),
                        _ => {}
                    };
                    for b in &f.blocks {
                        for ins in &b.insts {
                            ins.for_each_val(|v| {
                                if let Val::Addr(s, _) = v {
                                    add_sym(s, &mut work);
                                }
                            });
                            match ins {
                                Inst::Load(_, Mem::Sym(s, _)) | Inst::Store(Mem::Sym(s, _), _, _) => add_sym(s, &mut work),
                                Inst::MemCopy(a, b, _) => {
                                    if let Mem::Sym(s, _) = a {
                                        add_sym(s, &mut work);
                                    }
                                    if let Mem::Sym(s, _) = b {
                                        add_sym(s, &mut work);
                                    }
                                }
                                Inst::MemSet(Mem::Sym(s, _), _, _) => add_sym(s, &mut work),
                                _ => {}
                            }
                            match ins {
                                Inst::Call(_, Callee::Direct(x), _) => work.push(crate::ast::Sym::Func(*x)),
                                Inst::Call(_, Callee::Runtime(n), _) => {
                                    runtime_used.insert(n.to_string());
                                }
                                Inst::Call(_, Callee::Indirect(_), _) => {
                                    runtime_used.insert("__call_dptr".into());
                                }
                                Inst::Asm(t) => {
                                    for s in asm_syms(t, tu) {
                                        if let Some(x) = name_to_sym.get(&s) {
                                            work.push(*x);
                                        } else {
                                            runtime_used.insert(s.to_string());
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        (reach_f, reach_g, runtime_used)
    };
    let (reach_f0, _, _) = compute_reach(&funcs);
    // ---- Inlining ----
    if opts.opt > 0 {
        let order = bottom_up(&funcs, &reach_f0);
        let keep = |f: usize| roots.contains(&f) || prog.funcs[f].addr_taken;
        let is_inline = |f: usize| prog.funcs[f].is_inline;
        crate::opt::inline::run(&mut funcs, &order, &keep, &is_inline, &|f: &mut Func| crate::opt::optimize_func(f, &ocx));
    }
    let (reach_f, reach_g, mut runtime_used) = compute_reach(&funcs);
    // Direct references to undefined functions are errors.
    for &fid in &reach_f {
        if funcs[fid].is_none() {
            let name = &prog.funcs[fid].name;
            let known_rt = runtime_modules().iter().any(|m| m.labels.iter().any(|l| l == &format!("_{}", name)));
            if !known_rt {
                return Err(format!("undefined reference to '{}'", name));
            }
        }
    }
    for &g in &reach_g {
        let gl = &prog.globals[g];
        if !gl.defined && !gl.tentative && gl.init.is_none() && gl.at.is_none() {
            return Err(format!("undefined reference to '{}'", gl.name));
        }
    }

    // Objects placed in the upper (indirectly addressed) internal RAM.
    let idata_globals: HashSet<usize> = reach_g
        .iter()
        .copied()
        .filter(|&g| {
            let gl = &prog.globals[g];
            gl.at.is_none() && opts.iram_size > 128 && upper_objects && (gl.space == Space::Idata || (gl.space == Space::Data && (gl.ty.is_array() || gl.ty.is_record())))
        })
        .collect();
    if upper_objects {
        for f in funcs.iter_mut().flatten() {
            for o in f.frame.iter_mut() {
                if o.space == Space::Data && &*o.name != "__varargs" && o.param.is_none() {
                    o.space = Space::Idata;
                }
            }
        }
    }
    let global_space = |g: usize| -> Space {
        if idata_globals.contains(&g) {
            Space::Idata
        } else if prog.globals[g].space == Space::Idata {
            Space::Data
        } else {
            prog.globals[g].space
        }
    };
    // ---- Call graph ----
    let fids: Vec<usize> = {
        let mut v: Vec<usize> = reach_f.iter().copied().filter(|f| funcs[*f].is_some()).collect();
        v.sort();
        v
    };
    let index: HashMap<usize, usize> = fids.iter().enumerate().map(|(i, f)| (*f, i)).collect();
    let n = fids.len();
    let addr_taken: Vec<bool> = fids.iter().map(|f| prog.funcs[*f].addr_taken && !prog.funcs[*f].name.starts_with("__asm_block_")).collect();
    let mut callees: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut has_indirect = vec![false; n];
    for (i, &fid) in fids.iter().enumerate() {
        let f = funcs[fid].as_ref().unwrap();
        for b in &f.blocks {
            for ins in &b.insts {
                match ins {
                    Inst::Call(_, Callee::Direct(x), _) => {
                        if let Some(j) = index.get(x) {
                            if !callees[i].contains(j) {
                                callees[i].push(*j);
                            }
                        }
                    }
                    Inst::Call(_, Callee::Indirect(_), _) => has_indirect[i] = true,
                    _ => {}
                }
            }
        }
    }
    for i in 0..n {
        if has_indirect[i] {
            for j in 0..n {
                if addr_taken[j] && !callees[i].contains(&j) {
                    callees[i].push(j);
                }
            }
        }
    }
    // Strongly connected components (recursion).
    let scc_id: Vec<usize> = {
        // Tarjan's algorithm (iterative-friendly recursion depth is fine for these program sizes).
        struct T<'a> {
            callees: &'a Vec<Vec<usize>>,
            index: Vec<i64>,
            low: Vec<i64>,
            on: Vec<bool>,
            stack: Vec<usize>,
            next: i64,
            comp: Vec<usize>,
            ncomp: usize,
        }
        fn strong(t: &mut T, v: usize) {
            t.index[v] = t.next;
            t.low[v] = t.next;
            t.next += 1;
            t.stack.push(v);
            t.on[v] = true;
            for k in 0..t.callees[v].len() {
                let w = t.callees[v][k];
                if t.index[w] < 0 {
                    strong(t, w);
                    t.low[v] = t.low[v].min(t.low[w]);
                } else if t.on[w] {
                    t.low[v] = t.low[v].min(t.index[w]);
                }
            }
            if t.low[v] == t.index[v] {
                loop {
                    let w = t.stack.pop().unwrap();
                    t.on[w] = false;
                    t.comp[w] = t.ncomp;
                    if w == v {
                        break;
                    }
                }
                t.ncomp += 1;
            }
        }
        let mut t = T { callees: &callees, index: vec![-1; n], low: vec![0; n], on: vec![false; n], stack: vec![], next: 0, comp: vec![0; n], ncomp: 0 };
        for v in 0..n {
            if t.index[v] < 0 {
                strong(&mut t, v);
            }
        }
        t.comp
    };
    let recursive: Vec<bool> = (0..n).map(|i| callees[i].contains(&i) || (0..n).any(|j| j != i && scc_id[j] == scc_id[i])).collect();
    for i in 0..n {
        if recursive[i] {
            // Recursive functions save their frame bytes with push/pop: keep them directly addressable.
            for o in funcs[fids[i]].as_mut().unwrap().frame.iter_mut() {
                if o.space == Space::Idata {
                    o.space = Space::Data;
                }
            }
            let f = funcs[fids[i]].as_ref().unwrap();
            if f.attrs.interrupt.is_some() {
                return Err(format!("interrupt handler '{}' cannot be recursive", f.name));
            }
            crate::diag::warn(prog.funcs[fids[i]].loc, format!("'{}' is recursive: its stack usage is unbounded", f.name));
        }
    }
    // Bottom-up order (callees first).
    let mut post = Vec::new();
    {
        let mut seen = vec![false; n];
        fn visit(v: usize, callees: &Vec<Vec<usize>>, seen: &mut Vec<bool>, post: &mut Vec<usize>) {
            seen[v] = true;
            for &w in &callees[v] {
                if !seen[w] {
                    visit(w, callees, seen, post);
                }
            }
            post.push(v);
        }
        for i in 0..n {
            if !seen[i] {
                visit(i, &callees, &mut seen, &mut post);
            }
        }
    }

    // ---- Code generation ----
    let fname_index: HashMap<Rc<str>, usize> = fnames.iter().enumerate().map(|(i, n)| (n.clone(), i)).collect();
    let mut summaries: Vec<Option<Summary>> = vec![None; prog.funcs.len()];
    // Summaries of naked and address-taken functions are fixed up-front.
    for (i, &fid) in fids.iter().enumerate() {
        let f = funcs[fid].as_ref().unwrap();
        if f.attrs.naked {
            let (p, r) = sdcc_convention(f);
            summaries[fid] = Some(Summary { params: p, ret: r, clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false });
        } else if recursive[i] {
            let (p, r) = fixed_convention(f);
            summaries[fid] = Some(Summary { params: p, ret: r, clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false });
        }
    }
    let mut codes: Vec<(usize, Vec<Item>)> = Vec::new();
    let mut frames: Vec<FrameReq> = vec![FrameReq::default(); n];
    let mut total_clobbers: Vec<RegSet> = vec![0; prog.funcs.len()];
    // Transitive use of B / DPTR (for interrupt context saving).
    let mut total_uses_b: Vec<bool> = vec![false; prog.funcs.len()];
    let mut total_uses_dptr: Vec<bool> = vec![false; prog.funcs.len()];
    for &i in &post {
        let fid = fids[i];
        // Fixed-convention functions: parameters get their own vregs so they can move off their register.
        let f_owned: Func = {
            let mut f = funcs[fid].as_ref().unwrap().clone();
            if (addr_taken[i] || recursive[i]) && !f.attrs.naked {
                split_params(&mut f);
            }
            f
        };
        let f = &f_owned;
        let callee_summary = |c: &Callee| -> Summary {
            match c {
                Callee::Direct(x) => summaries[*x].clone().unwrap_or_else(|| {
                    // Undefined (runtime/asm) function: assume SDCC convention.
                    let (p, r) = (vec![], vec![ACC]);
                    Summary { params: p, ret: r, clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false }
                }),
                Callee::Indirect(_) => {
                    // Callee must use the fixed convention; take any address-taken function's shape: computed per call.
                    Summary { params: vec![], ret: vec![], clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false }
                }
                Callee::Runtime(n) => helper_summary(n),
            }
        };
        let is_naked = f.attrs.naked;
        let fixed = if addr_taken[i] || recursive[i] { Some(fixed_convention(f)) } else { None };
        let ret_locs = if is_naked {
            sdcc_convention(f).1
        } else {
            fixed.as_ref().map(|x| x.1.clone()).unwrap_or_else(|| ret_convention(f.ret))
        };
        let mem_space = |m: &Mem| -> Option<Space> {
            match m {
                Mem::Sym(Sym::Global(g), _) => Some(global_space(*g)),
                Mem::Sym(Sym::Frame(ff, o), _) => funcs[*ff].as_ref().map(|x| x.frame[*o as usize].space),
                _ => None,
            }
        };
        let fold = fold::compute(f, &mem_space);
        // Indirect calls: give the IR callee a fixed-convention summary based on argument types.
        let indirect_summary = |args: &[Val], ret: Option<Ty>| -> Summary {
            let order = [7u8, 6, 5, 4, 3, 2];
            let mut k = 0;
            let mut params = Vec::new();
            for a in args {
                let t = match a {
                    Val::R(r) => f.ty(*r),
                    _ => Ty::I16,
                };
                let mut v = Vec::new();
                for _ in 0..t.bytes() {
                    if k < order.len() {
                        v.push(Loc::R(order[k]));
                        k += 1;
                    }
                }
                params.push(v);
            }
            Summary { params, ret: ret_convention(ret), clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false }
        };
        let _ = &indirect_summary;
        let callee_for_alloc = |c: &Callee| -> Summary { callee_summary(c) };
        let reserved: RegSet = 0;
        let actx = AllocCtx {
            f,
            fold: &fold,
            callee: &callee_for_alloc,
            fixed_params: fixed.as_ref().map(|x| x.0.clone()),
            ret_locs: ret_locs.clone(),
            reserved,
            min_reg_weight: if f.attrs.interrupt.is_some() { 3 } else { 0 },
            no_bit_slots: recursive[i],
        };
        let al = alloc::allocate(&actx);
        if opts.dump_ir {
            eprintln!("{}", ir::print::func(f));
            for (v, l) in al.locs.iter().enumerate() {
                if !l.is_empty() {
                    eprintln!("  %{} -> {:?}", v, l);
                }
            }
            for (v, k) in fold.kind.iter().enumerate() {
                if *k != fold::FoldKind::None {
                    eprintln!("  %{} folded {:?}", v, k);
                }
            }
        }
        let tu = prog.funcs[fid].tu;
        let resolve = |name: &str| asm_resolver(tu, name);
        let sym_space = |s: &Sym| -> Space {
            match s {
                Sym::Global(g) => global_space(*g),
                Sym::Func(_) => Space::Code,
                Sym::Frame(ff, o) => funcs[*ff].as_ref().map(|x| x.frame[*o as usize].space).unwrap_or(Space::Data),
                Sym::Named(_) => Space::Data,
            }
        };
        let sym_expr = |s: &Sym| -> Rc<str> {
            match s {
                Sym::Global(g) => gnames[*g].clone(),
                Sym::Func(x) => fnames[*x].clone(),
                Sym::Frame(ff, o) => frame_obj_sym(*ff, *o),
                Sym::Named(n) => (*n).into(),
            }
        };
        let callee_sym = |c: &Callee| -> Rc<str> {
            match c {
                Callee::Direct(x) => fnames[*x].clone(),
                Callee::Indirect(_) => "__call_dptr".into(),
                Callee::Runtime(n) => (*n).into(),
            }
        };
        // Indirect calls need per-call summaries: wrap the callee function.
        let call_sum = |c: &Callee| -> Summary {
            match c {
                Callee::Indirect(_) => Summary { params: vec![], ret: vec![], clobbers: ALL_REGS, keeps_b: false, keeps_dptr: false },
                _ => callee_summary(c),
            }
        };
        let my_scc = scc_id[i];
        let is_rec = recursive[i];
        let same_scc = |c: &Callee| -> bool {
            match c {
                Callee::Direct(x) => is_rec && index.get(x).map_or(false, |&j| scc_id[j] == my_scc),
                // An indirect call may reach any address-taken function, including this one.
                Callee::Indirect(_) => is_rec,
                _ => false,
            }
        };
        let gcx = GenCtx { callee: &call_sum, callee_sym: &callee_sym, sym_space: &sym_space, sym_expr: &sym_expr, asm_resolve: &resolve, same_scc: &same_scc };
        let bank = f.attrs.using.unwrap_or(0);
        // Rewrite indirect calls' argument locations by giving them explicit summaries at codegen time.
        let f_for_gen: Func = rewrite_indirect(f, &indirect_summary);
        let g = Gen::new(&gcx, &f_for_gen, &al, &fold, ret_locs.clone(), bank);
        let code = g.run(fnames[fid].clone());
        let mut items = code.items;
        // Peephole optimization.
        {
            let fname_to_fid: &HashMap<Rc<str>, usize> = &fname_index;
            let call_uses = |name: &str| -> peep::Res {
                let locs_res = |locs: &[Loc]| -> peep::Res {
                    let mut r = 0;
                    for l in locs {
                        match l {
                            Loc::R(n) => r |= peep::reg_res(*n),
                            Loc::Dir(0xE0) => r |= peep::R_A,
                            Loc::Dir(0xF0) => r |= peep::R_B,
                            Loc::Dir(0x82) => r |= peep::R_DPL,
                            Loc::Dir(0x83) => r |= peep::R_DPH,
                            _ => {}
                        }
                    }
                    r
                };
                if name == "__call_dptr" {
                    return peep::R_ALL;
                }
                if let Some(&cf) = fname_to_fid.get(name) {
                    return match &summaries[cf] {
                        Some(sm) => sm.params.iter().map(|p| locs_res(p)).fold(0, |a, b| a | b),
                        None => peep::R_ALL,
                    };
                }
                if name.starts_with("__") {
                    let sm = helper_summary(name);
                    let r = sm.params.iter().map(|p| locs_res(p)).fold(0, |a, b| a | b);
                    // A/B based helpers.
                    return r | peep::R_A | peep::R_B | peep::R_DPTR;
                }
                peep::R_ALL
            };
            let ret_uses = if f.attrs.interrupt.is_some() {
                peep::R_ALL
            } else {
                let mut r = 0;
                for l in &ret_locs {
                    r |= match l {
                        Loc::R(n) => peep::reg_res(*n),
                        Loc::Dir(0xE0) => peep::R_A,
                        Loc::Dir(0xF0) => peep::R_B,
                        Loc::Dir(0x82) => peep::R_DPL,
                        Loc::Dir(0x83) => peep::R_DPH,
                        Loc::BitAbs(0xD7) => peep::R_C,
                        _ => 0,
                    };
                }
                r
            };
            let pcx = peep::PeepCtx { bank, call_uses: &call_uses, ret_uses, is_isr: f.attrs.interrupt.is_some() };
            let has_asm_code = f.blocks.iter().any(|b| b.insts.iter().any(|x| matches!(x, Inst::Asm(_))));
            if opts.opt > 0 && !is_naked {
                peep::optimize(&mut items, &pcx, has_asm_code);
            }
        }
        // Clobbers: own + callees.
        let mut clob = code.clobbers;
        for &j in &callees[i] {
            clob |= total_clobbers[fids[j]];
        }
        let has_asm = f.blocks.iter().any(|b| b.insts.iter().any(|x| matches!(x, Inst::Asm(_))));
        if has_asm || has_indirect[i] {
            clob = ALL_REGS;
        }
        for b in &f.blocks {
            for ins in &b.insts {
                if let Inst::Call(_, Callee::Runtime(_), _) = ins {
                    clob = ALL_REGS;
                }
                if alloc::inst_clobbers(f, ins, &callee_for_alloc) != 0 {
                    clob |= alloc::inst_clobbers(f, ins, &callee_for_alloc);
                }
            }
        }
        for l in &ret_locs {
            if let Loc::R(r) = l {
                clob |= 1 << r;
            }
        }
        total_clobbers[fid] = clob;
        let mut ub = code.uses_b || has_asm || has_indirect[i] || is_naked;
        let mut ud = code.uses_dptr || has_asm || has_indirect[i] || is_naked;
        for &j in &callees[i] {
            ub |= total_uses_b[fids[j]];
            ud |= total_uses_dptr[fids[j]];
        }
        for b in &f.blocks {
            for ins in &b.insts {
                if let Inst::Call(_, Callee::Runtime(_), _) = ins {
                    ub = true;
                    ud = true;
                }
                if alloc::inst_clobbers(f, ins, &callee_for_alloc) == ALL_REGS {
                    ub = true;
                    ud = true;
                }
            }
        }
        total_uses_b[fid] = ub;
        total_uses_dptr[fid] = ud;
        // ISR prologue/epilogue.
        if let Some(_vec) = f.attrs.interrupt {
            expand_isr(&mut items, clob, ub, ud, f.attrs.using);
        }
        // Summary.
        if !is_naked {
            let params: Vec<Vec<Loc>> = if let Some((p, _)) = &fixed {
                p.clone()
            } else {
                f.params
                    .iter()
                    .map(|p| match p {
                        ParamLoc::Reg(r) => al.locs[*r as usize].clone(),
                        ParamLoc::Frame(_) => vec![],
                    })
                    .collect()
            };
            let clob_s = if recursive[i] { ALL_REGS } else { clob };
            summaries[fid] = Some(Summary { params, ret: ret_locs.clone(), clobbers: clob_s, keeps_b: !ub && !recursive[i], keeps_dptr: !ud && !recursive[i] });
        }
        // Frame request.
        let mut fr = FrameReq::default();
        for (oi, o) in f.frame.iter().enumerate() {
            if o.space == Space::Data || o.space == Space::Idata {
                fr.objects.push(RamObj { sym: frame_obj_sym(fid, oi as u32), size: o.size, upper: o.space == Space::Idata });
            }
        }
        if is_naked {
            for (pi, t) in f.param_tys.iter().enumerate().skip(1) {
                let _ = t;
                fr.objects.push(RamObj { sym: frame_obj_sym(fid, 1000 + pi as u32), size: t.bytes(), upper: false });
            }
        }
        let mut nslots = al.nslots;
        let mut nbits = al.nbits;
        if let Some((p, _)) = &fixed {
            for l in p.iter().flatten() {
                match l {
                    Loc::Slot(_, s) => nslots = nslots.max(s + 1),
                    Loc::BitSlot(_, s) => nbits = nbits.max(s + 1),
                    _ => {}
                }
            }
        }
        fr.slots = (0..nslots).map(|s| slot_sym(fid, s)).collect();
        fr.bits = (0..nbits).map(|s| bit_slot_sym(fid, s)).collect();
        frames[i] = fr;
        codes.push((fid, items));
    }

    // ---- RAM layout ----
    let mut global_bits = Vec::new();
    let mut ram_globals = Vec::new();
    let mut xdata_globals = Vec::new();
    let mut code_globals = Vec::new();
    let mut abs_syms: HashMap<Rc<str>, i64> = HashMap::new();
    let mut reach_g_sorted: Vec<usize> = reach_g.iter().copied().collect();
    reach_g_sorted.sort();
    for &g in &reach_g_sorted {
        let gl = &prog.globals[g];
        let name = gnames[g].clone();
        if let Some(a) = gl.at {
            abs_syms.insert(name.clone(), a as i64);
            if gl.init.is_some() && gl.space != Space::Code {
                crate::diag::warn(gl.loc, format!("initializer of '{}' at fixed address is ignored", gl.name));
            }
            continue;
        }
        match global_space(g) {
            Space::Sfr | Space::Sbit => {}
            Space::Bit => global_bits.push(name),
            Space::Data | Space::Idata => ram_globals.push(g),
            Space::Xdata | Space::Pdata => xdata_globals.push(g),
            Space::Code => code_globals.push(g),
        }
    }
    // Initialized globals first so that zero-initialized ones form a contiguous block to clear.
    ram_globals.sort_by_key(|g| prog.globals[*g].init.as_ref().map_or(1, |i| if i.bytes.iter().all(|b| *b == 0) && i.relocs.is_empty() { 1 } else { 0 }));
    let mut globals_req: Vec<RamObj> = ram_globals.iter().map(|&g| RamObj { sym: gnames[g].clone(), size: prog.size(&prog.globals[g].ty).max(1), upper: idata_globals.contains(&g) }).collect();
    // Runtime scratch RAM.
    let rt_mods = runtime_modules();
    let mut rt_needed: Vec<usize> = Vec::new();
    {
        // Implied helpers from IR.
        for &fid in &fids {
            let f = funcs[fid].as_ref().unwrap();
            for b in &f.blocks {
                for ins in &b.insts {
                    if let Inst::Bin(op, d, _, _) = ins {
                        let ty = f.ty(*d);
                        match op {
                            ir::BinK::Mul | ir::BinK::DivU | ir::BinK::DivS | ir::BinK::ModU | ir::BinK::ModS if ty != Ty::I8 => {
                                runtime_used.insert(sel::helper_name(*op, ty).to_string());
                            }
                            ir::BinK::DivS => {
                                runtime_used.insert("__divschar_ab".into());
                            }
                            ir::BinK::ModS => {
                                runtime_used.insert("__modschar_ab".into());
                            }
                            _ => {}
                        }
                    }
                    if let Inst::Load(_, Mem::Ptr(_, _, ir::PSpace::Generic)) = ins {
                        runtime_used.insert("__gptrget".into());
                    }
                    if let Inst::Store(Mem::Ptr(_, _, ir::PSpace::Generic), _, _) = ins {
                        runtime_used.insert("__gptrput".into());
                    }
                }
            }
        }
        // Also scan generated code for runtime symbol references.
        for (_, items) in &codes {
            for it in items {
                if let Item::Insn(i) = it {
                    if let Some(t) = i.target() {
                        if let Some(s) = &t.sym {
                            if s.starts_with("__") && !s.starts_with("__s") && !s.starts_with("__o") && !s.starts_with("__b") {
                                runtime_used.insert(s.to_string());
                            }
                        }
                    }
                }
            }
        }
        // Closure over module dependencies.
        let mut changed = true;
        while changed {
            changed = false;
            for (mi, m) in rt_mods.iter().enumerate() {
                if rt_needed.contains(&mi) {
                    continue;
                }
                if m.labels.iter().any(|l| runtime_used.contains(l)) {
                    rt_needed.push(mi);
                    changed = true;
                    // References from this module.
                    for w in m.text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                        if w.starts_with("__") {
                            runtime_used.insert(w.to_string());
                        }
                    }
                }
            }
        }
        let uses_rt_ram = rt_needed.iter().any(|&m| rt_mods[m].text.contains("__rt_t"));
        if uses_rt_ram {
            for k in 0..8 {
                globals_req.push(RamObj { sym: format!("__rt_t{}", k).into(), size: 1, upper: false });
            }
        }
    }
    let mut callers: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        for &j in &callees[i] {
            callers[j].push(i);
        }
    }
    let isr_roots: Vec<usize> = (0..n).filter(|&i| funcs[fids[i]].as_ref().unwrap().attrs.interrupt.is_some()).collect();
    let mut banks = 1u8;
    for &fid in &fids {
        if let Some(u) = funcs[fid].as_ref().unwrap().attrs.using {
            banks = banks.max(u + 1);
        }
    }
    let lay = match layout::layout(&LayoutInput { iram_size: opts.iram_size, banks, global_bits: global_bits.clone(), globals: globals_req.clone(), frames: frames.clone(), callers: callers.clone(), isr_roots }) {
        Ok(l) => l,
        Err(e) => {
            let mut msg = format!("{}\n  globals: {} bytes", e, globals_req.iter().map(|g| g.size).sum::<u32>());
            for (i, fr) in frames.iter().enumerate() {
                let sz: u32 = fr.objects.iter().map(|o| o.size).sum::<u32>() + fr.slots.len() as u32;
                if sz > 0 {
                    let cs: Vec<String> = callers[i].iter().map(|c| prog.funcs[fids[*c]].name.to_string()).collect();
                    msg.push_str(&format!("\n  frame of {}: {} bytes (called from {})", prog.funcs[fids[i]].name, sz, cs.join(", ")));
                }
            }
            return Err(msg);
        }
    };
    let mut syms = lay.syms.clone();
    for (k, v) in abs_syms {
        syms.insert(k, v);
    }
    // Xdata globals.
    let mut xptr = opts.xram_start;
    for &g in &xdata_globals {
        syms.insert(gnames[g].clone(), xptr as i64);
        xptr += prog.size(&prog.globals[g].ty).max(1);
    }
    if xptr > opts.xram_start + opts.xram_size {
        return Err("external RAM exhausted".into());
    }

    // ---- Startup code ----
    let mut sections: Vec<Section> = Vec::new();
    let main_fid = roots.iter().copied().find(|&f| &*prog.funcs[f].name == "main").unwrap();
    let mut vec_items = vec![Item::Insn(crate::asm::Insn::new(Mn::Jmp, vec![Op::label(&"__start".into())]))];
    sections.push(Section { name: "vector0".into(), items: std::mem::take(&mut vec_items), org: Some(opts.code_start) });
    let mut max_vec = 0u32;
    for &fid in &fids {
        if let Some(v) = funcs[fid].as_ref().unwrap().attrs.interrupt {
            if v == 255 {
                continue;
            }
            let addr = opts.code_start + 3 + 8 * v as u32;
            max_vec = max_vec.max(addr + 3);
            sections.push(Section {
                name: format!("vector{}", v).into(),
                items: vec![Item::Insn(crate::asm::Insn::new(Mn::Jmp, vec![Op::label(&fnames[fid])]))],
                org: Some(addr),
            });
        }
    }
    let mut start = vec![Item::Label("__start".into())];
    let stack_start = lay.ram_end;
    let sp_init = stack_start as i64 - 1;
    if sp_init != 7 {
        start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::dir(0x81), Op::imm(sp_init)])));
    }
    // Zero-initialize internal RAM globals (and bits).
    let needs_clear = ram_globals.iter().any(|g| prog.globals[*g].init.as_ref().map_or(true, |i| i.bytes.iter().any(|b| *b == 0))) || !global_bits.is_empty();
    if needs_clear {
        let mut end = lay.globals_end;
        if lay.upper_start < opts.iram_size.min(0x100) && idata_globals.iter().any(|g| prog.globals[*g].init.is_none()) {
            end = opts.iram_size.min(0x100);
        }
        if !global_bits.is_empty() {
            end = end.max(0x20 + ((global_bits.len() as u32 + 7) / 8));
        }
        if end > 1 {
            start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::R(0), Op::imm((end - 1) as i64)])));
            start.push(Item::Insn(crate::asm::Insn::new(Mn::Clr, vec![Op::A])));
            start.push(Item::Label("__clear_loop".into()));
            start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::AtR(0), Op::A])));
            start.push(Item::Insn(crate::asm::Insn::new(Mn::Djnz, vec![Op::R(0), Op::label(&"__clear_loop".into())])));
        }
    }
    // Initialized data.
    let mut init_moves = 0;
    for &g in &ram_globals {
        let gl = &prog.globals[g];
        let Some(init) = &gl.init else { continue };
        let reloc_at: HashMap<u32, &crate::ast::Reloc> = init.relocs.iter().map(|r| (r.offset, r)).collect();
        // (offset, value) pairs to write.
        let mut writes: Vec<(u32, Expr)> = Vec::new();
        let mut k = 0u32;
        while k < init.bytes.len() as u32 {
            if let Some(r) = reloc_at.get(&k) {
                let tname = match r.target {
                    RelocTarget::Global(x) => gnames[x].clone(),
                    RelocTarget::Func(x) => fnames[x].clone(),
                };
                let e = Expr::sym_off(&tname, r.addend);
                let parts: Vec<Expr> = match r.size {
                    1 => vec![e.lo()],
                    2 => vec![e.clone().lo(), e.hi()],
                    _ => {
                        let tag = match r.target {
                            RelocTarget::Global(x) => global_space(x).gptr_tag(),
                            RelocTarget::Func(_) => 0x80,
                        };
                        vec![e.clone().lo(), e.hi(), Expr::num(tag as i64)]
                    }
                };
                for (pi, p) in parts.into_iter().enumerate() {
                    writes.push((k + pi as u32, p));
                }
                k += r.size as u32;
                continue;
            }
            let b = init.bytes[k as usize];
            if b != 0 {
                writes.push((k, Expr::num(b as i64)));
            }
            k += 1;
        }
        let upper = idata_globals.contains(&g);
        let mut r0_at: Option<u32> = None;
        for (off, val) in writes {
            init_moves += 1;
            if upper {
                match r0_at {
                    Some(p) if p + 1 == off => start.push(Item::Insn(crate::asm::Insn::new(Mn::Inc, vec![Op::R(0)]))),
                    Some(p) if p == off => {}
                    _ => start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::R(0), Op::Imm(Expr::sym_off(&gnames[g], off as i64))]))),
                }
                r0_at = Some(off);
                start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::AtR(0), Op::Imm(val)])));
            } else {
                start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::Dir(Expr::sym_off(&gnames[g], off as i64)), Op::Imm(val)])));
            }
        }
    }
    let _ = init_moves;
    // Xdata init.
    for &g in &xdata_globals {
        let gl = &prog.globals[g];
        let size = prog.size(&gl.ty);
        let bytes = gl.init.as_ref().map(|i| i.bytes.clone()).unwrap_or_else(|| vec![0; size as usize]);
        start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::Dptr, Op::Imm(Expr::sym(&gnames[g]))])));
        for (k, b) in bytes.iter().enumerate() {
            if k > 0 {
                start.push(Item::Insn(crate::asm::Insn::new(Mn::Inc, vec![Op::Dptr])));
            }
            start.push(Item::Insn(crate::asm::Insn::new(Mn::Mov, vec![Op::A, Op::imm(*b as i64)])));
            start.push(Item::Insn(crate::asm::Insn::new(Mn::Movx, vec![Op::AtDptr, Op::A])));
        }
    }
    // Enter main. If main never returns, jump; else call and halt.
    let main_returns = funcs[main_fid].as_ref().unwrap().blocks.iter().any(|b| matches!(b.term, Term::Ret(_)));
    if main_returns {
        start.push(Item::Insn(crate::asm::Insn::new(Mn::Call, vec![Op::label(&fnames[main_fid])])));
        start.push(Item::Label("__halt".into()));
        start.push(Item::Insn(crate::asm::Insn::new(Mn::Jmp, vec![Op::label(&"__halt".into())])));
    } else {
        start.push(Item::Insn(crate::asm::Insn::new(Mn::Jmp, vec![Op::label(&fnames[main_fid])])));
    }
    let code_origin = (opts.code_start + 3).max(max_vec);
    sections.push(Section { name: "startup".into(), items: start, org: Some(code_origin) });
    let nfixed = sections.len();
    // Functions: main first, then the rest (the order is optimized below).
    let mut order_codes: Vec<(usize, Vec<Item>)> = codes;
    order_codes.sort_by_key(|(fid, _)| if *fid == main_fid { 0 } else { 1 });
    let mut outline_ok: Vec<bool> = vec![false; nfixed];
    for (fid, items) in order_codes {
        let f = funcs[fid].as_ref().unwrap();
        let has_asm = f.blocks.iter().any(|b| b.insts.iter().any(|x| matches!(x, Inst::Asm(_))));
        outline_ok.push(!has_asm && !f.attrs.naked);
        sections.push(Section { name: fnames[fid].clone(), items, org: None });
    }
    // Runtime modules.
    for &mi in &rt_needed {
        let resolve = |_: &str| -> Option<SymRes> { None };
        let mut p = AsmParser::new(&resolve, 0);
        p.label_prefix = format!("__rt{}", mi);
        let items = p.parse(&rt_mods[mi].text).map_err(|e| format!("runtime: {}", e))?;
        sections.push(Section { name: format!("runtime{}", mi).into(), items, org: None });
    }
    // Procedural abstraction over functions and runtime code.
    outline_ok.resize(sections.len(), false);
    if opts.opt > 0 {
        let mut globals: HashSet<Rc<str>> = fnames.iter().cloned().collect();
        for m in &rt_mods {
            for l in &m.labels {
                globals.insert(l.as_str().into());
            }
        }
        cg_outline(&mut sections, &outline_ok, &globals);
    }
    // Constant data in code space.
    for &g in &code_globals {
        let gl = &prog.globals[g];
        let size = prog.size(&gl.ty);
        let mut items = vec![Item::Label(gnames[g].clone())];
        let init = gl.init.clone().unwrap_or_default();
        let mut bytes: Vec<Expr> = (0..size as usize).map(|k| Expr::num(*init.bytes.get(k).unwrap_or(&0) as i64)).collect();
        for r in &init.relocs {
            let tname = match r.target {
                RelocTarget::Global(x) => gnames[x].clone(),
                RelocTarget::Func(x) => fnames[x].clone(),
            };
            let e = Expr::sym_off(&tname, r.addend);
            let o = r.offset as usize;
            bytes[o] = e.clone().lo();
            if r.size >= 2 {
                bytes[o + 1] = e.clone().hi();
            }
            if r.size >= 3 {
                let tag = match r.target {
                    RelocTarget::Global(x) => prog.globals[x].space.gptr_tag(),
                    RelocTarget::Func(_) => 0x80,
                };
                bytes[o + 2] = Expr::num(tag as i64);
            }
        }
        items.push(Item::Db(bytes));
        sections.push(Section { name: gnames[g].clone(), items, org: None });
    }
    let nmovable = sections.len() - nfixed;
    let code_end = opts.code_start + opts.code_size;
    // Order movable sections (functions and runtime) to maximize short calls and jumps.
    let sections = if opts.opt > 0 && nmovable > 1 {
        order_sections(sections, nfixed, nmovable, &syms, code_origin)
    } else {
        sections
    };
    let lk = link::link(sections, syms, code_origin, code_end)?;
    let ranges: Vec<(u32, u32)> = lk.sections.iter().map(|(_, a, s)| (*a, *a + *s)).collect();
    let hex = link::intel_hex(&lk.image, &ranges);
    let code_size: u32 = lk.sections.iter().map(|s| s.2).sum();
    let mut map = String::new();
    use std::fmt::Write as _;
    let _ = writeln!(map, "Code sections:");
    let mut secs = lk.sections.clone();
    secs.sort_by_key(|s| s.1);
    for (name, a, s) in &secs {
        if *s > 0 {
            let _ = writeln!(map, "  {:#06x} {:5} {}", a, s, name);
        }
    }
    let _ = writeln!(map, "Total code: {} bytes", code_size);
    let _ = writeln!(map, "Internal RAM: data/frames end at {:#04x}, {} bit bytes; stack starts at {:#04x}", lay.ram_end, lay.bit_bytes, stack_start);
    let mut rs: Vec<(&Rc<str>, &i64)> = lay.syms.iter().collect();
    rs.sort_by_key(|x| (*x.1, x.0.clone()));
    let _ = writeln!(map, "RAM symbols:");
    for (k, v) in rs {
        let _ = writeln!(map, "  {:#04x} {}", v, k);
    }
    Ok(Output { image: lk.image, hex, listing: lk.listing, map, code_size, ram_used: lay.ram_end })
}

fn cg_outline(sections: &mut Vec<Section>, ok: &[bool], globals: &HashSet<Rc<str>>) {
    super::outline::run(sections, ok, globals);
}

/// Search for a section order that minimizes code size.
fn order_sections(sections: Vec<Section>, nfixed: usize, nmovable: usize, syms: &HashMap<Rc<str>, i64>, origin: u32) -> Vec<Section> {
    let total = sections.len();
    let fixed: Vec<usize> = (0..nfixed).collect();
    let tail: Vec<usize> = (nfixed + nmovable..total).collect();
    let movable: Vec<usize> = (nfixed..nfixed + nmovable).collect();
    let eval = |m: &[usize]| -> u32 {
        let order: Vec<usize> = fixed.iter().chain(m.iter()).chain(tail.iter()).copied().collect();
        link::measure(&sections, &order, syms, origin).unwrap_or(u32::MAX)
    };
    // Call counts per section label.
    let mut calls: HashMap<Rc<str>, u32> = HashMap::new();
    for s in &sections {
        for it in &s.items {
            if let Item::Insn(i) = it {
                if matches!(i.mn, Mn::Call | Mn::Jmp) {
                    if let Some(t) = i.target().and_then(|t| t.sym.clone()) {
                        *calls.entry(t).or_default() += 1;
                    }
                }
            }
        }
    }
    let size_of = |i: usize| -> u32 { sections[i].items.iter().map(|it| if let Item::Insn(x) = it { crate::asm::insn_size(x, crate::asm::Reach::Short) } else { 0 }).sum() };
    let heat = |i: usize| -> u32 {
        let first_label = sections[i].items.iter().find_map(|it| if let Item::Label(l) = it { Some(l.clone()) } else { None });
        first_label.and_then(|l| calls.get(&l).copied()).unwrap_or(0)
    };
    let mut candidates: Vec<Vec<usize>> = Vec::new();
    candidates.push(movable.clone());
    // Hot small callees first.
    let mut hot = movable.clone();
    hot.sort_by_key(|&i| std::cmp::Reverse((heat(i) * 64) / (size_of(i) + 8)));
    candidates.push(hot.clone());
    // Callees around the biggest function.
    let mut big = hot.clone();
    if let Some(pos) = big.iter().position(|&i| i == movable[0]) {
        let m = big.remove(pos);
        let half = big.len() / 2;
        big.insert(half, m);
    }
    candidates.push(big);
    let mut best = movable.clone();
    let mut best_size = u32::MAX;
    for c in candidates {
        let sz = eval(&c);
        if sz < best_size {
            best_size = sz;
            best = c;
        }
    }
    // Hill climbing with a deterministic PRNG.
    let mut seed: u64 = 0x9e3779b97f4a7c15;
    let mut rnd = |n: usize| -> usize {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % n as u64) as usize
    };
    let iters = (nmovable * 6).min(400);
    for _ in 0..iters {
        let i = rnd(nmovable);
        let j = rnd(nmovable);
        if i == j {
            continue;
        }
        let mut cand = best.clone();
        if rnd(2) == 0 {
            cand.swap(i, j);
        } else {
            let x = cand.remove(i);
            cand.insert(j, x);
        }
        let sz = eval(&cand);
        if sz < best_size {
            best_size = sz;
            best = cand;
        }
    }
    let order: Vec<usize> = fixed.iter().chain(best.iter()).chain(tail.iter()).copied().collect();
    let mut slots: Vec<Option<Section>> = sections.into_iter().map(Some).collect();
    order.into_iter().map(|i| slots[i].take().unwrap()).collect()
}

/// Bottom-up (callee-first) order of the reachable functions.
fn bottom_up(funcs: &[Option<Func>], reach: &HashSet<usize>) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    fn visit(f: usize, funcs: &[Option<Func>], seen: &mut HashSet<usize>, out: &mut Vec<usize>) {
        if !seen.insert(f) {
            return;
        }
        if let Some(func) = &funcs[f] {
            for b in &func.blocks {
                for i in &b.insts {
                    if let Inst::Call(_, Callee::Direct(c), _) = i {
                        visit(*c, funcs, seen, out);
                    }
                }
            }
        }
        out.push(f);
    }
    let mut r: Vec<usize> = reach.iter().copied().collect();
    r.sort();
    for f in r {
        visit(f, funcs, &mut seen, &mut out);
    }
    out
}

fn glob_res(prog: &Program, g: usize, gnames: &[Rc<str>]) -> SymRes {
    let gl = &prog.globals[g];
    match (gl.space, gl.at) {
        (Space::Sfr, Some(a)) => SymRes::Const(a as i64),
        (Space::Sbit, Some(a)) => SymRes::Bit(a as i64),
        _ => SymRes::Sym(gnames[g].clone()),
    }
}

/// Replace uses of register parameters with copies made at function entry.
fn split_params(f: &mut Func) {
    let params: Vec<ir::VReg> = f.params.iter().filter_map(|p| if let ParamLoc::Reg(r) = p { Some(*r) } else { None }).collect();
    let mut copies = Vec::new();
    for p in params {
        let ty = f.ty(p);
        let n = f.new_vreg(ty);
        f.vregs[n as usize].name = f.vregs[p as usize].name.clone();
        for b in f.blocks.iter_mut() {
            for ins in b.insts.iter_mut() {
                if ins.def() == Some(p) {
                    ins.set_def(n);
                }
                ins.for_each_val_mut(|v| {
                    if *v == Val::R(p) {
                        *v = Val::R(n);
                    }
                });
                if let Inst::CritExit(r) = ins {
                    if *r == p {
                        *r = n;
                    }
                }
            }
            b.term.for_each_val_mut(|v| {
                if *v == Val::R(p) {
                    *v = Val::R(n);
                }
            });
        }
        copies.push(Inst::Copy(n, Val::R(p)));
    }
    let entry = &mut f.blocks[0].insts;
    for (k, c) in copies.into_iter().enumerate() {
        entry.insert(k, c);
    }
}

/// Indirect calls: nothing to rewrite in the IR (summaries are resolved in codegen by argument types).
fn rewrite_indirect(f: &Func, _s: &dyn Fn(&[Val], Option<Ty>) -> Summary) -> Func {
    f.clone()
}

fn expand_isr(items: &mut Vec<Item>, clob: RegSet, uses_b: bool, uses_dptr: bool, using: Option<u8>) {
    use crate::asm::Insn;
    let mut pro = Vec::new();
    let mut saves: Vec<i64> = vec![0xE0];
    if uses_b {
        saves.push(0xF0);
    }
    if uses_dptr {
        saves.push(0x82);
        saves.push(0x83);
    }
    saves.push(0xD0);
    for s in &saves {
        pro.push(Item::Insn(Insn::new(Mn::Push, vec![Op::dir(*s)])));
    }
    if let Some(b) = using {
        pro.push(Item::Insn(Insn::new(Mn::Mov, vec![Op::dir(0xD0), Op::imm((b as i64) << 3)])));
    } else {
        for r in 0..8 {
            if clob & (1 << r) != 0 {
                pro.push(Item::Insn(Insn::new(Mn::Push, vec![Op::dir(r)])));
            }
        }
    }
    let mut epi = Vec::new();
    for it in pro.iter().rev() {
        if let Item::Insn(i) = it {
            if i.mn == Mn::Push {
                epi.push(Item::Insn(Insn::new(Mn::Pop, i.ops.clone())));
            }
        }
    }
    let mut out = Vec::new();
    for it in items.drain(..) {
        match &it {
            Item::Comment(c) if c == "@isr_prologue" => out.extend(pro.iter().cloned()),
            Item::Comment(c) if c == "@isr_epilogue" => out.extend(epi.iter().cloned()),
            _ => out.push(it),
        }
    }
    *items = out;
}
