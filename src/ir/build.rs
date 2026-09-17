//! Lowering of the typed AST to IR.

use super::*;
use crate::ast::{BinOp, Builtin, ExprKind, Expr, FuncId, Function, GlobalId, LabelId, Linkage, LocalId, LocalInit, Program, Stmt, UnOp};
use crate::diag::{Loc, Result, err};
use crate::types::{IntKind, Type, TypeKind};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug)]
enum Storage {
    Reg(VReg),
    Frame(u32),
}

#[derive(Clone, Debug)]
enum LVal {
    Reg(VReg),
    Mem(Mem, Ty),
    /// Bit-field inside memory: (mem of first storage byte, bit offset, width, value type, signed)
    Bits(Mem, u8, u8, Ty, bool),
    /// 16/32-bit SFR composed of separate byte addresses (little-endian order).
    SfrMulti(Vec<u32>),
}

pub fn ir_ty(prog: &Program, t: &Type) -> Ty {
    match &t.kind {
        TypeKind::Int(k, _) | TypeKind::Enum(_, k, _) => match k {
            IntKind::Bit => Ty::Bit,
            IntKind::Bool | IntKind::Char => Ty::I8,
            IntKind::Short | IntKind::Int => Ty::I16,
            IntKind::Long => Ty::I32,
            IntKind::LongLong => Ty::I64,
        },
        TypeKind::Float | TypeKind::Double => Ty::I32,
        TypeKind::Pointer(..) => {
            if prog.size(t) == 3 {
                Ty::I24
            } else {
                Ty::I16
            }
        }
        _ => Ty::I16,
    }
}

pub fn ptr_pspace(prog: &Program, t: &Type) -> PSpace {
    match prog.ptr_space(t) {
        Some(s) => PSpace::S(s),
        None => PSpace::Generic,
    }
}

fn mem_add(m: Mem, k: i32) -> Mem {
    match m {
        Mem::Sym(s, o) => Mem::Sym(s, o + k),
        Mem::Abs(sp, a) => Mem::Abs(sp, (a as i64 + k as i64) as u32),
        Mem::Ptr(v, o, sp) => Mem::Ptr(v, o + k, sp),
    }
}

/// Frame object index used for a parameter that is received in memory.
pub fn param_in_memory(_prog: &Program, f: &Function, i: usize) -> bool {
    let lid = f.params[i];
    !f.locals[lid].ty.is_scalar()
}

pub fn param_frame_index(prog: &Program, f: &Function, i: usize) -> Option<u32> {
    if !param_in_memory(prog, f, i) {
        return None;
    }
    Some((0..i).filter(|&j| param_in_memory(prog, f, j)).count() as u32)
}

pub struct Builder<'a> {
    prog: &'a Program,
    fid: FuncId,
    pub f: Func,
    cur: BlockId,
    locals: HashMap<LocalId, Storage>,
    labels: HashMap<LabelId, BlockId>,
    breaks: Vec<BlockId>,
    continues: Vec<BlockId>,
    crit: Vec<VReg>,
    ret_type: Type,
    sealed: bool,
}

thread_local! {
    static VARARG_SIZES: std::cell::RefCell<Vec<u32>> = Default::default();
}

pub fn set_vararg_sizes(v: Vec<u32>) {
    VARARG_SIZES.with(|s| *s.borrow_mut() = v);
}

fn vararg_size(fid: FuncId) -> u32 {
    VARARG_SIZES.with(|s| s.borrow().get(fid).copied().unwrap_or(0))
}

/// Frame object index of the variable-argument area of a variadic function.
pub fn varargs_obj_index(prog: &Program, fid: FuncId) -> u32 {
    let f = &prog.funcs[fid];
    let mem_params = (0..f.params.len()).filter(|&j| param_in_memory(prog, f, j)).count() as u32;
    mem_params + if f.ftype().ret.is_record() { 1 } else { 0 }
}

pub fn build_func(prog: &Program, fid: FuncId) -> Result<Func> {
    let af = &prog.funcs[fid];
    let ft = af.ftype().clone();
    let ret = if ft.ret.is_void() || ft.ret.is_record() { None } else { Some(ir_ty(prog, &ft.ret)) };
    let f = Func {
        id: fid,
        name: af.name.clone(),
        vregs: Vec::new(),
        blocks: Vec::new(),
        params: Vec::new(),
        param_tys: Vec::new(),
        ret,
        ret_obj: None,
        frame: Vec::new(),
        attrs: ft.attrs.clone(),
        is_static: af.linkage == Linkage::Internal,
        addr_taken: af.addr_taken,
        nooverlay: af.nooverlay,
        variadic: ft.variadic,
    };
    let mut b = Builder {
        prog,
        fid,
        f,
        cur: 0,
        locals: HashMap::new(),
        labels: HashMap::new(),
        breaks: Vec::new(),
        continues: Vec::new(),
        crit: Vec::new(),
        ret_type: ft.ret.clone(),
        sealed: false,
    };
    b.f.new_block();
    // Parameters.
    for (i, &lid) in af.params.iter().enumerate() {
        let l = &af.locals[lid];
        let ty = if l.ty.is_scalar() { ir_ty(prog, &l.ty) } else { Ty::I16 };
        b.f.param_tys.push(ty);
        if param_in_memory(prog, af, i) {
            let size = prog.size(&l.ty);
            let idx = b.f.frame.len() as u32;
            b.f.frame.push(FrameObj { size, space: Space::Data, name: l.name.clone(), param: Some(i) });
            b.locals.insert(lid, Storage::Frame(idx));
            b.f.params.push(ParamLoc::Frame(idx));
        } else {
            let v = b.f.new_vreg(ty);
            b.f.vregs[v as usize].name = Some(l.name.clone());
            b.f.params.push(ParamLoc::Reg(v));
            if l.addr_taken {
                // Received in a register, then kept in memory.
                let idx = b.f.frame.len() as u32;
                b.f.frame.push(FrameObj { size: prog.size(&l.ty), space: Space::Data, name: l.name.clone(), param: None });
                b.locals.insert(lid, Storage::Frame(idx));
                let sty = if ty == Ty::Bit { Ty::I8 } else { ty };
                let sv = if ty == Ty::Bit { b.resize(Val::R(v), Ty::Bit, Ty::I8, false) } else { Val::R(v) };
                b.emit(Inst::Store(Mem::Sym(Sym::Frame(b.fid, idx), 0), sv, sty));
            } else {
                b.locals.insert(lid, Storage::Reg(v));
            }
        }
    }
    if ft.ret.is_record() {
        let size = prog.size(&ft.ret);
        let idx = b.f.frame.len() as u32;
        b.f.frame.push(FrameObj { size, space: Space::Data, name: "__retval".into(), param: None });
        b.f.ret_obj = Some(idx);
    }
    if ft.variadic {
        let idx = b.f.frame.len() as u32;
        debug_assert_eq!(idx, varargs_obj_index(prog, fid));
        b.f.frame.push(FrameObj { size: vararg_size(fid).max(1), space: Space::Data, name: "__varargs".into(), param: None });
    }
    let body = af.body.as_ref().unwrap();
    if ft.attrs.critical {
        let v = b.f.new_vreg(Ty::Bit);
        b.emit(Inst::CritEnter(v));
        b.crit.push(v);
    }
    b.stmt(body)?;
    if !b.sealed {
        for &c in b.crit.clone().iter().rev() {
            b.emit(Inst::CritExit(c));
        }
        let t = if b.f.ret.is_some() {
            // Falling off the end of a non-void function: return an undefined value (0).
            if &*af.name == "main" { Term::Ret(Some(Val::K(0))) } else { Term::Ret(Some(Val::K(0))) }
        } else {
            Term::Ret(None)
        };
        b.terminate(t);
    }
    Ok(b.f)
}

impl<'a> Builder<'a> {
    fn emit(&mut self, i: Inst) {
        if self.sealed {
            // Unreachable code after a terminator: start a fresh (unreachable) block.
            let nb = self.f.new_block();
            self.cur = nb;
            self.sealed = false;
        }
        self.f.blocks[self.cur as usize].insts.push(i);
    }
    fn terminate(&mut self, t: Term) {
        if self.sealed {
            let nb = self.f.new_block();
            self.cur = nb;
        }
        self.f.blocks[self.cur as usize].term = t;
        self.sealed = true;
    }
    fn start(&mut self, b: BlockId) {
        if !self.sealed {
            self.f.blocks[self.cur as usize].term = Term::Jmp(b);
        }
        self.cur = b;
        self.sealed = false;
    }
    /// Callee for a library routine implemented in C (soft float).
    fn lib_callee(&self, name: &'static str) -> Callee {
        match self.prog.externs.get(name) {
            Some(crate::ast::Sym::Func(fid)) if self.prog.funcs[*fid].body.is_some() => Callee::Direct(*fid),
            _ => Callee::Runtime(name),
        }
    }

