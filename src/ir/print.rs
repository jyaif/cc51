//! Textual dump of the IR (for debugging).

use super::*;
use std::fmt::Write;

pub fn val(f: &Func, v: &Val) -> String {
    match v {
        Val::R(r) => reg(f, *r),
        Val::K(k) => format!("#{}", k),
        Val::Addr(s, o) => format!("&{}{:+}", sym(s), o),
    }
}

pub fn reg(f: &Func, r: VReg) -> String {
    match f.vregs.get(r as usize).and_then(|v| v.name.as_ref()) {
        Some(n) => format!("%{}.{}", r, n),
        None => format!("%{}", r),
    }
}

pub fn sym(s: &Sym) -> String {
    match s {
        Sym::Global(g) => format!("g{}", g),
        Sym::Func(fid) => format!("f{}", fid),
        Sym::Frame(fid, o) => format!("f{}.frame{}", fid, o),
        Sym::Named(n) => n.to_string(),
    }
}

pub fn mem(f: &Func, m: &Mem) -> String {
    match m {
        Mem::Sym(s, o) => format!("[{}{:+}]", sym(s), o),
        Mem::Abs(sp, a) => format!("[{:?}:{:#x}]", sp, a),
        Mem::Ptr(v, o, sp) => format!("[{:?} {}{:+}]", sp, val(f, v), o),
    }
}

pub fn inst(f: &Func, i: &Inst) -> String {
    let t = |r: VReg| format!("{:?}", f.ty(r));
    match i {
        Inst::Copy(d, a) => format!("{} = {} {}", reg(f, *d), t(*d), val(f, a)),
        Inst::Bin(k, d, a, b) => format!("{} = {} {} {}, {}", reg(f, *d), k.name(), t(*d), val(f, a), val(f, b)),
        Inst::Un(k, d, a) => format!("{} = {:?} {} {}", reg(f, *d), k, t(*d), val(f, a)),
        Inst::Cmp(c, d, a, b, ty) => format!("{} = cmp.{} {:?} {}, {}", reg(f, *d), c.name(), ty, val(f, a), val(f, b)),
        Inst::Ext(d, a, s) => format!("{} = {} {} {}", reg(f, *d), if *s { "sext" } else { "zext" }, t(*d), val(f, a)),
        Inst::Trunc(d, a) => format!("{} = trunc {} {}", reg(f, *d), t(*d), val(f, a)),
        Inst::Load(d, m) => format!("{} = load {} {}", reg(f, *d), t(*d), mem(f, m)),
        Inst::Store(m, v, ty) => format!("store {:?} {} <- {}", ty, mem(f, m), val(f, v)),
        Inst::Call(d, c, args) => {
            let mut s = String::new();
            if let Some(d) = d {
                write!(s, "{} = ", reg(f, *d)).unwrap();
            }
            match c {
                Callee::Direct(fid) => write!(s, "call f{}", fid).unwrap(),
                Callee::Indirect(v, _) => write!(s, "call *{}", val(f, v)).unwrap(),
                Callee::Runtime(n) => write!(s, "call {}", n).unwrap(),
            }
            let a: Vec<String> = args.iter().map(|a| val(f, a)).collect();
            write!(s, "({})", a.join(", ")).unwrap();
            s
        }
        Inst::Asm(a) => format!("asm {:?}", a),
        Inst::CritEnter(v) => format!("{} = critical_enter", reg(f, *v)),
        Inst::CritExit(v) => format!("critical_exit {}", reg(f, *v)),
        Inst::MemCopy(d, s, n) => format!("memcopy {} <- {} x{}", mem(f, d), mem(f, s), n),
        Inst::MemSet(d, v, n) => format!("memset {} <- {} x{}", mem(f, d), val(f, v), n),
        Inst::Nop => "nop".into(),
    }
}

pub fn term(f: &Func, t: &Term) -> String {
    match t {
        Term::Jmp(b) => format!("jmp b{}", b),
        Term::Br(v, a, b) => format!("br {} ? b{} : b{}", val(f, v), a, b),
        Term::CmpBr(c, x, y, ty, a, b) => format!("br {} {:?} {}, {} ? b{} : b{}", c.name(), ty, val(f, x), val(f, y), a, b),
        Term::Switch(v, ty, cases, d) => {
            let cs: Vec<String> = cases.iter().map(|(k, b)| format!("{}: b{}", k, b)).collect();
            format!("switch {:?} {} [{}] default b{}", ty, val(f, v), cs.join(", "), d)
        }
        Term::Ret(Some(v)) => format!("ret {}", val(f, v)),
        Term::Ret(None) => "ret".into(),
        Term::Unreachable => "unreachable".into(),
    }
}

pub fn func(f: &Func) -> String {
    let mut s = String::new();
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| match p {
            ParamLoc::Reg(r) => format!("{} {:?}", reg(f, *r), f.ty(*r)),
            ParamLoc::Frame(o) => format!("frame{}", o),
        })
        .collect();
    writeln!(s, "func f{} {}({}) -> {:?}", f.id, f.name, params.join(", "), f.ret).unwrap();
    for (i, o) in f.frame.iter().enumerate() {
        writeln!(s, "  frame{}: {} {} bytes {:?}", i, o.name, o.size, o.space).unwrap();
    }
    let preds = f.preds();
    for (i, b) in f.blocks.iter().enumerate() {
        let p: Vec<String> = preds[i].iter().map(|p| format!("b{}", p)).collect();
        writeln!(s, "b{}:  ; preds {}", i, p.join(" ")).unwrap();
        for ins in &b.insts {
            writeln!(s, "    {}", inst(f, ins)).unwrap();
        }
        writeln!(s, "    {}", term(f, &b.term)).unwrap();
    }
    s
}
