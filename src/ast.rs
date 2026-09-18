//! Typed abstract syntax tree produced by the parser/semantic analyzer.

use crate::diag::Loc;
use crate::types::*;
use std::collections::HashMap;
use std::rc::Rc;

pub type GlobalId = usize;
pub type FuncId = usize;
pub type LocalId = usize;
pub type LabelId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    BitNot,
    LogNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    And,
    Or,
    Xor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LogAnd,
    LogOr,
}

impl BinOp {
    pub fn is_cmp(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Global(GlobalId),
    Local(LocalId),
    Func(FuncId),
    Unary(UnOp, Box<Expr>),
    /// Arithmetic on already-converted operands (both of the result type, except shifts and comparisons).
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Pointer + integer (integer already scaled to bytes and converted to int).
    PtrAdd(Box<Expr>, Box<Expr>),
    /// Pointer - pointer, divided by element size.
    PtrDiff(Box<Expr>, Box<Expr>, u32),
    Assign(Box<Expr>, Box<Expr>),
    /// `lhs op= rhs`: the operation is done in type `op_ty`, then converted back to lhs type.
    /// For pointer lhs, op is Add/Sub and rhs is scaled already.
    CompoundAssign(BinOp, Box<Expr>, Box<Expr>, Type),
    /// Pre/post increment: (lvalue, delta, is_post). For pointers delta is scaled.
    IncDec(Box<Expr>, i64, bool),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Comma(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Cast(Box<Expr>),
    Deref(Box<Expr>),
    AddrOf(Box<Expr>),
    /// Struct/union member access on an lvalue.
    Member(Box<Expr>, usize /* field index */),
    /// GNU statement expression.
    StmtExpr(Vec<Stmt>, Option<Box<Expr>>),
    /// Builtin with name, for things like __builtin_expect, memcpy intrinsics.
    Builtin(Builtin, Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    VaStart,
    VaArg,
    VaEnd,
    VaCopy,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
    pub loc: Loc,
}

impl Expr {
    pub fn new(kind: ExprKind, ty: Type, loc: Loc) -> Expr {
        Expr { kind, ty, loc }
    }
    pub fn int(v: i64, ty: Type, loc: Loc) -> Expr {
        Expr { kind: ExprKind::Int(v), ty, loc }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self.kind {
            ExprKind::Int(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Expr(Expr),
    Block(Vec<Stmt>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    /// switch(expr) body; cases: (value, label), default label.
    Switch(Expr, Box<Stmt>, Vec<(i64, LabelId)>, Option<LabelId>),
    Break,
    Continue,
    Return(Option<Expr>, Loc),
    Goto(LabelId, Loc),
    Label(LabelId, Box<Stmt>),
    /// Inline assembly.
    Asm(String, Loc),
    /// __critical { ... }
    Critical(Box<Stmt>),
    /// Initialize a local variable: sequence of (offset, value) assignments preceded by optional zeroing.
    InitLocal(LocalId, Vec<LocalInit>, Loc),
    Empty,
}

#[derive(Clone, Debug)]
pub enum LocalInit {
    /// Zero-fill the whole object.
    Zero,
    /// Assign `expr` (of scalar type) at byte offset, optionally into a bit-field.
    Scalar(u32, Option<(u8, u8)>, Expr),
    /// Copy an aggregate value (struct/union expression) at offset.
    Aggregate(u32, Expr),
    /// Copy bytes (e.g. from a string literal) at offset.
    Bytes(u32, Vec<u8>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    External,
    Internal,
}

/// A relocation inside initialized data.
#[derive(Clone, Debug, PartialEq)]
pub enum RelocTarget {
    Global(GlobalId),
    Func(FuncId),
}

#[derive(Clone, Debug)]
pub struct Reloc {
    pub offset: u32,
    pub target: RelocTarget,
    pub addend: i64,
    /// Size of the relocated field (1, 2 or 3 bytes for generic pointers).
    pub size: u8,
    /// Space variable of the pointer type (to compute generic pointer tags).
    pub space_var: Option<SpaceVar>,
}

#[derive(Clone, Debug, Default)]
pub struct InitData {
    pub bytes: Vec<u8>,
    pub relocs: Vec<Reloc>,
}

#[derive(Clone, Debug)]
pub struct Global {
    pub name: Rc<str>,
    pub ty: Type,
    pub linkage: Linkage,
    /// Storage space of the object.
    pub space: Space,
    pub init: Option<InitData>,
    pub defined: bool,
    /// Fixed address (`__at`).
    pub at: Option<u32>,
    pub loc: Loc,
    /// Address taken anywhere in the program.
    pub addr_taken: bool,
    /// Used anywhere (reachability computed later).
    pub is_string: bool,
    /// Tentative definition (no initializer).
    pub tentative: bool,
    pub is_extern_decl: bool,
    pub volatile: bool,
    pub tu: usize,
}

#[derive(Clone, Debug)]
pub struct Local {
    pub name: Rc<str>,
    pub ty: Type,
    pub addr_taken: bool,
    pub is_param: bool,
    pub loc: Loc,
    /// Explicit storage space qualifier.
    pub space: Option<Space>,
    pub register: bool,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub name: Rc<str>,
    pub ty: Type,
    pub linkage: Linkage,
    pub body: Option<Stmt>,
    pub locals: Vec<Local>,
    pub params: Vec<LocalId>,
    pub labels: Vec<Rc<str>>,
    pub loc: Loc,
    pub is_inline: bool,
    /// The body came from an inline definition only (C99 6.7.4: it provides no external symbol).
    pub inline_body: bool,
    /// A non-inline (external) definition exists.
    pub has_external_def: bool,
    pub addr_taken: bool,
    /// `#pragma nooverlay` was active.
    pub nooverlay: bool,
    pub tu: usize,
}

impl Function {
    pub fn ftype(&self) -> &Rc<FuncType> {
        match &self.ty.kind {
            TypeKind::Func(f) => f,
            _ => unreachable!(),
        }
    }
}

#[derive(Default)]
pub struct SpaceSolver {
    pub parent: Vec<u32>,
    pub spaces: Vec<Vec<Space>>,
    /// Final solution per root (filled by solve()).
    pub solved: HashMap<u32, Option<Space>>,
    /// Variables known (from a previous pass) to be generic.
    pub generic_hint: std::collections::HashSet<u32>,
    /// Variables of pointers declared with an explicit target space: never merged with other classes.
    pub pinned: Vec<bool>,
}

impl SpaceSolver {
    pub fn new_var(&mut self, s: Option<Space>) -> SpaceVar {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        self.spaces.push(s.into_iter().collect());
        self.pinned.push(false);
        id
    }
    pub fn is_pinned(&self, v: SpaceVar) -> bool {
        self.pinned[self.find_const(v) as usize]
    }
    /// A variable fixed to one space (explicitly declared).
    pub fn new_pinned(&mut self, s: Space) -> SpaceVar {
        let v = self.new_var(Some(s));
        self.pinned[v as usize] = true;
        v
    }
    pub fn find(&mut self, v: SpaceVar) -> SpaceVar {
        let mut r = v;
        while self.parent[r as usize] != r {
            r = self.parent[r as usize];
        }
        let mut x = v;
        while self.parent[x as usize] != r {
            let n = self.parent[x as usize];
            self.parent[x as usize] = r;
            x = n;
        }
        r
    }
    pub fn find_const(&self, v: SpaceVar) -> SpaceVar {
        let mut r = v;
        while self.parent[r as usize] != r {
            r = self.parent[r as usize];
        }
        r
    }
    pub fn union(&mut self, a: SpaceVar, b: SpaceVar) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return;
        }
        // A pinned class keeps its space; the other class must be able to hold it.
        match (self.pinned[ra as usize], self.pinned[rb as usize]) {
            (true, true) => return,
            (true, false) | (false, true) => {
                let (p, o) = if self.pinned[ra as usize] { (ra, rb) } else { (rb, ra) };
                for s in self.spaces[p as usize].clone() {
                    if !self.spaces[o as usize].contains(&s) {
                        self.spaces[o as usize].push(s);
                    }
                }
                return;
            }
            _ => {}
        }
        let sb = std::mem::take(&mut self.spaces[rb as usize]);
        for s in sb {
            if !self.spaces[ra as usize].contains(&s) {
                self.spaces[ra as usize].push(s);
            }
        }
        self.parent[rb as usize] = ra;
    }
    pub fn add_space(&mut self, v: SpaceVar, s: Space) {
        let r = self.find(v);
        let s = normalize_ptr_space(s);
        if self.pinned[r as usize] {
            return;
        }
        if !self.spaces[r as usize].contains(&s) {
            self.spaces[r as usize].push(s);
        }
    }
    /// The resolved target space of a pointer variable; None = generic pointer.
    pub fn resolve(&self, v: SpaceVar) -> Option<Space> {
        let r = self.find_const(v);
        let sp = &self.spaces[r as usize];
        match sp.len() {
            0 => Some(Space::Data),
            1 => Some(sp[0]),
            _ => {
                // Data and Idata are both reachable through @Ri.
                if sp.iter().all(|s| matches!(s, Space::Data | Space::Idata)) {
                    Some(Space::Idata)
                } else {
                    None
                }
            }
        }
    }
    pub fn is_generic(&self, v: SpaceVar) -> bool {
        if self.generic_hint.contains(&v) {
            return true;
        }
        self.resolve(v).is_none()
    }
}

pub fn normalize_ptr_space(s: Space) -> Space {
    match s {
        Space::Sfr | Space::Sbit | Space::Bit => Space::Data,
        s => s,
    }
}

pub struct Program {
    pub globals: Vec<Global>,
    pub funcs: Vec<Function>,
    pub records: Vec<RecordDef>,
    pub enums: Vec<EnumDef>,
    pub spaces: SpaceSolver,
    /// External-linkage symbol table: name -> global or function.
    pub externs: HashMap<Rc<str>, Sym>,
    pub string_pool: HashMap<(Vec<u8>, u8), GlobalId>,
    pub tu_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sym {
    Global(GlobalId),
    Func(FuncId),
}

impl Program {
    pub fn new() -> Program {
        Program {
            globals: Vec::new(),
            funcs: Vec::new(),
            records: Vec::new(),
            enums: Vec::new(),
            spaces: SpaceSolver::default(),
            externs: HashMap::new(),
            string_pool: HashMap::new(),
            tu_count: 0,
        }
    }

    pub fn ptr_size(&self, v: SpaceVar, pointee: &Type) -> u32 {
        if pointee.is_func() {
            return 2;
        }
        if self.spaces.is_generic(v) { 3 } else { 2 }
    }

    pub fn sizeof(&self, t: &Type) -> Option<u32> {
        Some(match &t.kind {
            TypeKind::Void => 1,
            TypeKind::Int(k, _) | TypeKind::Enum(_, k, _) => Type::int_size(*k),
            TypeKind::Float | TypeKind::Double => 4,
            TypeKind::Pointer(p, v) => self.ptr_size(*v, p),
            TypeKind::Array(e, n) => self.sizeof(e)? * (*n)?,
            TypeKind::Func(_) => 1,
            TypeKind::Record(r) => {
                let r = &self.records[*r];
                if !r.complete {
                    return None;
                }
                r.size
            }
        })
    }

    /// Type compatibility across translation units (records compared structurally).
    pub fn compatible(&self, a: &Type, b: &Type) -> bool {
        self.compat(a, b, 0)
    }

    fn compat(&self, a: &Type, b: &Type, depth: u32) -> bool {
        match (&a.kind, &b.kind) {
            (TypeKind::Record(x), TypeKind::Record(y)) => {
                if x == y {
                    return true;
                }
                let (rx, ry) = (&self.records[*x], &self.records[*y]);
                if rx.tag != ry.tag || rx.is_union != ry.is_union {
                    return false;
                }
                if !rx.complete || !ry.complete || depth > 0 {
                    return true;
                }
                rx.size == ry.size
                    && rx.fields.len() == ry.fields.len()
                    && rx.fields.iter().zip(&ry.fields).all(|(f, g)| {
                        f.name == g.name && f.offset == g.offset && f.bits == g.bits && self.compat(&f.ty, &g.ty, depth)
                    })
            }
            (TypeKind::Enum(_, k1, s1), TypeKind::Enum(_, k2, s2)) => k1 == k2 && s1 == s2,
            (TypeKind::Pointer(p, _), TypeKind::Pointer(q, _)) => self.compat(p, q, depth + 1),
            (TypeKind::Array(p, n), TypeKind::Array(q, m)) => (n == m || n.is_none() || m.is_none()) && self.compat(p, q, depth),
            (TypeKind::Func(f), TypeKind::Func(g)) => {
                self.compat(&f.ret, &g.ret, depth)
                    && (f.unprototyped
                        || g.unprototyped
                        || (f.params.len() == g.params.len()
                            && f.variadic == g.variadic
                            && f.params.iter().zip(&g.params).all(|(x, y)| self.compat(x, y, depth))))
            }
            _ => a.same(b),
        }
    }

    pub fn size(&self, t: &Type) -> u32 {
        self.sizeof(t).unwrap_or(0)
    }

    /// Storage size of a global (a flexible array member may extend past the type's size).
    pub fn global_size(&self, g: usize) -> u32 {
        let gl = &self.globals[g];
        self.size(&gl.ty).max(gl.init.as_ref().map_or(0, |i| i.bytes.len() as u32))
    }

    /// The pointer target space (None for generic pointers).
    pub fn ptr_space(&self, t: &Type) -> Option<Space> {
        match &t.kind {
            TypeKind::Pointer(p, v) => {
                if p.is_func() {
                    return Some(Space::Code);
                }
                if self.spaces.is_generic(*v) { None } else { self.spaces.resolve(*v) }
            }
            _ => None,
        }
    }
}

/// Visit every expression (pre-order) in a statement.
pub fn walk_stmt_exprs(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    match s {
        Stmt::Expr(e) => walk_expr(e, f),
        Stmt::Block(v) => v.iter().for_each(|x| walk_stmt_exprs(x, f)),
        Stmt::If(c, t, e) => {
            walk_expr(c, f);
            walk_stmt_exprs(t, f);
            if let Some(e) = e {
                walk_stmt_exprs(e, f);
            }
        }
        Stmt::While(c, b) => {
            walk_expr(c, f);
            walk_stmt_exprs(b, f);
        }
        Stmt::DoWhile(b, c) => {
            walk_stmt_exprs(b, f);
            walk_expr(c, f);
        }
        Stmt::For(i, c, st, b) => {
            if let Some(i) = i {
                walk_stmt_exprs(i, f);
            }
            if let Some(c) = c {
                walk_expr(c, f);
            }
            if let Some(st) = st {
                walk_expr(st, f);
            }
            walk_stmt_exprs(b, f);
        }
        Stmt::Switch(e, b, _, _) => {
            walk_expr(e, f);
            walk_stmt_exprs(b, f);
        }
        Stmt::Return(Some(e), _) => walk_expr(e, f),
        Stmt::Label(_, s) | Stmt::Critical(s) => walk_stmt_exprs(s, f),
        Stmt::InitLocal(_, inits, _) => {
            for i in inits {
                match i {
                    LocalInit::Scalar(_, _, e) | LocalInit::Aggregate(_, e) => walk_expr(e, f),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

pub fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Unary(_, a) | ExprKind::Cast(a) | ExprKind::Deref(a) | ExprKind::AddrOf(a) | ExprKind::Member(a, _) | ExprKind::IncDec(a, _, _) => walk_expr(a, f),
        ExprKind::Binary(_, a, b) | ExprKind::PtrAdd(a, b) | ExprKind::PtrDiff(a, b, _) | ExprKind::Assign(a, b) | ExprKind::CompoundAssign(_, a, b, _) | ExprKind::Comma(a, b) => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        ExprKind::Cond(a, b, c) => {
            walk_expr(a, f);
            walk_expr(b, f);
            walk_expr(c, f);
        }
        ExprKind::Call(c, args) => {
            walk_expr(c, f);
            args.iter().for_each(|a| walk_expr(a, f));
        }
        ExprKind::StmtExpr(stmts, v) => {
            stmts.iter().for_each(|s| walk_stmt_exprs(s, f));
            if let Some(v) = v {
                walk_expr(v, f);
            }
        }
        ExprKind::Builtin(_, args) => args.iter().for_each(|a| walk_expr(a, f)),
        _ => {}
    }
}

impl Program {
    /// Data pointers passed as variable arguments are generic unless their target space is explicit.
    pub fn is_generic_vararg_ptr(&self, t: &Type) -> bool {
        t.is_pointer() && !t.is_func_ptr() && t.space_var().map_or(false, |v| self.spaces.is_generic(v))
    }

    /// Does any call pass a floating-point variable argument?
    pub fn has_float_varargs(&self) -> bool {
        let mut found = false;
        for func in &self.funcs {
            let Some(body) = &func.body else { continue };
            walk_stmt_exprs(body, &mut |e| {
                if let ExprKind::Call(c, args) = &e.kind {
                    let fid = match &c.kind {
                        ExprKind::Func(x) => Some(*x),
                        ExprKind::AddrOf(inner) => match inner.kind {
                            ExprKind::Func(x) => Some(x),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(fid) = fid {
                        let ft = self.funcs[fid].ftype();
                        if ft.variadic && args.len() > ft.params.len() && args[ft.params.len()..].iter().any(|a| a.ty.is_float()) {
                            found = true;
                        }
                    }
                }
            });
        }
        found
    }

    /// Size of the variable-argument area needed by each variadic function.
    pub fn vararg_sizes(&self) -> Vec<u32> {
        let mut sizes = vec![0u32; self.funcs.len()];
        for func in &self.funcs {
            let Some(body) = &func.body else { continue };
            walk_stmt_exprs(body, &mut |e| {
                if let ExprKind::Call(c, args) = &e.kind {
                    let fid = match &c.kind {
                        ExprKind::Func(x) => Some(*x),
                        ExprKind::AddrOf(inner) => match inner.kind {
                            ExprKind::Func(x) => Some(x),
                            _ => None,
                        },
                        _ => None,
                    };
                    if let Some(fid) = fid {
                        let ft = self.funcs[fid].ftype();
                        if ft.variadic && args.len() > ft.params.len() {
                            let n: u32 = args[ft.params.len()..]
                                .iter()
                                .map(|a| if a.ty.is_array() || self.is_generic_vararg_ptr(&a.ty) { 3 } else if a.ty.is_bit() { 1 } else { self.size(&a.ty) })
                                .sum();
                            sizes[fid] = sizes[fid].max(n);
                        }
                    }
                }
            });
        }
        sizes
    }
}