    fn tmp(&mut self, ty: Ty) -> VReg {
        self.f.new_vreg(ty)
    }
    fn ty(&self, t: &Type) -> Ty {
        ir_ty(self.prog, t)
    }
    fn val_ty(&self, v: Val, default: Ty) -> Ty {
        match v {
            Val::R(r) => self.f.ty(r),
            _ => default,
        }
    }

    fn local_storage(&mut self, lid: LocalId) -> Storage {
        if let Some(s) = self.locals.get(&lid) {
            return *s;
        }
        let l = &self.prog.funcs[self.fid].locals[lid];
        let s = if l.ty.is_scalar() && !l.addr_taken && l.space.is_none() {
            let v = self.f.new_vreg(ir_ty(self.prog, &l.ty));
            self.f.vregs[v as usize].name = Some(l.name.clone());
            Storage::Reg(v)
        } else {
            let size = self.prog.size(&l.ty);
            let idx = self.f.frame.len() as u32;
            let space = match l.space {
                Some(Space::Idata) => Space::Idata,
                Some(Space::Xdata) => Space::Xdata,
                _ => Space::Data,
            };
            self.f.frame.push(FrameObj { size, space, name: l.name.clone(), param: None });
            Storage::Frame(idx)
        };
        self.locals.insert(lid, s);
        s
    }

    fn global_mem(&self, g: GlobalId) -> LVal {
        let gl = &self.prog.globals[g];
        let size = if gl.ty.is_bit() { 0 } else { self.prog.size(&gl.ty) };
        match gl.space {
            Space::Sfr => {
                let at = gl.at.unwrap_or(0);
                if size <= 1 {
                    LVal::Mem(Mem::Abs(Space::Sfr, at), Ty::I8)
                } else {
                    // SDCC: __sfr16 __at(0xHHLL): little-endian byte addresses packed in the value.
                    let addrs = (0..size).map(|i| (at >> (8 * i)) & 0xff).collect();
                    LVal::SfrMulti(addrs)
                }
            }
            Space::Sbit => LVal::Mem(Mem::Abs(Space::Sbit, gl.at.unwrap_or(0)), Ty::Bit),
            Space::Bit => match gl.at {
                Some(a) => LVal::Mem(Mem::Abs(Space::Sbit, a), Ty::Bit),
                None => LVal::Mem(Mem::Sym(Sym::Global(g), 0), Ty::Bit),
            },
            sp => {
                let ty = if gl.ty.is_scalar() { self.ty(&gl.ty) } else { Ty::I8 };
                match gl.at {
                    Some(a) => LVal::Mem(Mem::Abs(sp, a), ty),
                    None => LVal::Mem(Mem::Sym(Sym::Global(g), 0), ty),
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Lvalues

    fn lvalue(&mut self, e: &Expr) -> Result<LVal> {
        let ety = if e.ty.is_scalar() { self.ty(&e.ty) } else { Ty::I8 };
        match &e.kind {
            ExprKind::Local(l) => Ok(match self.local_storage(*l) {
                Storage::Reg(v) => LVal::Reg(v),
                Storage::Frame(o) => {
                    let sp = self.f.frame[o as usize].space;
                    let _ = sp;
                    LVal::Mem(Mem::Sym(Sym::Frame(self.fid, o), 0), ety)
                }
            }),
            ExprKind::Global(g) => Ok(match self.global_mem(*g) {
                LVal::Mem(m, _) if !e.ty.is_bit() && !matches!(self.prog.globals[*g].space, Space::Sfr) => LVal::Mem(m, ety),
                other => other,
            }),
            ExprKind::Deref(p) => {
                let m = self.ptr_mem(p)?;
                if e.ty.is_bit() {
                    return Ok(LVal::Mem(m, Ty::Bit));
                }
                Ok(LVal::Mem(m, ety))
            }
            ExprKind::Member(b, fi) => {
                let TypeKind::Record(rid) = b.ty.kind else { return err(e.loc, "member of non-record") };
                let f = &self.prog.records[rid].fields[*fi];
                let (off, bits) = (f.offset, f.bits);
                let signed = f.ty.is_signed() && !f.ty.is_bool();
                let vty = self.ty(&f.ty);
                let base = self.lvalue(b)?;
                let m = match base {
                    LVal::Mem(m, _) => m,
                    _ => return err(e.loc, "member access on non-memory object"),
                };
                let m = mem_add(m, off as i32);
                if let Some((bo, w)) = bits {
                    return Ok(LVal::Bits(m, bo, w, vty, signed));
                }
                Ok(LVal::Mem(m, ety))
            }
            ExprKind::StmtExpr(stmts, Some(v)) => {
                for s in stmts {
                    self.stmt(s)?;
                }
                self.lvalue(v)
            }
            ExprKind::Call(..) if e.ty.is_record() => {
                // Aggregate returned by a call: lives in the callee's return object.
                self.call(e)?;
                let ExprKind::Call(callee, _) = &e.kind else { unreachable!() };
                match &callee.kind {
                    ExprKind::Func(fid) => {
                        let obj = self.ret_obj_index(*fid);
                        Ok(LVal::Mem(Mem::Sym(Sym::Frame(*fid, obj), 0), Ty::I8))
                    }
                    _ => err(e.loc, "indirect call returning a struct is not supported"),
                }
            }
            ExprKind::Assign(l, r) if e.ty.is_record() => {
                let size = self.prog.size(&e.ty);
                let src = self.agg_mem(r)?;
                let dst = self.agg_mem(l)?;
                self.emit(Inst::MemCopy(dst, src, size));
                Ok(LVal::Mem(dst, Ty::I8))
            }
            ExprKind::Cond(c, a, b) if e.ty.is_record() => {
                // Copy the selected aggregate into a temporary frame object.
                let size = self.prog.size(&e.ty);
                let idx = self.f.frame.len() as u32;
                self.f.frame.push(FrameObj { size, space: Space::Data, name: "__condtmp".into(), param: None });
                let dst = Mem::Sym(Sym::Frame(self.fid, idx), 0);
                let (tb, fb, join) = (self.f.new_block(), self.f.new_block(), self.f.new_block());
                self.cond_branch(c, tb, fb)?;
                self.start(tb);
                let am = self.agg_mem(a)?;
                self.emit(Inst::MemCopy(dst, am, size));
                self.terminate(Term::Jmp(join));
                self.start(fb);
                let bm = self.agg_mem(b)?;
                self.emit(Inst::MemCopy(dst, bm, size));
                self.start(join);
                Ok(LVal::Mem(dst, Ty::I8))
            }
            ExprKind::Comma(a, b) => {
                self.effect(a)?;
                self.lvalue(b)
            }
            _ => err(e.loc, "expression is not an lvalue"),
        }
    }

    fn ret_obj_index(&self, fid: FuncId) -> u32 {
        let f = &self.prog.funcs[fid];
        (0..f.params.len()).filter(|&j| param_in_memory(self.prog, f, j)).count() as u32
    }

    fn agg_mem(&mut self, e: &Expr) -> Result<Mem> {
        match self.lvalue(e)? {
            LVal::Mem(m, _) => Ok(m),
            _ => err(e.loc, "aggregate is not in memory"),
        }
    }

    /// Memory designated by dereferencing pointer expression `p`.
    fn ptr_mem(&mut self, p: &Expr) -> Result<Mem> {
        let sp = ptr_pspace(self.prog, &p.ty);
        // Fold constant offsets.
        if let ExprKind::PtrAdd(base, idx) = &p.kind {
            if let Some(k) = idx.as_int() {
                let m = self.ptr_mem(base)?;
                return Ok(mem_add(m, k as i32));
            }
        }
        if let ExprKind::AddrOf(inner) = &p.kind {
            if let Ok(LVal::Mem(m, _)) = self.lvalue(inner) {
                return Ok(m);
            }
        }
        let v = self.rvalue(p)?;
        Ok(match v {
            Val::Addr(s, o) => Mem::Sym(s, o),
            Val::K(k) => match sp {
                PSpace::S(s) => Mem::Abs(s, (k & 0xffff) as u32),
                PSpace::Generic => {
                    let tag = (k >> 16) & 0xff;
                    let s = match tag {
                        0x00 => Space::Xdata,
                        0x60 => Space::Pdata,
                        0x80 => Space::Code,
                        _ => Space::Data,
                    };
                    Mem::Abs(s, (k & 0xffff) as u32)
                }
            },
            v => Mem::Ptr(v, 0, sp),
        })
    }

    fn load(&mut self, lv: &LVal) -> Val {
        match lv {
            LVal::Reg(v) => Val::R(*v),
            LVal::Mem(m, ty) => {
                let t = self.tmp(*ty);
                self.emit(Inst::Load(t, *m));
                Val::R(t)
            }
            LVal::Bits(m, bo, w, vty, signed) => {
                let nbytes = ((*bo as u32 + *w as u32) + 7) / 8;
                let sty = Ty::from_bytes(if nbytes <= 1 { 1 } else if nbytes <= 2 { 2 } else { 4 });
                let raw = self.load_bytes(*m, nbytes, sty);
                let mut v = raw;
                if *bo > 0 {
                    let t = self.tmp(sty);
                    self.emit(Inst::Bin(BinK::ShrU, t, v, Val::K(*bo as i64)));
                    v = Val::R(t);
                }
                let mask = if *w as u32 >= sty.bits() { sty.mask() as i64 } else { (1i64 << *w) - 1 };
                let t = self.tmp(sty);
                self.emit(Inst::Bin(BinK::And, t, v, Val::K(mask)));
                v = Val::R(t);
                if *signed {
                    // Sign-extend from w bits.
                    let sh = sty.bits() as i64 - *w as i64;
                    if sh > 0 {
                        let t1 = self.tmp(sty);
                        self.emit(Inst::Bin(BinK::Shl, t1, v, Val::K(sh)));
                        let t2 = self.tmp(sty);
                        self.emit(Inst::Bin(BinK::ShrS, t2, Val::R(t1), Val::K(sh)));
                        v = Val::R(t2);
                    }
                }
                // Then to the field's type.
                self.resize(v, sty, *vty, *signed)
            }
            LVal::SfrMulti(addrs) => {
                let n = addrs.len() as u32;
                let ty = Ty::from_bytes(n);
                let mut acc: Option<Val> = None;
                for (i, a) in addrs.iter().enumerate() {
                    let b = self.tmp(Ty::I8);
                    self.emit(Inst::Load(b, Mem::Abs(Space::Sfr, *a)));
                    let w = self.tmp(ty);
                    self.emit(Inst::Ext(w, Val::R(b), false));
                    let mut part = Val::R(w);
                    if i > 0 {
                        let s = self.tmp(ty);
                        self.emit(Inst::Bin(BinK::Shl, s, part, Val::K(8 * i as i64)));
                        part = Val::R(s);
                    }
                    acc = Some(match acc {
                        None => part,
                        Some(prev) => {
                            let o = self.tmp(ty);
                            self.emit(Inst::Bin(BinK::Or, o, prev, part));
                            Val::R(o)
                        }
                    });
                }
                acc.unwrap()
            }
        }
    }

    fn load_bytes(&mut self, m: Mem, nbytes: u32, ty: Ty) -> Val {
        if nbytes == ty.bytes() {
            let t = self.tmp(ty);
            self.emit(Inst::Load(t, m));
            return Val::R(t);
        }
        // Load a smaller integer and zero-extend.
        let lt = Ty::from_bytes(if nbytes == 3 { 4 } else { nbytes });
        let t = self.tmp(lt);
        self.emit(Inst::Load(t, m));
        if lt == ty {
            return Val::R(t);
        }
        let e = self.tmp(ty);
        self.emit(Inst::Ext(e, Val::R(t), false));
        Val::R(e)
    }

    fn store(&mut self, lv: &LVal, v: Val) {
        match lv {
            LVal::Reg(r) => self.emit(Inst::Copy(*r, v)),
            LVal::Mem(m, ty) => self.emit(Inst::Store(*m, v, *ty)),
            LVal::Bits(m, bo, w, _, _) => {
                let nbytes = ((*bo as u32 + *w as u32) + 7) / 8;
                let sty = Ty::from_bytes(if nbytes <= 1 { 1 } else if nbytes <= 2 { 2 } else { 4 });
                let mask = if *w as u32 >= sty.bits() { sty.mask() as i64 } else { ((1i64 << *w) - 1) << *bo };
                let mask = sty.norm(mask);
                let vt = self.val_ty(v, sty);
                // Truncate/extend value to storage width.
                let v = self.resize(v, vt, sty, false);
                let shifted = if *bo > 0 {
                    let t = self.tmp(sty);
                    self.emit(Inst::Bin(BinK::Shl, t, v, Val::K(*bo as i64)));
                    Val::R(t)
                } else {
                    v
                };
                let masked = {
                    let t = self.tmp(sty);
                    self.emit(Inst::Bin(BinK::And, t, shifted, Val::K(mask)));
                    Val::R(t)
                };
                if mask == sty.mask() as i64 {
                    self.emit(Inst::Store(m.clone(), masked, sty));
                    return;
                }
                let old = self.load_bytes(*m, nbytes, sty);
                let cleared = self.tmp(sty);
                self.emit(Inst::Bin(BinK::And, cleared, old, Val::K(sty.norm(!mask))));
                let new = self.tmp(sty);
                self.emit(Inst::Bin(BinK::Or, new, Val::R(cleared), masked));
                if nbytes == sty.bytes() {
                    self.emit(Inst::Store(*m, Val::R(new), sty));
                } else {
                    for i in 0..nbytes {
                        let b = self.byte_of(Val::R(new), sty, i);
                        self.emit(Inst::Store(mem_add(*m, i as i32), b, Ty::I8));
                    }
                }
            }
            LVal::SfrMulti(addrs) => {
                let ty = Ty::from_bytes(addrs.len() as u32);
                for (i, a) in addrs.iter().enumerate() {
                    let b = self.byte_of(v, ty, i as u32);
                    self.emit(Inst::Store(Mem::Abs(Space::Sfr, *a), b, Ty::I8));
                }
            }
        }
    }

    fn byte_of(&mut self, v: Val, ty: Ty, i: u32) -> Val {
        if let Val::K(k) = v {
            return Val::K((k >> (8 * i)) & 0xff);
        }
        let mut x = v;
        if i > 0 {
            let t = self.tmp(ty);
            self.emit(Inst::Bin(BinK::ShrU, t, x, Val::K(8 * i as i64)));
            x = Val::R(t);
        }
        let t = self.tmp(Ty::I8);
        self.emit(Inst::Trunc(t, x));
        Val::R(t)
    }

    /// Change width of an integer value.
    fn resize(&mut self, v: Val, from: Ty, to: Ty, signed: bool) -> Val {
        if from == to {
            return v;
        }
        if let Val::K(k) = v {
            let k = if signed { from.sext(k) } else { from.norm(k) };
            return Val::K(to.norm(k));
        }
        if to == Ty::Bit {
            let t = self.tmp(Ty::Bit);
            self.emit(Inst::Cmp(Cond::Ne, t, v, Val::K(0), from));
            return Val::R(t);
        }
        let t = self.tmp(to);
        if to.bits() > from.bits() {
            self.emit(Inst::Ext(t, v, signed && from != Ty::Bit));
        } else {
            if let Val::Addr(..) = v {
                if to == Ty::I8 {
                    self.emit(Inst::Trunc(t, v));
                    return Val::R(t);
                }
            }
            self.emit(Inst::Trunc(t, v));
        }
        Val::R(t)
    }

    // ------------------------------------------------------------------
    // Conversions

    fn convert(&mut self, v: Val, from: &Type, to: &Type) -> Result<Val> {
        if to.is_void() {
            return Ok(v);
        }
        if from.is_float() || to.is_float() {
            return self.float_convert(v, from, to);
        }
        let ft = self.ty(from);
        let tt = self.ty(to);
        if to.is_bool() {
            if from.is_bool() && ft == tt {
                return Ok(v);
            }
            if let Val::K(k) = v {
                return Ok(Val::K((ft.norm(k) != 0) as i64));
            }
            if let Val::Addr(..) = v {
                return Ok(Val::K(1));
            }
            if from.is_bit() && tt == Ty::I8 {
                return Ok(self.resize(v, Ty::Bit, Ty::I8, false));
            }
            if from.is_bool() && to.is_bit() {
                let t = self.tmp(Ty::Bit);
                self.emit(Inst::Trunc(t, v));
                return Ok(Val::R(t));
            }
            let (v, z, ft) = self.gptr_cmp_operands(v, Val::K(0), ft);
            let t = self.tmp(tt);
            self.emit(Inst::Cmp(Cond::Ne, t, v, z, ft));
            return Ok(Val::R(t));
        }
        // Pointer conversions between generic and specific.
        if from.is_pointer() && to.is_pointer() && ft != tt {
            if tt == Ty::I24 {
                let tag = match self.prog.ptr_space(from) {
                    Some(s) => s.gptr_tag(),
                    None => 0x40,
                };
                return Ok(self.make_gptr(v, tag));
            }
            return Ok(self.resize(v, ft, tt, false));
        }
        if from.is_integer() && to.is_pointer() && tt == Ty::I24 {
            // Integer to generic pointer: tag from the pointee's declared space if any.
            let tag = to.pointee().and_then(|p| p.q.space).map(|s| s.gptr_tag()).unwrap_or(0x40);
            let v16 = self.resize(v, ft, Ty::I16, from.is_signed());
            return Ok(self.make_gptr(v16, tag));
        }
        if let (Val::Addr(s, o), Ty::I8) = (v, tt) {
            let t = self.tmp(Ty::I8);
            self.emit(Inst::Trunc(t, Val::Addr(s, o)));
            return Ok(Val::R(t));
        }
        Ok(self.resize(v, ft, tt, from.is_signed()))
    }

    /// Operands for comparing generic pointers: a pointer whose address bytes are zero is NULL
    /// whatever its tag (as in SDCC).
    fn gptr_cmp_operands(&mut self, a: Val, b: Val, ty: Ty) -> (Val, Val, Ty) {
        if ty != Ty::I24 {
            return (a, b, ty);
        }
        let is_null = |v: Val| matches!(v, Val::K(k) if k & 0xffff == 0);
        if is_null(a) || is_null(b) {
            let other = if is_null(b) { a } else { b };
            let lo = self.resize(other, Ty::I24, Ty::I16, false);
            return if is_null(b) { (lo, Val::K(0), Ty::I16) } else { (Val::K(0), lo, Ty::I16) };
        }
        (self.gptr_normalize(a), self.gptr_normalize(b), ty)
    }

    fn gptr_normalize(&mut self, v: Val) -> Val {
        if !matches!(v, Val::R(_)) {
            return v;
        }
        let lo = self.resize(v, Ty::I24, Ty::I16, false);
        let nz = self.tmp(Ty::I8);
        self.emit(Inst::Cmp(Cond::Ne, nz, lo, Val::K(0), Ty::I16));
        let m = self.tmp(Ty::I8);
        self.emit(Inst::Bin(BinK::Sub, m, Val::K(0), Val::R(nz)));
        let hi = self.tmp(Ty::I24);
        self.emit(Inst::Bin(BinK::ShrU, hi, v, Val::K(16)));
        let hi8 = self.resize(Val::R(hi), Ty::I24, Ty::I8, false);
        let tg = self.tmp(Ty::I8);
        self.emit(Inst::Bin(BinK::And, tg, hi8, Val::R(m)));
        let tg24 = self.resize(Val::R(tg), Ty::I8, Ty::I24, false);
        let sh = self.tmp(Ty::I24);
        self.emit(Inst::Bin(BinK::Shl, sh, tg24, Val::K(16)));
        let lo24 = self.resize(lo, Ty::I16, Ty::I24, false);
        let r = self.tmp(Ty::I24);
        self.emit(Inst::Bin(BinK::Or, r, lo24, Val::R(sh)));
        Val::R(r)
    }

    fn make_gptr(&mut self, v: Val, tag: u8) -> Val {
        // The address of a known object carries its own space (the type's space class may be ambiguous).
        let tag = match v {
            Val::Addr(Sym::Global(g), _) => match self.prog.globals[g].space {
                s @ (Space::Code | Space::Xdata | Space::Pdata) => s.gptr_tag(),
                _ => Space::Data.gptr_tag(),
            },
            Val::Addr(Sym::Func(_), _) => Space::Code.gptr_tag(),
            Val::Addr(Sym::Frame(..), _) => Space::Data.gptr_tag(),
            _ => tag,
        };
        if let Val::K(k) = v {
            return Val::K((k & 0xffff) | ((tag as i64) << 16));
        }
        let e = self.tmp(Ty::I24);
        self.emit(Inst::Ext(e, v, false));
        if tag == 0 {
            return Val::R(e);
        }
        let o = self.tmp(Ty::I24);
        self.emit(Inst::Bin(BinK::Or, o, Val::R(e), Val::K((tag as i64) << 16)));
        Val::R(o)
    }

    fn float_convert(&mut self, v: Val, from: &Type, to: &Type) -> Result<Val> {
        if from.is_float() && to.is_float() {
            return Ok(v);
        }
        if to.is_float() {
            // int -> float
            let ft = self.ty(from);
            let signed = from.is_signed();
            let v32 = self.resize(v, ft, Ty::I32, signed);
            let t = self.tmp(Ty::I32);
            let name = if signed { "__sl2fs" } else { "__ul2fs" };
            let c = self.lib_callee(name);
            self.emit(Inst::Call(Some(t), c, vec![v32]));
            return Ok(Val::R(t));
        }
        // float -> int
        let tt = self.ty(to);
        if to.is_bool() {
            let t = self.tmp(tt);
            // Compare ignoring the sign bit.
            let m = self.tmp(Ty::I32);
            self.emit(Inst::Bin(BinK::And, m, v, Val::K(0x7fff_ffff)));
            self.emit(Inst::Cmp(Cond::Ne, t, Val::R(m), Val::K(0), Ty::I32));
            return Ok(Val::R(t));
        }
        let t = self.tmp(Ty::I32);
        let name = if to.is_signed() { "__fs2sl" } else { "__fs2ul" };
        let c = self.lib_callee(name);
        self.emit(Inst::Call(Some(t), c, vec![v]));
        Ok(self.resize(Val::R(t), Ty::I32, tt, to.is_signed()))
    }

    // ------------------------------------------------------------------
    // Expressions

    /// Evaluate for side effects only.
    fn effect(&mut self, e: &Expr) -> Result<()> {
        match &e.kind {
            ExprKind::Cast(inner) if e.ty.is_void() => self.effect(inner),
            ExprKind::Comma(a, b) => {
                self.effect(a)?;
                self.effect(b)
            }
            ExprKind::Int(_) | ExprKind::Float(_) => Ok(()),
            ExprKind::IncDec(l, d, _) => {
                // Pre/post doesn't matter.
                self.incdec(l, *d, false)?;
                Ok(())
            }
            ExprKind::Cond(c, a, b) if e.ty.is_void() || true => {
                let (tb, fb, join) = (self.f.new_block(), self.f.new_block(), self.f.new_block());
                self.cond_branch(c, tb, fb)?;
                self.start(tb);
                self.effect(a)?;
                self.terminate(Term::Jmp(join));
                self.start(fb);
                self.effect(b)?;
                self.start(join);
                Ok(())
            }
            ExprKind::Binary(BinOp::LogAnd | BinOp::LogOr, ..) => {
                let (tb, join) = (self.f.new_block(), self.f.new_block());
                self.cond_branch(e, tb, join)?;
                self.start(tb);
                self.start(join);
                Ok(())
            }
            _ => {
                if e.ty.is_record() {
                    if let ExprKind::Call(..) = e.kind {
                        self.call(e)?;
                        return Ok(());
                    }
                    let _ = self.lvalue(e)?;
                    return Ok(());
                }
                self.rvalue(e)?;
                Ok(())
            }
        }
    }

    pub fn rvalue(&mut self, e: &Expr) -> Result<Val> {
        let ty = if e.ty.is_scalar() || e.ty.is_void() { self.ty(&e.ty) } else { Ty::I16 };
        match &e.kind {
            ExprKind::Int(v) => Ok(Val::K(ty.norm(*v))),
            ExprKind::Float(f) => Ok(Val::K((*f as f32).to_bits() as i64)),
            ExprKind::Global(_) | ExprKind::Local(_) | ExprKind::Deref(_) | ExprKind::Member(..) => {
                if !e.ty.is_scalar() {
                    return err(e.loc, "aggregate used as scalar value");
                }
                let lv = self.lvalue(e)?;
                Ok(self.load(&lv))
            }
            ExprKind::Func(f) => Ok(Val::Addr(Sym::Func(*f), 0)),
            ExprKind::AddrOf(inner) => {
                if let ExprKind::Func(f) = inner.kind {
                    return Ok(Val::Addr(Sym::Func(f), 0));
                }
                let lv = self.lvalue(inner)?;
                let v = match lv {
                    LVal::Mem(Mem::Sym(s, o), _) => Val::Addr(s, o),
                    LVal::Mem(Mem::Abs(_, a), _) => Val::K(a as i64),
                    LVal::Mem(Mem::Ptr(p, o, _), _) => {
                        if o == 0 {
                            p
                        } else {
                            let pt = self.val_ty(p, ty);
                            self.ptr_offset(p, pt, Val::K(o as i64))
                        }
                    }
                    _ => return err(e.loc, "cannot take the address of this object"),
                };
                // Result may be a generic pointer.
                if ty == Ty::I24 && self.val_ty(v, Ty::I16) != Ty::I24 {
                    let tag = match &inner.kind {
                        ExprKind::Global(g) => self.prog.globals[*g].space.gptr_tag(),
                        _ => match self.prog.ptr_space(&e.ty) {
                            Some(s) => s.gptr_tag(),
                            None => 0x40,
                        },
                    };
                    let tag = if let ExprKind::Deref(p) = &inner.kind {
                        self.prog.ptr_space(&p.ty).map(|s| s.gptr_tag()).unwrap_or(tag)
                    } else {
                        tag
                    };
                    return Ok(self.make_gptr(v, tag));
                }
                Ok(v)
            }
            ExprKind::Cast(inner) => {
                if e.ty.is_void() {
                    self.effect(inner)?;
                    return Ok(Val::K(0));
                }
                let v = self.rvalue(inner)?;
                self.convert(v, &inner.ty, &e.ty)
            }
            ExprKind::Unary(op, a) => {
                let v = self.rvalue(a)?;
                match op {
                    UnOp::LogNot => {
                        let at = self.ty(&a.ty);
                        if a.ty.is_float() {
                            let m = self.tmp(Ty::I32);
                            self.emit(Inst::Bin(BinK::And, m, v, Val::K(0x7fff_ffff)));
                            let t = self.tmp(Ty::I16);
                            self.emit(Inst::Cmp(Cond::Eq, t, Val::R(m), Val::K(0), Ty::I32));
                            return Ok(Val::R(t));
                        }
                        let (v, z, at) = self.gptr_cmp_operands(v, Val::K(0), at);
                        let t = self.tmp(Ty::I8);
                        self.emit(Inst::Cmp(Cond::Eq, t, v, z, at));
                        Ok(self.resize(Val::R(t), Ty::I8, ty, false))
                    }
                    UnOp::Neg => {
                        if e.ty.is_float() {
                            let t = self.tmp(Ty::I32);
                            self.emit(Inst::Bin(BinK::Xor, t, v, Val::K(0x8000_0000)));
                            return Ok(Val::R(t));
                        }
                        let t = self.tmp(ty);
                        self.emit(Inst::Un(UnK::Neg, t, v));
                        Ok(Val::R(t))
                    }
                    UnOp::BitNot => {
                        let t = self.tmp(ty);
                        self.emit(Inst::Un(UnK::Not, t, v));
                        Ok(Val::R(t))
                    }
                }
            }
            ExprKind::Binary(op, a, b) => self.binary(e, *op, a, b, ty),
            ExprKind::PtrAdd(p, i) => {
                let pv = self.rvalue(p)?;
                let iv = self.rvalue(i)?;
                Ok(self.ptr_offset(pv, ty, iv))
            }
            ExprKind::PtrDiff(a, b, size) => {
                let at = self.ty(&a.ty);
                let av = self.rvalue(a)?;
                let bv = self.rvalue(b)?;
                let av = self.resize(av, at, Ty::I16, false);
                let bv = self.resize(bv, at, Ty::I16, false);
                let d = self.tmp(Ty::I16);
                self.emit(Inst::Bin(BinK::Sub, d, av, bv));
                if *size == 1 {
                    return Ok(Val::R(d));
                }
                let q = self.tmp(Ty::I16);
                if size.is_power_of_two() {
                    self.emit(Inst::Bin(BinK::ShrS, q, Val::R(d), Val::K(size.trailing_zeros() as i64)));
                } else {
                    self.emit(Inst::Bin(BinK::DivS, q, Val::R(d), Val::K(*size as i64)));
                }
                Ok(Val::R(q))
            }
            ExprKind::Assign(l, r) => {
                if e.ty.is_record() {
                    let size = self.prog.size(&e.ty);
                    let src = self.agg_mem(r)?;
                    let dst = self.agg_mem(l)?;
                    self.emit(Inst::MemCopy(dst, src, size));
                    return Ok(Val::K(0));
                }
                let v = self.rvalue(r)?;
                let lv = self.lvalue(l)?;
                // Keep the stored value in a vreg only if needed later (the optimizer removes the copy otherwise).
                self.store(&lv, v);
                if let LVal::Bits(_, _, w, _, signed) = lv {
                    if signed {
                        return Ok(self.load(&lv));
                    }
                    // The value of the assignment is the truncated bit-field value.
                    let vt = self.val_ty(v, ty);
                    let mask = if w as u32 >= vt.bits() { vt.mask() as i64 } else { (1i64 << w) - 1 };
                    let t = self.tmp(vt);
                    self.emit(Inst::Bin(BinK::And, t, v, Val::K(mask)));
                    return Ok(Val::R(t));
                }
                Ok(v)
            }
            ExprKind::CompoundAssign(op, l, r, op_ty) => {
                let lv = self.lvalue(l)?;
                let rv = self.rvalue(r)?;
                let old = self.load(&lv);
                let res = if l.ty.is_pointer() {
                    let lt = self.ty(&l.ty);
                    self.ptr_offset(old, lt, rv)
                } else {
                    let ov = self.convert(old, &l.ty, op_ty)?;
                    let rv2 = if matches!(op, BinOp::Shl | BinOp::Shr) { rv } else { rv };
                    let rt = if matches!(op, BinOp::Shl | BinOp::Shr) { r.ty.clone() } else { op_ty.clone() };
                    let nv = self.arith(*op, ov, rv2, op_ty, &rt)?;
                    self.convert(nv, op_ty, &l.ty)?
                };
                self.store(&lv, res);
                Ok(res)
            }
            ExprKind::IncDec(l, d, post) => self.incdec(l, *d, *post),
            ExprKind::Cond(c, a, b) => {
                // Constant-valued arms: use a result vreg.
                if e.ty.is_void() {
                    self.effect(e)?;
                    return Ok(Val::K(0));
                }
                let r = self.tmp(ty);
                let (tb, fb, join) = (self.f.new_block(), self.f.new_block(), self.f.new_block());
                self.cond_branch(c, tb, fb)?;
                self.start(tb);
                let av = self.rvalue(a)?;
                self.emit(Inst::Copy(r, av));
                self.terminate(Term::Jmp(join));
                self.start(fb);
                let bv = self.rvalue(b)?;
                self.emit(Inst::Copy(r, bv));
                self.start(join);
                Ok(Val::R(r))
            }
            ExprKind::Comma(a, b) => {
                self.effect(a)?;
                self.rvalue(b)
            }
            ExprKind::Call(..) => {
                let r = self.call(e)?;
                Ok(r.unwrap_or(Val::K(0)))
            }
            ExprKind::StmtExpr(stmts, v) => {
                for s in stmts {
                    self.stmt(s)?;
                }
                match v {
                    Some(v) => self.rvalue(v),
                    None => Ok(Val::K(0)),
                }
            }
            ExprKind::Builtin(Builtin::VaStart, args) => {
                if !self.prog.funcs[self.fid].ftype().variadic {
                    return err(e.loc, "va_start used in a function with fixed arguments");
                }
                let obj = varargs_obj_index(self.prog, self.fid);
                let lv = self.lvalue(&args[0])?;
                self.store(&lv, Val::Addr(Sym::Frame(self.fid, obj), 0));
                Ok(Val::K(0))
            }
            ExprKind::Builtin(Builtin::VaArg, args) => {
                let ap = &args[0];
                let lv = self.lvalue(ap)?;
                let p = self.load(&lv);
                let sp = ptr_pspace(self.prog, &ap.ty);
                let size = if e.ty.is_bit() { 1 } else { self.prog.size(&e.ty) };
                let lt = if e.ty.is_bit() { Ty::I8 } else { ty };
                let t = self.tmp(lt);
                self.emit(Inst::Load(t, Mem::Ptr(p, 0, sp)));
                let pt = self.val_ty(p, Ty::I16);
                let np = self.ptr_offset(p, pt, Val::K(size as i64));
                self.store(&lv, np);
                if e.ty.is_bit() {
                    return Ok(self.resize(Val::R(t), Ty::I8, Ty::Bit, false));
                }
                Ok(Val::R(t))
            }
            ExprKind::Builtin(_, _) => Ok(Val::K(0)),
        }
    }

    fn ptr_offset(&mut self, p: Val, pty: Ty, off: Val) -> Val {
        if off == Val::K(0) {
            return p;
        }
        match (p, off) {
            (Val::Addr(s, o), Val::K(k)) => return Val::Addr(s, o + k as i16 as i32),
            (Val::K(a), Val::K(k)) => {
                let lo = (a + k) & 0xffff;
                return Val::K((a & !0xffff) | lo);
            }
            _ => {}
        }
        if pty == Ty::I24 {
            // Only the low 16 bits change.
            let lo = self.tmp(Ty::I16);
            self.emit(Inst::Trunc(lo, p));
            let sum = self.tmp(Ty::I16);
            self.emit(Inst::Bin(BinK::Add, sum, Val::R(lo), off));
            let tag = self.tmp(Ty::I24);
            self.emit(Inst::Bin(BinK::And, tag, p, Val::K(0xff0000)));
            let wide = self.tmp(Ty::I24);
            self.emit(Inst::Ext(wide, Val::R(sum), false));
            let r = self.tmp(Ty::I24);
            self.emit(Inst::Bin(BinK::Or, r, Val::R(wide), Val::R(tag)));
            return Val::R(r);
        }
        let t = self.tmp(Ty::I16);
        self.emit(Inst::Bin(BinK::Add, t, p, off));
        Val::R(t)
    }

    fn incdec(&mut self, l: &Expr, d: i64, post: bool) -> Result<Val> {
        let lv = self.lvalue(l)?;
        let old = self.load(&lv);
        let lt = self.ty(&l.ty);
        let new = if l.ty.is_pointer() {
            self.ptr_offset(old, lt, Val::K(Ty::I16.norm(d)))
        } else if l.ty.is_float() {
            let one = Val::K((d as f32).to_bits() as i64);
            let t = self.tmp(Ty::I32);
            let c = self.lib_callee("__fsadd");
            self.emit(Inst::Call(Some(t), c, vec![old, one]));
            Val::R(t)
        } else if l.ty.is_bool() {
            // bool++ sets to 1; bool-- toggles.
            if d > 0 {
                Val::K(1)
            } else {
                let t = self.tmp(lt);
                self.emit(Inst::Bin(BinK::Xor, t, old, Val::K(1)));
                Val::R(t)
            }
        } else {
            let t = self.tmp(lt);
            self.emit(Inst::Bin(BinK::Add, t, old, Val::K(lt.norm(d))));
            Val::R(t)
        };
        if post {
            // Preserve the old value if it is a register that gets overwritten.
            let saved = match (lv.clone(), old) {
                (LVal::Reg(r), Val::R(o)) if r == o => {
                    let t = self.tmp(lt);
                    self.emit(Inst::Copy(t, old));
                    // Recompute `new` from the saved copy to keep dataflow simple.
                    Val::R(t)
                }
                _ => old,
            };
            self.store(&lv, new);
            Ok(saved)
        } else {
            self.store(&lv, new);
            Ok(new)
        }
    }

    fn binary(&mut self, e: &Expr, op: BinOp, a: &Expr, b: &Expr, ty: Ty) -> Result<Val> {
        match op {
            BinOp::LogAnd | BinOp::LogOr => {
                let r = self.tmp(ty);
                let (tb, fb, join) = (self.f.new_block(), self.f.new_block(), self.f.new_block());
                self.cond_branch(e, tb, fb)?;
                self.start(tb);
                self.emit(Inst::Copy(r, Val::K(1)));
                self.terminate(Term::Jmp(join));
                self.start(fb);
                self.emit(Inst::Copy(r, Val::K(0)));
                self.start(join);
                Ok(Val::R(r))
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let av = self.rvalue(a)?;
                let bv = self.rvalue(b)?;
                if a.ty.is_float() {
                    return self.float_cmp(op, av, bv, ty);
                }
                let at = self.ty(&a.ty);
                let (av, bv, at) = self.gptr_cmp_operands(av, bv, at);
                let cond = cond_of(op, a.ty.is_signed() && !a.ty.is_pointer());
                let t = self.tmp(Ty::I8);
                self.emit(Inst::Cmp(cond, t, av, bv, at));
                Ok(self.resize(Val::R(t), Ty::I8, ty, false))
            }
            _ => {
                let av = self.rvalue(a)?;
                let bv = self.rvalue(b)?;
                self.arith(op, av, bv, &e.ty, &b.ty)
            }
        }
    }

    fn float_cmp(&mut self, op: BinOp, a: Val, b: Val, ty: Ty) -> Result<Val> {
        let t = self.tmp(Ty::I8);
        let (name, swap) = match op {
            BinOp::Eq => ("__fseq", false),
            BinOp::Ne => ("__fseq", false),
            BinOp::Lt => ("__fslt", false),
            BinOp::Gt => ("__fslt", true),
            BinOp::Le => ("__fslt", true),
            BinOp::Ge => ("__fslt", false),
            _ => unreachable!(),
        };
        let args = if swap { vec![b, a] } else { vec![a, b] };
        let c = self.lib_callee(name);
        self.emit(Inst::Call(Some(t), c, args));
        let r = if matches!(op, BinOp::Ne | BinOp::Le | BinOp::Ge) {
            let n = self.tmp(Ty::I8);
            self.emit(Inst::Bin(BinK::Xor, n, Val::R(t), Val::K(1)));
            Val::R(n)
        } else {
            Val::R(t)
        };
        Ok(self.resize(r, Ty::I8, ty, false))
    }

    /// Arithmetic with operands already converted (shift count may have another type).
    fn arith(&mut self, op: BinOp, a: Val, b: Val, ty: &Type, bty: &Type) -> Result<Val> {
        let t = self.ty(ty);
        if ty.is_float() {
            let name = match op {
                BinOp::Add => "__fsadd",
                BinOp::Sub => "__fssub",
                BinOp::Mul => "__fsmul",
                BinOp::Div => "__fsdiv",
                _ => return err(Loc::default(), "invalid floating point operation"),
            };
            let r = self.tmp(Ty::I32);
            let c = self.lib_callee(name);
            self.emit(Inst::Call(Some(r), c, vec![a, b]));
            return Ok(Val::R(r));
        }
        let signed = ty.is_signed();
        let k = match op {
            BinOp::Add => BinK::Add,
            BinOp::Sub => BinK::Sub,
            BinOp::Mul => BinK::Mul,
            BinOp::Div => {
                if signed {
                    BinK::DivS
                } else {
                    BinK::DivU
                }
            }
            BinOp::Mod => {
                if signed {
                    BinK::ModS
                } else {
                    BinK::ModU
                }
            }
            BinOp::And => BinK::And,
            BinOp::Or => BinK::Or,
            BinOp::Xor => BinK::Xor,
            BinOp::Shl => BinK::Shl,
            BinOp::Shr => {
                if signed {
                    BinK::ShrS
                } else {
                    BinK::ShrU
                }
            }
            _ => unreachable!(),
        };
        let b = if matches!(op, BinOp::Shl | BinOp::Shr) {
            // Shift counts are always 8-bit in the IR.
            let bt = self.ty(bty);
            match b {
                Val::K(c) => Val::K(if bt.sext(c) < 0 || bt.norm(c) > 64 { 64 } else { bt.norm(c) }),
                _ => self.resize(b, bt, Ty::I8, false),
            }
        } else {
            b
        };
        let r = self.tmp(t);
        self.emit(Inst::Bin(k, r, a, b));
        Ok(Val::R(r))
    }

    fn call(&mut self, e: &Expr) -> Result<Option<Val>> {
        let ExprKind::Call(callee, args) = &e.kind else { unreachable!() };
        let ft = callee.ty.func().unwrap().clone();
        let (target, def_params): (Callee, Option<Vec<Type>>) = match &callee.kind {
            ExprKind::Func(fid) => {
                let fft = self.prog.funcs[*fid].ftype().clone();
                (Callee::Direct(*fid), Some(fft.params.clone()))
            }
            ExprKind::AddrOf(inner) if matches!(inner.kind, ExprKind::Func(_)) => {
                let ExprKind::Func(fid) = inner.kind else { unreachable!() };
                let fft = self.prog.funcs[fid].ftype().clone();
                (Callee::Direct(fid), Some(fft.params.clone()))
            }
            _ => {
                let v = self.rvalue(callee)?;
                match v {
                    Val::Addr(Sym::Func(fid), 0) => {
                        let fft = self.prog.funcs[fid].ftype().clone();
                        (Callee::Direct(fid), Some(fft.params.clone()))
                    }
                    _ => (Callee::Indirect(v, Rc::from(Vec::new())), None),
                }
            }
        };
        let params = def_params.unwrap_or_else(|| ft.params.clone());
        let target = match target {
            Callee::Indirect(v, _) => Callee::Indirect(v, params.iter().map(|t| if t.is_scalar() { self.ty(t) } else { Ty::I16 }).collect()),
            t => t,
        };
        let variadic_extra = args.len() > params.len();
        // Evaluate variable arguments first; they are stored after all arguments are evaluated.
        let mut var_vals: Vec<(Val, Ty)> = Vec::new();
        if variadic_extra {
            for a in &args[params.len()..] {
                let v = self.rvalue(a)?;
                if self.prog.is_generic_vararg_ptr(&a.ty) || a.ty.is_array() {
                    // Pass data pointers as generic pointers.
                    let pt = self.ty(&a.ty);
                    let gv = if pt == Ty::I24 {
                        v
                    } else {
                        let tag = self.prog.ptr_space(&a.ty).map(|s| s.gptr_tag()).unwrap_or(0x40);
                        self.make_gptr(v, tag)
                    };
                    var_vals.push((gv, Ty::I24));
                    continue;
                }
                let t = if a.ty.is_array() { Ty::I16 } else { self.ty(&a.ty) };
                let (v, t) = if t == Ty::Bit { (self.resize(v, Ty::Bit, Ty::I8, false), Ty::I8) } else { (v, t) };
                // Keep constants/addresses as is; registers may be clobbered by later argument calls only if they are memory reads.
                var_vals.push((v, t));
            }
        }
        let mut vals = Vec::new();
        for (i, a) in args.iter().enumerate().take(params.len()) {
            let pt = params.get(i).cloned().unwrap_or_else(|| a.ty.clone());
            if pt.is_record() {
                let Callee::Direct(fid) = target else { return err(e.loc, "struct arguments to indirect calls are not supported") };
                let af = &self.prog.funcs[fid];
                let idx = param_frame_index(self.prog, af, i).unwrap();
                let src = self.agg_mem(a)?;
                let size = self.prog.size(&pt);
                self.emit(Inst::MemCopy(Mem::Sym(Sym::Frame(fid, idx), 0), src, size));
                vals.push(Val::K(0));
                continue;
            }
            let v = self.rvalue(a)?;
            let v = if !pt.same(&a.ty) || self.ty(&pt) != self.ty(&a.ty) { self.convert(v, &a.ty, &pt)? } else { v };
            vals.push(v);
        }
        if variadic_extra {
            let Callee::Direct(fid) = target else { return err(e.loc, "variadic calls through function pointers are not supported") };
            let obj = varargs_obj_index(self.prog, fid);
            let mut off = 0i32;
            for (v, t) in var_vals {
                self.emit(Inst::Store(Mem::Sym(Sym::Frame(fid, obj), off), v, t));
                off += t.bytes() as i32;
            }
        }
        if ft.ret.is_void() || ft.ret.is_record() {
            self.emit(Inst::Call(None, target, vals));
            if ft.attrs.noreturn {
                self.terminate(Term::Unreachable);
            }
            return Ok(None);
        }
        let rt = self.ty(&ft.ret);
        let r = self.tmp(rt);
        self.emit(Inst::Call(Some(r), target, vals));
        if ft.attrs.noreturn {
            self.terminate(Term::Unreachable);
        }
        Ok(Some(Val::R(r)))
    }

    // ------------------------------------------------------------------
    // Conditions

    fn cond_branch(&mut self, e: &Expr, t: BlockId, f: BlockId) -> Result<()> {
        match &e.kind {
            ExprKind::Int(v) => {
                self.terminate(Term::Jmp(if *v != 0 { t } else { f }));
                Ok(())
            }
            ExprKind::Unary(UnOp::LogNot, a) => self.cond_branch(a, f, t),
            ExprKind::Binary(BinOp::LogAnd, a, b) => {
                let mid = self.f.new_block();
                self.cond_branch(a, mid, f)?;
                self.start(mid);
                self.cond_branch(b, t, f)
            }
            ExprKind::Binary(BinOp::LogOr, a, b) => {
                let mid = self.f.new_block();
                self.cond_branch(a, t, mid)?;
                self.start(mid);
                self.cond_branch(b, t, f)
            }
            ExprKind::Binary(op, a, b) if op.is_cmp() && !a.ty.is_float() => {
                let av = self.rvalue(a)?;
                let bv = self.rvalue(b)?;
                let at = self.ty(&a.ty);
                let (av, bv, at) = self.gptr_cmp_operands(av, bv, at);
                let cond = cond_of(*op, a.ty.is_signed() && !a.ty.is_pointer());
                self.terminate(Term::CmpBr(cond, av, bv, at, t, f));
                Ok(())
            }
            ExprKind::Cast(inner) if e.ty.is_bool() && inner.ty.is_scalar() && !inner.ty.is_float() => self.cond_branch(inner, t, f),
            ExprKind::Comma(a, b) => {
                self.effect(a)?;
                self.cond_branch(b, t, f)
            }
            _ => {
                let v = self.rvalue(e)?;
                if e.ty.is_float() {
                    let m = self.tmp(Ty::I32);
                    self.emit(Inst::Bin(BinK::And, m, v, Val::K(0x7fff_ffff)));
                    self.terminate(Term::Br(Val::R(m), t, f));
                    return Ok(());
                }
                match v {
                    Val::K(k) => self.terminate(Term::Jmp(if k != 0 { t } else { f })),
                    Val::Addr(..) => self.terminate(Term::Jmp(t)),
                    _ => {
                        // Generic pointers: test only the address bytes.
                        let v = if self.val_ty(v, Ty::I16) == Ty::I24 {
                            let lo = self.tmp(Ty::I16);
                            self.emit(Inst::Trunc(lo, v));
                            Val::R(lo)
                        } else {
                            v
                        };
                        self.terminate(Term::Br(v, t, f))
                    }
                }
                Ok(())
            }
        }
    }

    // ------------------------------------------------------------------
    // Statements

    fn label_block(&mut self, l: LabelId) -> BlockId {
        if let Some(b) = self.labels.get(&l) {
            return *b;
        }
        let b = self.f.new_block();
        self.labels.insert(l, b);
        b
    }

    fn stmt(&mut self, s: &Stmt) -> Result<()> {
        match s {
            Stmt::Empty => Ok(()),
            Stmt::Expr(e) => self.effect(e),
            Stmt::Block(v) => {
                for s in v {
                    self.stmt(s)?;
                }
                Ok(())
            }
            Stmt::If(c, t, e) => {
                let tb = self.f.new_block();
                let join = self.f.new_block();
                let fb = if e.is_some() { self.f.new_block() } else { join };
                self.cond_branch(c, tb, fb)?;
                self.start(tb);
                self.stmt(t)?;
                if let Some(e) = e {
                    self.terminate_if_open(Term::Jmp(join));
                    self.start(fb);
                    self.stmt(e)?;
                }
                self.start(join);
                Ok(())
            }
            Stmt::While(c, body) => {
                let (test, bodyb, exit) = (self.f.new_block(), self.f.new_block(), self.f.new_block());
                self.start(test);
                self.cond_branch(c, bodyb, exit)?;
                self.start(bodyb);
                self.breaks.push(exit);
                self.continues.push(test);
                self.stmt(body)?;
                self.breaks.pop();
                self.continues.pop();
                self.terminate_if_open(Term::Jmp(test));
                self.start(exit);
                Ok(())
            }
            Stmt::DoWhile(body, c) => {
                let (bodyb, test, exit) = (self.f.new_block(), self.f.new_block(), self.f.new_block());
                self.start(bodyb);
                self.breaks.push(exit);
                self.continues.push(test);
                self.stmt(body)?;
                self.breaks.pop();
                self.continues.pop();
                self.start(test);
                self.cond_branch(c, bodyb, exit)?;
                self.start(exit);
                Ok(())
            }
            Stmt::For(init, c, step, body) => {
                if let Some(i) = init {
                    self.stmt(i)?;
                }
                let (test, bodyb, stepb, exit) = (self.f.new_block(), self.f.new_block(), self.f.new_block(), self.f.new_block());
                self.start(test);
                match c {
                    Some(c) => self.cond_branch(c, bodyb, exit)?,
                    None => self.terminate(Term::Jmp(bodyb)),
                }
                self.start(bodyb);
                self.breaks.push(exit);
                self.continues.push(stepb);
                self.stmt(body)?;
                self.breaks.pop();
                self.continues.pop();
                self.start(stepb);
                if let Some(st) = step {
                    self.effect(st)?;
                }
                self.terminate_if_open(Term::Jmp(test));
                self.start(exit);
                Ok(())
            }
            Stmt::Switch(e, body, cases, default) => {
                let v = self.rvalue(e)?;
                let ty = self.ty(&e.ty);
                let exit = self.f.new_block();
                let mut ir_cases = Vec::new();
                for (val, l) in cases {
                    let b = self.label_block(*l);
                    ir_cases.push((ty.norm(*val), b));
                }
                let d = match default {
                    Some(l) => self.label_block(*l),
                    None => exit,
                };
                if let Val::K(k) = v {
                    let target = ir_cases.iter().find(|c| c.0 == ty.norm(k)).map(|c| c.1).unwrap_or(d);
                    self.terminate(Term::Jmp(target));
                } else {
                    self.terminate(Term::Switch(v, ty, ir_cases, d));
                }
                self.breaks.push(exit);
                self.stmt(body)?;
                self.breaks.pop();
                self.start(exit);
                Ok(())
            }
            Stmt::Break => {
                let b = *self.breaks.last().unwrap();
                self.terminate(Term::Jmp(b));
                Ok(())
            }
            Stmt::Continue => {
                let b = *self.continues.last().unwrap();
                self.terminate(Term::Jmp(b));
                Ok(())
            }
            Stmt::Return(e, loc) => {
                let v = match e {
                    Some(e) if e.ty.is_record() => {
                        let size = self.prog.size(&e.ty);
                        let src = self.agg_mem(e)?;
                        let obj = self.f.ret_obj.unwrap();
                        self.emit(Inst::MemCopy(Mem::Sym(Sym::Frame(self.fid, obj), 0), src, size));
                        None
                    }
                    Some(e) => Some(self.rvalue(e)?),
                    None => None,
                };
                let _ = loc;
                for &c in self.crit.clone().iter().rev() {
                    self.emit(Inst::CritExit(c));
                }
                let v = if self.f.ret.is_some() { Some(v.unwrap_or(Val::K(0))) } else { None };
                let _ = &self.ret_type;
                self.terminate(Term::Ret(v));
                Ok(())
            }
            Stmt::Goto(l, _) => {
                let b = self.label_block(*l);
                self.terminate(Term::Jmp(b));
                Ok(())
            }
            Stmt::Label(l, s) => {
                let b = self.label_block(*l);
                self.start(b);
                self.stmt(s)
            }
            Stmt::Asm(text, _) => {
                self.emit(Inst::Asm(text.as_str().into()));
                Ok(())
            }
            Stmt::Critical(body) => {
                let v = self.f.new_vreg(Ty::Bit);
                self.emit(Inst::CritEnter(v));
                self.crit.push(v);
                self.stmt(body)?;
                self.crit.pop();
                self.emit(Inst::CritExit(v));
                Ok(())
            }
            Stmt::InitLocal(lid, inits, _) => self.init_local(*lid, inits),
        }
    }

    fn terminate_if_open(&mut self, t: Term) {
        if !self.sealed {
            self.terminate(t);
        }
    }

    fn init_local(&mut self, lid: LocalId, inits: &[LocalInit]) -> Result<()> {
        let st = self.local_storage(lid);
        let l = &self.prog.funcs[self.fid].locals[lid];
        let lty = l.ty.clone();
        match st {
            Storage::Reg(v) => {
                for i in inits {
                    if let LocalInit::Scalar(_, _, e) = i {
                        let val = self.rvalue(e)?;
                        self.emit(Inst::Copy(v, val));
                    }
                }
                Ok(())
            }
            Storage::Frame(obj) => {
                let size = self.prog.size(&lty);
                let base = Mem::Sym(Sym::Frame(self.fid, obj), 0);
                // Bytes covered by explicit initializers.
                let mut covered = vec![false; size as usize];
                for i in inits {
                    match i {
                        LocalInit::Scalar(off, None, e) => {
                            let n = if e.ty.is_bit() { 1 } else { self.prog.size(&e.ty) };
                            for k in 0..n {
                                if let Some(c) = covered.get_mut((off + k) as usize) {
                                    *c = true;
                                }
                            }
                        }
                        LocalInit::Aggregate(off, e) => {
                            for k in 0..self.prog.size(&e.ty) {
                                if let Some(c) = covered.get_mut((off + k) as usize) {
                                    *c = true;
                                }
                            }
                        }
                        LocalInit::Bytes(off, b) => {
                            for k in 0..b.len() as u32 {
                                if let Some(c) = covered.get_mut((off + k) as usize) {
                                    *c = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                for i in inits {
                    match i {
                        LocalInit::Zero => {
                            let mut k = 0;
                            while k < size {
                                if covered[k as usize] {
                                    k += 1;
                                    continue;
                                }
                                let start = k;
                                while k < size && !covered[k as usize] {
                                    k += 1;
                                }
                                self.emit(Inst::MemSet(mem_add(base, start as i32), Val::K(0), k - start));
                            }
                        }
                        LocalInit::Scalar(off, bits, e) => {
                            let v = self.rvalue(e)?;
                            let m = mem_add(base, *off as i32);
                            let ety = self.ty(&e.ty);
                            let lv = match bits {
                                Some((bo, w)) => LVal::Bits(m, *bo, *w, ety, false),
                                None => LVal::Mem(m, ety),
                            };
                            self.store(&lv, v);
                        }
                        LocalInit::Aggregate(off, e) => {
                            let src = self.agg_mem(e)?;
                            let n = self.prog.size(&e.ty);
                            self.emit(Inst::MemCopy(mem_add(base, *off as i32), src, n));
                        }
                        LocalInit::Bytes(off, b) => {
                            for (k, x) in b.iter().enumerate() {
                                self.emit(Inst::Store(mem_add(base, *off as i32 + k as i32), Val::K(*x as i64), Ty::I8));
                            }
                        }
                    }
                }
                Ok(())
            }
        }
    }
}

fn cond_of(op: BinOp, signed: bool) -> Cond {
    match (op, signed) {
        (BinOp::Eq, _) => Cond::Eq,
        (BinOp::Ne, _) => Cond::Ne,
        (BinOp::Lt, true) => Cond::LtS,
        (BinOp::Le, true) => Cond::LeS,
        (BinOp::Gt, true) => Cond::GtS,
        (BinOp::Ge, true) => Cond::GeS,
        (BinOp::Lt, false) => Cond::LtU,
        (BinOp::Le, false) => Cond::LeU,
        (BinOp::Gt, false) => Cond::GtU,
        (BinOp::Ge, false) => Cond::GeU,
        _ => unreachable!(),
    }
}
