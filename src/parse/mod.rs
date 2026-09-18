//! Parser and semantic analysis. Produces a typed AST into a shared `Program`.

mod consteval;
mod expr;
mod init;
mod stmt;

pub use consteval::ConstVal;

use crate::ast::*;
use crate::diag::{self, Loc, Result, err, error};
use crate::lex::{Kw, Tok, Token};
use crate::types::*;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
enum Entry {
    Global(GlobalId),
    Func(FuncId),
    Local(LocalId),
    Typedef(Type),
    EnumConst(i64),
}

#[derive(Clone, Copy, Debug)]
enum Tag {
    Record(RecordId),
    Enum(EnumId, IntKind, bool),
}

#[derive(Default)]
struct Scope {
    names: HashMap<Rc<str>, Entry>,
    tags: HashMap<Rc<str>, Tag>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Storage {
    None,
    Typedef,
    Extern,
    Static,
    Auto,
    Register,
}

#[derive(Clone, Debug)]
struct DeclSpec {
    ty: Type,
    storage: Storage,
    inline: bool,
    noreturn: bool,
    at: Option<u32>,
    fattrs: FuncAttrs,
    /// __sfr / __sbit declarations.
    special: Option<Space>,
}

struct SwitchCtx {
    cases: Vec<(i64, LabelId)>,
    default: Option<LabelId>,
    ty: Type,
}

struct FuncCtx {
    id: FuncId,
    locals: Vec<Local>,
    labels: Vec<Rc<str>>,
    label_defined: Vec<bool>,
    label_map: HashMap<Rc<str>, LabelId>,
    label_uses: Vec<(LabelId, Loc)>,
    switches: Vec<SwitchCtx>,
    loop_depth: usize,
    break_depth: usize,
    ret_ty: Type,
}

pub struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    pub prog: &'a mut Program,
    tu: usize,
    scopes: Vec<Scope>,
    fctx: Option<FuncCtx>,
    nooverlay: bool,
    /// `__at` seen inside a declarator (applies to the declared object).
    pending_at: Option<u32>,
    /// ISO C mode (`#pragma std_cXX`): no SDCC-specific argument passing.
    iso_std: bool,
    pragma_stack: Vec<bool>,
    anon_counter: usize,
    /// Pending expression for initializer brace elision.
    pending_init: Option<Expr>,
    /// Suppress creation of objects (inside sizeof/typeof).
    in_sizeof: usize,
    /// Definitions in this unit yield to existing ones (library code).
    weak: bool,
    /// Nesting depth of a static initializer being parsed.
    static_init: u32,
}

pub fn parse_tu(toks: Vec<Token>, prog: &mut Program) -> Result<()> {
    parse_tu_ex(toks, prog, false)
}

pub fn parse_tu_ex(toks: Vec<Token>, prog: &mut Program, weak: bool) -> Result<()> {
    let tu = prog.tu_count;
    prog.tu_count += 1;
    let mut p = Parser {
        toks,
        pos: 0,
        prog,
        tu,
        scopes: vec![Scope::default()],
        fctx: None,
        nooverlay: false,
        pending_at: None,
        iso_std: false,
        pragma_stack: Vec::new(),
        anon_counter: 0,
        pending_init: None,
        in_sizeof: 0,
        weak,
        static_init: 0,
    };
    p.builtin_typedefs();
    while !p.at_eof() {
        p.top_level()?;
    }
    Ok(())
}

/// Keywords that only qualify a type (they can precede `*` in a declarator).
fn is_qual_kw(t: &Tok) -> bool {
    matches!(
        t,
        Tok::Kw(Kw::Const)
            | Tok::Kw(Kw::Volatile)
            | Tok::Kw(Kw::Restrict)
            | Tok::Kw(Kw::Data)
            | Tok::Kw(Kw::Near)
            | Tok::Kw(Kw::Idata)
            | Tok::Kw(Kw::Xdata)
            | Tok::Kw(Kw::Far)
            | Tok::Kw(Kw::Pdata)
            | Tok::Kw(Kw::Code)
    )
}

impl<'a> Parser<'a> {
    fn builtin_typedefs(&mut self) {
        let va = Type::new(TypeKind::Pointer(Rc::new(Type::uchar()), self.prog.spaces.new_var(None)));
        self.scopes[0].names.insert("__builtin_va_list".into(), Entry::Typedef(va));
    }

    // ------------------------------------------------------------------
    // Token helpers

    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i].tok
    }
    fn loc(&self) -> Loc {
        self.toks[self.pos].loc
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn next(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(x) if *x == p)
    }
    fn is_kw(&self, k: Kw) -> bool {
        matches!(self.peek(), Tok::Kw(x) if *x == k)
    }
    fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: Kw) -> bool {
        if self.is_kw(k) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect_p(&mut self, p: &str) -> Result<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            err(self.loc(), format!("expected '{}' but found {}", p, self.describe()))
        }
    }
    fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("'{}'", s),
            Tok::Kw(k) => format!("'{}'", k.as_str()),
            Tok::Int(v, _) => format!("'{}'", v),
            Tok::Float(v, _) => format!("'{}'", v),
            Tok::Str(..) => "string literal".into(),
            Tok::Punct(p) => format!("'{}'", p),
            Tok::Asm(_) => "assembly block".into(),
            Tok::Pragma(p) => format!("'#pragma {}'", p),
            Tok::Eof => "end of file".into(),
        }
    }
    fn expect_ident(&mut self) -> Result<Rc<str>> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.pos += 1;
                Ok(s)
            }
            _ => err(self.loc(), format!("expected identifier but found {}", self.describe())),
        }
    }

    // ------------------------------------------------------------------
    // Scopes

    fn lookup(&self, name: &str) -> Option<&Entry> {
        for s in self.scopes.iter().rev() {
            if let Some(e) = s.names.get(name) {
                return Some(e);
            }
        }
        None
    }
    fn lookup_tag(&self, name: &str) -> Option<Tag> {
        for s in self.scopes.iter().rev() {
            if let Some(t) = s.tags.get(name) {
                return Some(*t);
            }
        }
        None
    }
    fn push_scope(&mut self) {
        self.scopes.push(Scope::default());
    }
    fn pop_scope(&mut self) {
        self.scopes.pop();
    }
    fn declare(&mut self, name: Rc<str>, e: Entry) {
        self.scopes.last_mut().unwrap().names.insert(name, e);
    }

    fn is_typename_tok(&self, t: &Tok) -> bool {
        match t {
            Tok::Kw(k) => matches!(
                k,
                Kw::Void
                    | Kw::Char
                    | Kw::Short
                    | Kw::Int
                    | Kw::Long
                    | Kw::Float
                    | Kw::Double
                    | Kw::Signed
                    | Kw::Unsigned
                    | Kw::Bool
                    | Kw::Struct
                    | Kw::Union
                    | Kw::Enum
                    | Kw::Const
                    | Kw::Volatile
                    | Kw::Restrict
                    | Kw::Typedef
                    | Kw::Extern
                    | Kw::Static
                    | Kw::Auto
                    | Kw::Register
                    | Kw::Inline
                    | Kw::Noreturn
                    | Kw::Alignas
                    | Kw::Atomic
                    | Kw::Complex
                    | Kw::ThreadLocal
                    | Kw::Typeof
                    | Kw::Attribute
                    | Kw::Extension
                    | Kw::Data
                    | Kw::Idata
                    | Kw::Xdata
                    | Kw::Pdata
                    | Kw::Code
                    | Kw::Bit
                    | Kw::Sbit
                    | Kw::Sfr
                    | Kw::Sfr16
                    | Kw::Sfr32
                    | Kw::At
                    | Kw::Near
                    | Kw::Far
                    | Kw::Banked
                    | Kw::Nonbanked
            ),
            Tok::Ident(n) => matches!(self.lookup(n), Some(Entry::Typedef(_))),
            _ => false,
        }
    }
    fn is_typename(&self) -> bool {
        self.is_typename_tok(self.peek())
    }

    // ------------------------------------------------------------------
    // Space inference helpers

    fn ptr_to(&mut self, t: Type) -> Type {
        let v = match t.q.space.map(normalize_ptr_space) {
            Some(s) => self.prog.spaces.new_pinned(s),
            None => self.prog.spaces.new_var(None),
        };
        Type::new(TypeKind::Pointer(Rc::new(t), v))
    }
    fn ptr_to_in(&mut self, t: Type, space: Space) -> Type {
        let v = self.prog.spaces.new_var(Some(normalize_ptr_space(space)));
        if let Some(s) = t.q.space {
            self.prog.spaces.add_space(v, s);
        }
        Type::new(TypeKind::Pointer(Rc::new(t), v))
    }
    fn unify(&mut self, a: &Type, b: &Type) {
        match (&a.kind, &b.kind) {
            (TypeKind::Pointer(pa, va), TypeKind::Pointer(pb, vb)) => {
                if !pa.is_func() && !pb.is_func() {
                    self.prog.spaces.union(*va, *vb);
                }
                let (pa, pb) = (pa.clone(), pb.clone());
                self.unify(&pa, &pb);
            }
            (TypeKind::Array(ea, _), TypeKind::Array(eb, _)) => {
                let (ea, eb) = (ea.clone(), eb.clone());
                self.unify(&ea, &eb);
            }
            (TypeKind::Func(fa), TypeKind::Func(fb)) => {
                let (fa, fb) = (fa.clone(), fb.clone());
                self.unify(&fa.ret, &fb.ret);
                for (x, y) in fa.params.iter().zip(fb.params.iter()) {
                    self.unify(x, y);
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------
    // Top level

    fn handle_pragma(&mut self, text: &str) {
        let words: Vec<&str> = text.split_whitespace().collect();
        match words.first().copied() {
            Some("save") => self.pragma_stack.push(self.nooverlay),
            Some("restore") => {
                if let Some(v) = self.pragma_stack.pop() {
                    self.nooverlay = v;
                }
            }
            Some("nooverlay") => self.nooverlay = true,
            Some(w) if w.starts_with("std_sdcc") => self.iso_std = false,
            Some(w) if w.starts_with("std_c") => self.iso_std = true,
            _ => {}
        }
    }

    fn top_level(&mut self) -> Result<()> {
        if let Tok::Pragma(p) = self.peek().clone() {
            self.pos += 1;
            self.handle_pragma(&p);
            return Ok(());
        }
        if self.eat_p(";") {
            return Ok(());
        }
        if self.is_kw(Kw::StaticAssert) {
            return self.static_assert();
        }
        if self.is_kw(Kw::AsmBegin) {
            // File-scope assembly block.
            let loc = self.loc();
            self.pos += 1;
            let text = match self.next().tok {
                Tok::Asm(s) => s,
                _ => return err(loc, "expected assembly block"),
            };
            self.eat_kw(Kw::AsmEnd);
            self.eat_p(";");
            self.add_global_asm(text, loc);
            return Ok(());
        }
        let loc = self.loc();
        let spec = if self.is_typename() {
            self.decl_spec()?
        } else {
            // Implicit int (old style).
            if !matches!(self.peek(), Tok::Ident(_)) {
                return err(loc, format!("expected declaration but found {}", self.describe()));
            }
            diag::warn(loc, "type specifier missing, defaults to 'int'");
            DeclSpec { ty: Type::int(), storage: Storage::None, inline: false, noreturn: false, at: None, fattrs: FuncAttrs::default(), special: None }
        };
        if self.eat_p(";") {
            return Ok(());
        }
        let mut first = true;
        loop {
            let (ty, name, dloc, fattrs, param_names) = self.declarator_full(spec.ty.clone())?;
            let mut spec2 = spec.clone();
            self.post_declarator_attrs(&mut spec2)?;
            let Some(name) = name else { return err(dloc, "declarator requires an identifier") };
            let mut fattrs = merge_attrs(&spec2.fattrs, &fattrs);
            if spec2.noreturn {
                fattrs.noreturn = true;
            }
            if spec2.storage == Storage::Typedef {
                self.declare(name, Entry::Typedef(ty));
            } else if ty.is_func() {
                let ty = with_fattrs(&ty, fattrs);
                if first && (self.is_p("{") || (self.is_typename() && !self.is_p(";"))) {
                    // Function definition (possibly K&R style).
                    return self.function_def(name, ty, &spec2, dloc, param_names);
                }
                self.declare_func(name, ty, &spec2, dloc, false)?;
            } else {
                self.global_decl(name, ty, &spec2, dloc)?;
            }
            first = false;
            if self.eat_p(",") {
                continue;
            }
            self.expect_p(";")?;
            return Ok(());
        }
    }

    fn add_global_asm(&mut self, text: String, loc: Loc) {
        // Represent file-scope asm as a naked, always-kept pseudo function.
        let name: Rc<str> = format!("__asm_block_{}_{}", self.tu, self.anon_counter).into();
        self.anon_counter += 1;
        let ft = FuncType {
            ret: Type::void(),
            params: vec![],
            variadic: false,
            unprototyped: false,
            attrs: FuncAttrs { naked: true, ..Default::default() },
        };
        let f = Function {
            name,
            ty: Type::new(TypeKind::Func(Rc::new(ft))),
            linkage: Linkage::Internal,
            body: Some(Stmt::Asm(text, loc)),
            locals: vec![],
            params: vec![],
            labels: vec![],
            loc,
            is_inline: false,
            inline_body: false,
            has_external_def: true,
            addr_taken: true,
            nooverlay: false,
            tu: self.tu,
        };
        self.prog.funcs.push(f);
    }

    fn static_assert(&mut self) -> Result<()> {
        let loc = self.loc();
        self.pos += 1;
        self.expect_p("(")?;
        let v = self.const_int_expr()?;
        let mut msg = String::new();
        if self.eat_p(",") {
            if let Tok::Str(s, _) = self.next().tok {
                msg = String::from_utf8_lossy(&s).into_owned();
            }
        }
        self.expect_p(")")?;
        self.expect_p(";")?;
        if v == 0 {
            return err(loc, format!("static assertion failed: {}", msg));
        }
        Ok(())
    }

    /// Parse SDCC function attributes and `__at` after a declarator.
    fn post_declarator_attrs(&mut self, spec: &mut DeclSpec) -> Result<()> {
        if let Some(a) = self.pending_at.take() {
            spec.at = Some(a);
        }
        loop {
            match self.peek() {
                Tok::Kw(Kw::At) => {
                    self.pos += 1;
                    spec.at = Some(self.at_address()?);
                }
                Tok::Kw(Kw::Attribute) => self.skip_attribute(Some(&mut spec.fattrs))?,
                Tok::Kw(Kw::AsmBegin) if false => {}
                Tok::Kw(Kw::AsmFn) => {
                    // `int x __asm__("name")` : assembler name, ignore.
                    self.pos += 1;
                    self.expect_p("(")?;
                    while !self.is_p(")") && !self.at_eof() {
                        self.pos += 1;
                    }
                    self.expect_p(")")?;
                }
                _ => {
                    if !self.func_attr(&mut spec.fattrs)? {
                        return Ok(());
                    }
                }
            }
        }
    }

    fn at_address(&mut self) -> Result<u32> {
        if self.eat_p("(") {
            let v = self.const_int_expr()?;
            self.expect_p(")")?;
            Ok(v as u32)
        } else {
            // `__at 0x1234`
            let e = self.primary_const()?;
            Ok(e as u32)
        }
    }

    fn primary_const(&mut self) -> Result<i64> {
        let loc = self.loc();
        match self.next().tok {
            Tok::Int(v, _) => Ok(v as i64),
            _ => err(loc, "expected constant"),
        }
    }

    /// Parse one function attribute keyword if present.
    fn func_attr(&mut self, a: &mut FuncAttrs) -> Result<bool> {
        let Tok::Kw(k) = *self.peek() else { return Ok(false) };
        match k {
            Kw::Interrupt => {
                self.pos += 1;
                if self.eat_p("(") {
                    let v = self.const_int_expr()?;
                    self.expect_p(")")?;
                    a.interrupt = Some(v as u8);
                } else if let Tok::Int(v, _) = *self.peek() {
                    self.pos += 1;
                    a.interrupt = Some(v as u8);
                } else {
                    a.interrupt = Some(255);
                }
            }
            Kw::Using => {
                self.pos += 1;
                if self.eat_p("(") {
                    let v = self.const_int_expr()?;
                    self.expect_p(")")?;
                    a.using = Some(v as u8);
                } else {
                    let v = self.primary_const()?;
                    a.using = Some(v as u8);
                }
            }
            Kw::Naked => {
                self.pos += 1;
                a.naked = true;
            }
            Kw::Critical => {
                self.pos += 1;
                a.critical = true;
            }
            Kw::Reentrant => {
                self.pos += 1;
                a.reentrant = true;
            }
            Kw::PreservesRegs => {
                self.pos += 1;
                self.expect_p("(")?;
                let mut regs = Vec::new();
                while !self.is_p(")") && !self.at_eof() {
                    if let Tok::Ident(s) = self.next().tok {
                        regs.push(s.to_string());
                    }
                    self.eat_p(",");
                }
                self.expect_p(")")?;
                a.preserves = Some(regs);
            }
            Kw::Banked | Kw::Nonbanked | Kw::Wparam | Kw::Shadowregs | Kw::Smallc | Kw::Fastcall | Kw::Callee | Kw::Raisonance | Kw::Iar | Kw::Cosmic | Kw::Hightide => {
                self.pos += 1;
            }
            Kw::Trap => {
                self.pos += 1;
            }
            Kw::Sdcccall => {
                self.pos += 1;
                self.expect_p("(")?;
                self.const_int_expr()?;
                self.expect_p(")")?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    fn skip_attribute(&mut self, mut attrs: Option<&mut FuncAttrs>) -> Result<()> {
        // __attribute__((...))
        self.pos += 1;
        self.expect_p("(")?;
        self.expect_p("(")?;
        let mut depth = 0;
        loop {
            if self.at_eof() {
                return err(self.loc(), "unterminated __attribute__");
            }
            if self.is_p("(") {
                depth += 1;
            } else if self.is_p(")") {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            } else if let Tok::Ident(s) = self.peek() {
                if depth == 0 && (&**s == "noreturn" || &**s == "__noreturn__") {
                    if let Some(a) = attrs.as_deref_mut() {
                        a.noreturn = true;
                    }
                }
            }
            self.pos += 1;
        }
        self.expect_p(")")?;
        self.expect_p(")")?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Declaration specifiers

    fn decl_spec(&mut self) -> Result<DeclSpec> {
        let loc = self.loc();
        let mut storage = Storage::None;
        let mut q = Quals::default();
        let (mut inline, mut noreturn) = (false, false);
        let mut at = None;
        let mut fattrs = FuncAttrs::default();
        let mut special = None;
        // `_BitInt(N)` without signed/unsigned is signed, even where plain char is not.
        let mut bitint = false;
        #[derive(Default)]
        struct C {
            void: u8,
            bool_: u8,
            bit: u8,
            char_: u8,
            short: u8,
            int: u8,
            long: u8,
            signed: u8,
            unsigned: u8,
            float: u8,
            double: u8,
            other: u8,
        }
        let mut c = C::default();
        let mut explicit: Option<Type> = None;
        loop {
            let t = self.peek().clone();
            match t {
                Tok::Kw(k) => {
                    match k {
                        Kw::Typedef | Kw::Extern | Kw::Static | Kw::Auto | Kw::Register => {
                            let s = match k {
                                Kw::Typedef => Storage::Typedef,
                                Kw::Extern => Storage::Extern,
                                Kw::Static => Storage::Static,
                                Kw::Auto => Storage::Auto,
                                _ => Storage::Register,
                            };
                            if storage != Storage::None && !(storage == s) {
                                return err(self.loc(), "multiple storage classes in declaration specifiers");
                            }
                            storage = s;
                        }
                        Kw::ThreadLocal => {}
                        Kw::Inline => inline = true,
                        Kw::Noreturn => noreturn = true,
                        Kw::Const => q.is_const = true,
                        Kw::Volatile => q.is_volatile = true,
                        Kw::Restrict | Kw::Complex => {}
                        Kw::Atomic => {
                            if matches!(self.peek_at(1), Tok::Punct("(")) {
                                self.pos += 2;
                                explicit = Some(self.type_name()?);
                                c.other += 1;
                                if !self.is_p(")") {
                                    return err(self.loc(), "expected ')'");
                                }
                            }
                        }
                        Kw::Extension => {}
                        Kw::Attribute => {
                            self.skip_attribute(Some(&mut fattrs))?;
                            continue;
                        }
                        Kw::Alignas => {
                            self.pos += 1;
                            self.expect_p("(")?;
                            if self.is_typename() {
                                self.type_name()?;
                            } else {
                                self.const_int_expr()?;
                            }
                            self.expect_p(")")?;
                            continue;
                        }
                        Kw::Data => q.space = Some(Space::Data),
                        Kw::Near => q.space = Some(Space::Data),
                        Kw::Far => q.space = Some(Space::Xdata),
                        Kw::Idata => q.space = Some(Space::Idata),
                        Kw::Xdata => q.space = Some(Space::Xdata),
                        Kw::Pdata => q.space = Some(Space::Pdata),
                        Kw::Code => q.space = Some(Space::Code),
                        Kw::At => {
                            self.pos += 1;
                            at = Some(self.at_address()?);
                            continue;
                        }
                        Kw::Sfr | Kw::Sfr16 | Kw::Sfr32 => {
                            special = Some(Space::Sfr);
                            c.other += 1;
                            explicit = Some(match k {
                                Kw::Sfr => Type::uchar(),
                                Kw::Sfr16 => Type::uint(),
                                _ => Type::ulong(),
                            });
                        }
                        Kw::Sbit => {
                            special = Some(Space::Sbit);
                            c.other += 1;
                            explicit = Some(Type::bit());
                        }
                        Kw::BitInt => {
                            // A bit-precise integer uses the smallest standard type that holds it.
                            self.pos += 1;
                            self.expect_p("(")?;
                            let w = self.const_int_expr()?;
                            self.expect_p(")")?;
                            if w < 1 || w > 64 {
                                return err(self.loc(), "unsupported _BitInt width");
                            }
                            match w {
                                1..=8 => c.char_ += 1,
                                9..=16 => c.int += 1,
                                17..=32 => c.long += 1,
                                _ => c.long += 2,
                            }
                            bitint = true;
                            continue;
                        }
                        Kw::Bit => c.bit += 1,
                        Kw::Void => c.void += 1,
                        Kw::Bool => c.bool_ += 1,
                        Kw::Char => c.char_ += 1,
                        Kw::Short => c.short += 1,
                        Kw::Int => c.int += 1,
                        Kw::Long => c.long += 1,
                        Kw::Signed => c.signed += 1,
                        Kw::Unsigned => c.unsigned += 1,
                        Kw::Float => c.float += 1,
                        Kw::Double => c.double += 1,
                        Kw::Struct | Kw::Union => {
                            if c.other > 0 {
                                return err(self.loc(), "two or more data types in declaration specifiers");
                            }
                            self.pos += 1;
                            explicit = Some(self.record_spec(k == Kw::Union)?);
                            c.other += 1;
                            continue;
                        }
                        Kw::Enum => {
                            if c.other > 0 {
                                return err(self.loc(), "two or more data types in declaration specifiers");
                            }
                            self.pos += 1;
                            explicit = Some(self.enum_spec()?);
                            c.other += 1;
                            continue;
                        }
                        Kw::Typeof => {
                            self.pos += 1;
                            self.expect_p("(")?;
                            let t = if self.is_typename() {
                                self.type_name()?
                            } else {
                                self.in_sizeof += 1;
                                let e = self.expr();
                                self.in_sizeof -= 1;
                                e?.ty
                            };
                            self.expect_p(")")?;
                            explicit = Some(t);
                            c.other += 1;
                            continue;
                        }
                        Kw::Interrupt | Kw::Using | Kw::Naked | Kw::Critical | Kw::Reentrant | Kw::Banked | Kw::Nonbanked => {
                            self.func_attr(&mut fattrs)?;
                            continue;
                        }
                        _ => break,
                    }
                    self.pos += 1;
                }
                Tok::Ident(ref n) => {
                    if c.other > 0 || c.void + c.bool_ + c.bit + c.char_ + c.short + c.int + c.long + c.signed + c.unsigned + c.float + c.double > 0 {
                        break;
                    }
                    match self.lookup(n) {
                        Some(Entry::Typedef(t)) => {
                            explicit = Some(t.clone());
                            c.other += 1;
                            self.pos += 1;
                        }
                        _ => break,
                    }
                }
                _ => break,
            }
        }
        let signed = c.signed > 0 || (bitint && c.unsigned == 0);
        let unsigned = c.unsigned > 0;
        if signed && unsigned {
            return err(loc, "both 'signed' and 'unsigned' in declaration specifiers");
        }
        let base = if let Some(t) = explicit {
            if c.void + c.bool_ + c.bit + c.char_ + c.short + c.int + c.long + c.float + c.double + c.signed + c.unsigned > 0 {
                return err(loc, "two or more data types in declaration specifiers");
            }
            t
        } else if c.void > 0 {
            Type::void()
        } else if c.bool_ > 0 {
            Type::bool_()
        } else if c.bit > 0 {
            Type::bit()
        } else if c.float > 0 {
            Type::new(TypeKind::Float)
        } else if c.double > 0 {
            Type::new(TypeKind::Double)
        } else if c.char_ > 0 {
            if signed {
                Type::schar()
            } else {
                Type::uchar()
            }
        } else if c.short > 0 {
            Type::intk(IntKind::Short, !unsigned)
        } else if c.long >= 2 {
            Type::intk(IntKind::LongLong, !unsigned)
        } else if c.long == 1 {
            Type::intk(IntKind::Long, !unsigned)
        } else if c.int > 0 || signed || unsigned {
            Type::intk(IntKind::Int, !unsigned)
        } else {
            if q == Quals::default() && storage == Storage::None && !inline && at.is_none() {
                return err(loc, format!("expected type specifier but found {}", self.describe()));
            }
            diag::warn(loc, "type specifier missing, defaults to 'int'");
            Type::int()
        };
        let mut ty = base;
        // Merge qualifiers (typedef may already carry some).
        ty.q.is_const |= q.is_const;
        ty.q.is_volatile |= q.is_volatile;
        if q.space.is_some() {
            ty.q.space = q.space;
        }
        if let Some(sp) = special {
            ty.q.space = Some(sp);
        }
        Ok(DeclSpec { ty, storage, inline, noreturn, at, fattrs, special })
    }

    fn record_spec(&mut self, is_union: bool) -> Result<Type> {
        while self.is_kw(Kw::Attribute) {
            self.skip_attribute(None)?;
        }
        let loc = self.loc();
        let tag = if let Tok::Ident(n) = self.peek().clone() {
            self.pos += 1;
            Some(n)
        } else {
            None
        };
        let id = if let Some(tag) = &tag {
            let existing = if self.is_p("{") || self.is_p(";") {
                // Definition or forward declaration: only the current scope matters.
                self.scopes.last().unwrap().tags.get(tag).copied()
            } else {
                self.lookup_tag(tag)
            };
            match existing {
                Some(Tag::Record(id)) => {
                    if self.prog.records[id].is_union != is_union {
                        return err(loc, format!("'{}' defined as wrong kind of tag", tag));
                    }
                    if self.is_p("{") && self.prog.records[id].complete {
                        return err(loc, format!("redefinition of '{}'", tag));
                    }
                    id
                }
                Some(Tag::Enum(..)) => return err(loc, format!("'{}' defined as wrong kind of tag", tag)),
                None => {
                    let id = self.prog.records.len();
                    self.prog.records.push(RecordDef { tag: Some(tag.clone()), is_union, fields: vec![], size: 0, complete: false });
                    self.scopes.last_mut().unwrap().tags.insert(tag.clone(), Tag::Record(id));
                    id
                }
            }
        } else {
            let id = self.prog.records.len();
            self.prog.records.push(RecordDef { tag: None, is_union, fields: vec![], size: 0, complete: false });
            id
        };
        if !self.eat_p("{") {
            if tag.is_none() {
                return err(loc, "expected '{' after anonymous struct/union");
            }
            return Ok(Type::new(TypeKind::Record(id)));
        }
        let mut fields: Vec<Field> = Vec::new();
        while !self.eat_p("}") {
            if let Tok::Pragma(p) = self.peek().clone() {
                self.pos += 1;
                self.handle_pragma(&p);
                continue;
            }
            if self.is_kw(Kw::StaticAssert) {
                self.static_assert()?;
                continue;
            }
            if self.eat_p(";") {
                continue;
            }
            let spec = self.decl_spec()?;
            if self.eat_p(";") {
                // Anonymous struct/union member.
                if let TypeKind::Record(_) = spec.ty.kind {
                    fields.push(Field { name: None, ty: spec.ty, offset: 0, bits: None });
                }
                continue;
            }
            loop {
                let (ty, name, dloc) = if self.is_p(":") {
                    (spec.ty.clone(), None, self.loc())
                } else {
                    self.declarator(spec.ty.clone())?
                };
                let mut bits = None;
                if self.eat_p(":") {
                    let w = self.const_int_expr()?;
                    if !ty.is_integer() {
                        return err(dloc, "bit-field has non-integral type");
                    }
                    let max = int_bits(ty.int_kind().unwrap().0) as i64;
                    if w < 0 || w > max {
                        return err(dloc, "invalid bit-field width");
                    }
                    bits = Some(w as u8);
                }
                while self.is_kw(Kw::Attribute) {
                    self.skip_attribute(None)?;
                }
                if let Some(w) = bits {
                    fields.push(Field { name, ty, offset: 0, bits: Some((0, w)) });
                } else {
                    if self.prog.sizeof(&ty).is_none() && !matches!(ty.kind, TypeKind::Array(_, None)) {
                        return err(dloc, "field has incomplete type");
                    }
                    fields.push(Field { name, ty, offset: 0, bits: None });
                }
                if self.eat_p(",") {
                    continue;
                }
                self.expect_p(";")?;
                break;
            }
        }
        while self.is_kw(Kw::Attribute) {
            self.skip_attribute(None)?;
        }
        // Layout (no padding on the 8051).
        let mut off: u32 = 0;
        let mut size: u32 = 0;
        // Current bit-field unit: (offset, bits used, unit size in bytes)
        // Current bit-field unit: (byte offset, bits used).
        let mut bit_unit: Option<(u32, u32)> = None;
        for f in fields.iter_mut() {
            if is_union {
                f.offset = 0;
                if let Some((_, w)) = f.bits {
                    f.bits = Some((0, w));
                    size = size.max((w as u32 + 7) / 8);
                } else {
                    size = size.max(self.prog.sizeof(&f.ty).unwrap_or(0));
                }
                continue;
            }
            if let Some((_, w)) = f.bits {
                let w = w as u32;
                // SDCC-style packing: bit-fields are packed into bytes, LSB first; a field that does not
                // fit in the current (partially used) byte starts a new one, as does any field of 8 bits or more.
                match bit_unit {
                    Some((uo, used)) if w > 0 && w < 8 && used % 8 != 0 && used % 8 + w <= 8 => {
                        f.offset = uo + used / 8;
                        f.bits = Some(((used % 8) as u8, w as u8));
                        bit_unit = Some((uo, used + w));
                    }
                    _ => {
                        if let Some((uo, used)) = bit_unit.take() {
                            off = uo + (used + 7) / 8;
                        }
                        f.offset = off;
                        f.bits = Some((0, w as u8));
                        if w > 0 {
                            bit_unit = Some((off, w));
                        }
                    }
                }
                if let Some((uo, used)) = bit_unit {
                    size = size.max(uo + (used + 7) / 8);
                }
                continue;
            }
            if let Some((uo, used)) = bit_unit.take() {
                off = uo + (used + 7) / 8;
            }
            f.offset = off;
            off += self.prog.sizeof(&f.ty).unwrap_or(0);
            size = size.max(off);
        }
        if let Some((uo, used)) = bit_unit {
            size = size.max(uo + (used + 7) / 8);
        }
        let fresh = id + 1 == self.prog.records.len();
        {
            let r = &mut self.prog.records[id];
            r.fields = fields;
            r.size = size;
            r.complete = true;
        }
        if fresh {
            // Merge with an identical definition from another translation unit.
            let t = Type::new(TypeKind::Record(id));
            for other in 0..id {
                let o = &self.prog.records[other];
                if o.complete && o.tag == self.prog.records[id].tag && self.prog.compatible(&Type::new(TypeKind::Record(other)), &t) {
                    if let Some(tag) = &tag {
                        self.scopes.last_mut().unwrap().tags.insert(tag.clone(), Tag::Record(other));
                    }
                    self.prog.records.pop();
                    return Ok(Type::new(TypeKind::Record(other)));
                }
            }
        }
        Ok(Type::new(TypeKind::Record(id)))
    }

    fn enum_spec(&mut self) -> Result<Type> {
        while self.is_kw(Kw::Attribute) {
            self.skip_attribute(None)?;
        }
        let loc = self.loc();
        let tag = if let Tok::Ident(n) = self.peek().clone() {
            self.pos += 1;
            Some(n)
        } else {
            None
        };
        // Optional fixed underlying type (C23).
        let mut fixed: Option<Type> = None;
        if self.is_p(":") && tag.is_some() || (tag.is_none() && self.is_p(":")) {
            self.pos += 1;
            fixed = Some(self.type_name()?);
        }
        if !self.is_p("{") {
            let Some(tag) = tag else { return err(loc, "expected '{' after enum") };
            return match self.lookup_tag(&tag) {
                Some(Tag::Enum(id, k, s)) => Ok(Type::new(TypeKind::Enum(id, k, s))),
                Some(_) => err(loc, format!("'{}' defined as wrong kind of tag", tag)),
                None => {
                    // Forward reference to an enum: assume int-sized until defined.
                    let id = self.prog.enums.len();
                    self.prog.enums.push(EnumDef { tag: Some(tag.clone()), complete: false });
                    let t = Tag::Enum(id, IntKind::Int, true);
                    self.scopes.last_mut().unwrap().tags.insert(tag, t);
                    Ok(Type::new(TypeKind::Enum(id, IntKind::Int, true)))
                }
            };
        }
        self.pos += 1;
        let id = self.prog.enums.len();
        self.prog.enums.push(EnumDef { tag: tag.clone(), complete: false });
        let mut val: i64 = 0;
        let (mut min, mut max) = (0i64, 0i64);
        let mut names = Vec::new();
        while !self.eat_p("}") {
            let n = self.expect_ident()?;
            while self.is_kw(Kw::Attribute) {
                self.skip_attribute(None)?;
            }
            if self.eat_p("=") {
                val = self.const_int_expr()?;
            }
            names.push(n.clone());
            self.declare(n, Entry::EnumConst(val));
            min = min.min(val);
            max = max.max(val);
            val = val.wrapping_add(1);
            if !self.eat_p(",") {
                self.expect_p("}")?;
                break;
            }
        }
        // Choose the underlying type (SDCC uses the smallest type that fits).
        let (k, s) = if let Some(f) = fixed {
            f.int_kind().unwrap_or((IntKind::Int, true))
        } else if min >= 0 && max <= 255 {
            (IntKind::Char, false)
        } else if min >= -128 && max <= 127 {
            (IntKind::Char, true)
        } else if min >= -32768 && max <= 32767 {
            (IntKind::Int, true)
        } else if min >= 0 && max <= 65535 {
            (IntKind::Int, false)
        } else if min >= i32::MIN as i64 && max <= i32::MAX as i64 {
            (IntKind::Long, true)
        } else if min >= 0 && max <= u32::MAX as i64 {
            (IntKind::Long, false)
        } else if min >= 0 {
            (IntKind::LongLong, false)
        } else {
            (IntKind::LongLong, true)
        };
        self.prog.enums[id].complete = true;
        if let Some(tag) = tag {
            self.scopes.last_mut().unwrap().tags.insert(tag, Tag::Enum(id, k, s));
        }
        Ok(Type::new(TypeKind::Enum(id, k, s)))
    }

    // ------------------------------------------------------------------
    // Declarators

    fn pointer_quals(&mut self) -> Result<Quals> {
        let mut q = Quals::default();
        loop {
            match self.peek() {
                Tok::Kw(Kw::Const) => q.is_const = true,
                Tok::Kw(Kw::Volatile) => q.is_volatile = true,
                Tok::Kw(Kw::Restrict) | Tok::Kw(Kw::Atomic) => {}
                Tok::Kw(Kw::Data) | Tok::Kw(Kw::Near) => q.space = Some(Space::Data),
                Tok::Kw(Kw::Idata) => q.space = Some(Space::Idata),
                Tok::Kw(Kw::Xdata) | Tok::Kw(Kw::Far) => q.space = Some(Space::Xdata),
                Tok::Kw(Kw::Pdata) => q.space = Some(Space::Pdata),
                Tok::Kw(Kw::Code) => q.space = Some(Space::Code),
                Tok::Kw(Kw::Attribute) => {
                    self.skip_attribute(None)?;
                    continue;
                }
                _ => return Ok(q),
            }
            self.pos += 1;
        }
    }

    fn declarator(&mut self, base: Type) -> Result<(Type, Option<Rc<str>>, Loc)> {
        let (t, n, l, _, _) = self.declarator_full(base)?;
        Ok((t, n, l))
    }

    /// Returns (type, name, loc, function attributes, parameter names of the outermost function declarator).
    fn declarator_full(&mut self, mut base: Type) -> Result<(Type, Option<Rc<str>>, Loc, FuncAttrs, Option<Vec<(Option<Rc<str>>, Type, Loc)>>)> {
        loop {
            // SDCC also allows the qualifiers before the star: `char (__code * p)`.
            let save = self.pos;
            let pre = if is_qual_kw(self.peek()) { self.pointer_quals()? } else { Quals::default() };
            if !self.eat_p("*") {
                self.pos = save;
                break;
            }
            let mut q = self.pointer_quals()?;
            q.is_const |= pre.is_const;
            q.is_volatile |= pre.is_volatile;
            q.space = q.space.or(pre.space);
            base = self.ptr_to(base);
            base.q = q;
            // `char * __code __at(0x1234) p`: the address belongs to the declared object.
            if self.is_kw(Kw::At) {
                self.pos += 1;
                self.pending_at = Some(self.at_address()?);
            }
        }
        while self.is_kw(Kw::Attribute) {
            self.skip_attribute(None)?;
        }
        // Nested declarator?
        if self.is_p("(") && !self.paren_starts_params() {
            self.pos += 1;
            let start = self.pos;
            // Skip the nested declarator to parse the suffix first.
            let mut depth = 1;
            while depth > 0 {
                if self.at_eof() {
                    return err(self.loc(), "unbalanced parentheses in declarator");
                }
                if self.is_p("(") {
                    depth += 1;
                } else if self.is_p(")") {
                    depth -= 1;
                }
                self.pos += 1;
            }
            let (outer, _, fattrs_outer) = self.declarator_suffix(base)?;
            let end = self.pos;
            self.pos = start;
            let (t, name, loc, fattrs, pn) = self.declarator_full(outer)?;
            self.expect_p(")")?;
            self.pos = end;
            let fattrs = merge_attrs(&fattrs, &fattrs_outer);
            return Ok((t, name, loc, fattrs, pn));
        }
        let loc = self.loc();
        let name = if let Tok::Ident(n) = self.peek().clone() {
            self.pos += 1;
            Some(n)
        } else {
            None
        };
        let (t, pn, fattrs) = self.declarator_suffix(base)?;
        Ok((t, name, loc, fattrs, pn))
    }

    fn paren_starts_params(&self) -> bool {
        // Called when at '('. It starts a parameter list if followed by ')' or a type name.
        let t = self.peek_at(1);
        // `(__code *)`: a qualifier right before '*' belongs to a nested pointer declarator.
        if is_qual_kw(t) && matches!(self.peek_at(2), Tok::Punct("*")) {
            return false;
        }
        matches!(t, Tok::Punct(")")) || (self.is_typename_tok(t) && !matches!(t, Tok::Kw(Kw::Attribute))) || matches!(t, Tok::Punct("..."))
    }

    fn declarator_suffix(&mut self, base: Type) -> Result<(Type, Option<Vec<(Option<Rc<str>>, Type, Loc)>>, FuncAttrs)> {
        if self.eat_p("[") {
            let mut q = Quals::default();
            loop {
                if self.eat_kw(Kw::Static) {
                    continue;
                }
                let q2 = self.pointer_quals()?;
                if q2 == Quals::default() {
                    break;
                }
                q = q2;
            }
            let _ = q;
            let n = if self.is_p("]") {
                None
            } else if self.is_p("*") && matches!(self.peek_at(1), Tok::Punct("]")) {
                self.pos += 1;
                None
            } else {
                let loc = self.loc();
                let v = self.const_int_expr()?;
                if v < 0 {
                    return err(loc, "array size is negative");
                }
                Some(v as u32)
            };
            self.expect_p("]")?;
            let (elem, _, fa) = self.declarator_suffix(base)?;
            if elem.is_func() {
                return err(self.loc(), "declaration of array of functions");
            }
            // Array elements inherit the storage space qualifier.
            let q = elem.q;
            let mut t = Type::new(TypeKind::Array(Rc::new(elem), n));
            t.q = q;
            return Ok((t, None, fa));
        }
        if self.eat_p("(") {
            let (params, names, variadic, unproto) = self.param_list()?;
            let mut fattrs = FuncAttrs::default();
            loop {
                if self.is_kw(Kw::Attribute) {
                    self.skip_attribute(Some(&mut fattrs))?;
                    continue;
                }
                if !self.func_attr(&mut fattrs)? {
                    break;
                }
            }
            let (ret, _, fa2) = self.declarator_suffix(base)?;
            let fattrs = merge_attrs(&fattrs, &fa2);
            if ret.is_func() || ret.is_array() {
                return err(self.loc(), "function cannot return array or function type");
            }
            let ft = FuncType { ret: ret.unqual_keep_space(), params, variadic, unprototyped: unproto, attrs: fattrs.clone() };
            return Ok((Type::new(TypeKind::Func(Rc::new(ft))), Some(names), fattrs));
        }
        Ok((base, None, FuncAttrs::default()))
    }

    #[allow(clippy::type_complexity)]
    fn param_list(&mut self) -> Result<(Vec<Type>, Vec<(Option<Rc<str>>, Type, Loc)>, bool, bool)> {
        if self.eat_p(")") {
            return Ok((vec![], vec![], false, true));
        }
        if self.is_kw(Kw::Void) && matches!(self.peek_at(1), Tok::Punct(")")) {
            self.pos += 2;
            return Ok((vec![], vec![], false, false));
        }
        // K&R identifier list?
        if matches!(self.peek(), Tok::Ident(_)) && !self.is_typename() {
            let mut names = Vec::new();
            loop {
                let loc = self.loc();
                let n = self.expect_ident()?;
                names.push((Some(n), Type::int(), loc));
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p(")")?;
            return Ok((vec![], names, false, true));
        }
        let mut params = Vec::new();
        let mut names = Vec::new();
        let mut variadic = false;
        loop {
            if self.eat_p("...") {
                variadic = true;
                break;
            }
            let spec = self.decl_spec()?;
            let (mut ty, name, loc) = self.declarator(spec.ty.clone())?;
            ty = self.adjust_param_type(ty);
            params.push(ty.clone());
            names.push((name, ty, loc));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok((params, names, variadic, false))
    }

    fn adjust_param_type(&mut self, ty: Type) -> Type {
        match &ty.kind {
            TypeKind::Array(e, _) => {
                let e = (**e).clone();
                self.ptr_to(e)
            }
            TypeKind::Func(_) => self.ptr_to(ty),
            _ => ty,
        }
    }

    pub(crate) fn type_name(&mut self) -> Result<Type> {
        let spec = self.decl_spec()?;
        let (t, name, loc) = self.declarator(spec.ty)?;
        if name.is_some() {
            return err(loc, "unexpected identifier in type name");
        }
        Ok(t)
    }

    // ------------------------------------------------------------------
    // Declarations

    fn object_space(&self, ty: &Type, spec: &DeclSpec) -> Space {
        if let Some(s) = spec.special {
            return s;
        }
        if let Some(s) = ty.q.space {
            return s;
        }
        let mut t = ty;
        while let TypeKind::Array(e, _) = &t.kind {
            if let Some(s) = e.q.space {
                return s;
            }
            t = e;
        }
        if ty.is_bit() {
            return Space::Bit;
        }
        // SDCC places const, non-volatile static objects into program memory.
        // The qualifier may sit on the array type itself (through a typedef) or on the elements.
        let mut base = ty;
        let mut is_const = ty.q.is_const;
        let mut is_volatile = ty.q.is_volatile;
        while let TypeKind::Array(e, _) = &base.kind {
            base = e;
            is_const |= base.q.is_const;
            is_volatile |= base.q.is_volatile;
        }
        if is_const && !is_volatile && spec.at.is_none() {
            return Space::Code;
        }
        Space::Data
    }

    fn global_decl(&mut self, name: Rc<str>, ty: Type, spec: &DeclSpec, loc: Loc) -> Result<()> {
        let space = self.object_space(&ty, spec);
        let linkage = if spec.storage == Storage::Static { Linkage::Internal } else { Linkage::External };
        let is_extern = spec.storage == Storage::Extern;
        let has_init = self.is_p("=");
        let gid = self.find_or_create_global(&name, &ty, linkage, space, loc, is_extern && !has_init)?;
        if let Some(a) = spec.at {
            self.prog.globals[gid].at = Some(a);
            self.prog.globals[gid].defined = true;
        }
        if spec.special.is_some() {
            self.prog.globals[gid].defined = true;
        }
        if has_init {
            self.pos += 1;
            // An earlier declaration may have completed the array type.
            let ty = match (&ty.kind, &self.prog.globals[gid].ty.kind) {
                (TypeKind::Array(_, None), TypeKind::Array(_, Some(_))) => {
                    let mut t = self.prog.globals[gid].ty.clone();
                    t.q = ty.q.clone();
                    t
                }
                _ => ty,
            };
            self.static_init += 1;
            let r = self.initializer(&ty);
            self.static_init -= 1;
            let (items, fty) = r?;
            let data = self.eval_static_init(&fty, &items, loc)?;
            let g = &mut self.prog.globals[gid];
            if g.init.is_some() {
                return err(loc, format!("redefinition of '{}'", name));
            }
            g.ty = fty;
            g.init = Some(data);
            g.defined = true;
            g.tentative = false;
            g.is_extern_decl = false;
        } else if !is_extern {
            let g = &mut self.prog.globals[gid];
            if !g.defined {
                g.tentative = true;
                g.is_extern_decl = false;
            }
        }
        Ok(())
    }

    fn find_or_create_global(&mut self, name: &Rc<str>, ty: &Type, linkage: Linkage, space: Space, loc: Loc, is_extern_decl: bool) -> Result<GlobalId> {
        // Existing declaration in file scope?
        let existing = match self.scopes[0].names.get(name) {
            Some(Entry::Global(g)) => Some(*g),
            Some(Entry::Func(_)) => return err(loc, format!("'{}' redeclared as different kind of symbol", name)),
            _ => None,
        };
        let existing = existing.or_else(|| {
            if linkage == Linkage::External {
                match self.prog.externs.get(name) {
                    Some(Sym::Global(g)) => Some(*g),
                    _ => None,
                }
            } else {
                None
            }
        });
        if let Some(g) = existing {
            let old_ty = self.prog.globals[g].ty.clone();
            if !self.prog.compatible(&old_ty, ty) {
                return err(loc, format!("conflicting types for '{}'", name));
            }
            self.unify(&old_ty, ty);
            // Complete array types.
            if let (TypeKind::Array(_, None), TypeKind::Array(_, Some(_))) = (&old_ty.kind, &ty.kind) {
                self.prog.globals[g].ty = ty.clone();
            }
            if !is_extern_decl && self.prog.globals[g].space != space && space != Space::Data {
                self.prog.globals[g].space = space;
            }
            self.scopes[0].names.insert(name.clone(), Entry::Global(g));
            return Ok(g);
        }
        if linkage == Linkage::External {
            if let Some(Sym::Func(_)) = self.prog.externs.get(name) {
                return err(loc, format!("'{}' redeclared as different kind of symbol", name));
            }
        }
        let id = self.prog.globals.len();
        self.prog.globals.push(Global {
            name: name.clone(),
            ty: ty.clone(),
            linkage,
            space,
            init: None,
            defined: false,
            at: None,
            loc,
            addr_taken: false,
            is_string: false,
            tentative: false,
            is_extern_decl,
            volatile: ty.q.is_volatile,
            tu: self.tu,
        });
        if linkage == Linkage::External {
            self.prog.externs.insert(name.clone(), Sym::Global(id));
        }
        if self.fctx.is_none() || linkage == Linkage::External {
            self.scopes[0].names.insert(name.clone(), Entry::Global(id));
        }
        Ok(id)
    }

    /// Add a fresh function entry for a name that already has a definition (an inline definition
    /// alongside an external one).
    fn add_func_entry(&mut self, name: Rc<str>, ty: Type, loc: Loc, spec: &DeclSpec, external: bool) -> FuncId {
        let id = self.prog.funcs.len();
        self.prog.funcs.push(Function {
            name: name.clone(),
            ty,
            linkage: if external { Linkage::External } else { Linkage::Internal },
            body: None,
            locals: vec![],
            params: vec![],
            labels: vec![],
            loc,
            is_inline: spec.inline,
            inline_body: false,
            has_external_def: external,
            addr_taken: false,
            nooverlay: self.nooverlay,
            tu: self.tu,
        });
        if external {
            self.prog.externs.insert(name.clone(), Sym::Func(id));
        }
        self.scopes[0].names.insert(name, Entry::Func(id));
        id
    }

    fn declare_func(&mut self, name: Rc<str>, ty: Type, spec: &DeclSpec, loc: Loc, is_def: bool) -> Result<FuncId> {
        let static_ = spec.storage == Storage::Static;
        let existing = match self.lookup(&name) {
            Some(Entry::Func(f)) => Some(*f),
            Some(Entry::Global(_)) if self.scopes[0].names.contains_key(&name) => {
                return err(loc, format!("'{}' redeclared as different kind of symbol", name));
            }
            _ => None,
        };
        let existing = existing.or_else(|| {
            if !static_ {
                match self.prog.externs.get(&name) {
                    Some(Sym::Func(f)) => Some(*f),
                    _ => None,
                }
            } else {
                None
            }
        });
        if let Some(fid) = existing {
            let f = &self.prog.funcs[fid];
            let old = f.ty.clone();
            if !self.prog.compatible(&old, &ty) {
                if self.weak {
                    // The program redefines a library function with another signature: its own
                    // definition owns the name, and the library one becomes unreferenced.
                    return Ok(self.add_func_entry(name, ty, loc, spec, false));
                }
                return err(loc, format!("conflicting types for '{}'", name));
            }
            self.unify(&old, &ty);
            // Merge prototype information and attributes.
            let oft = old.func().unwrap().clone();
            let nft = ty.func().unwrap().clone();
            let mut merged = if oft.unprototyped && !nft.unprototyped { (*nft).clone() } else { (*oft).clone() };
            merged.attrs = merge_attrs(&oft.attrs, &nft.attrs);
            if is_def {
                merged.params = nft.params.clone();
                merged.unprototyped = nft.unprototyped && nft.params.is_empty() && false;
                merged.variadic = nft.variadic;
            }
            let f = &mut self.prog.funcs[fid];
            f.ty = Type::new(TypeKind::Func(Rc::new(merged)));
            if !spec.inline && spec.storage != Storage::Static {
                f.has_external_def = f.has_external_def || is_def || spec.storage == Storage::Extern;
            }
            if spec.storage == Storage::Extern {
                f.has_external_def = true;
            }
            if spec.inline {
                f.is_inline = true;
            }
            if self.nooverlay {
                f.nooverlay = true;
            }
            self.declare(name, Entry::Func(fid));
            return Ok(fid);
        }
        let id = self.prog.funcs.len();
        self.prog.funcs.push(Function {
            name: name.clone(),
            ty,
            linkage: if static_ { Linkage::Internal } else { Linkage::External },
            body: None,
            locals: vec![],
            params: vec![],
            labels: vec![],
            loc,
            is_inline: spec.inline,
            inline_body: false,
            has_external_def: !spec.inline || spec.storage == Storage::Extern,
            addr_taken: false,
            nooverlay: self.nooverlay,
            tu: self.tu,
        });
        if !static_ {
            self.prog.externs.insert(name.clone(), Sym::Func(id));
        }
        if self.fctx.is_some() && !static_ {
            // Block-scope function declaration.
            self.declare(name.clone(), Entry::Func(id));
            self.scopes[0].names.entry(name).or_insert(Entry::Func(id));
        } else {
            self.scopes[0].names.insert(name, Entry::Func(id));
        }
        Ok(id)
    }

    fn function_def(&mut self, name: Rc<str>, mut ty: Type, spec: &DeclSpec, loc: Loc, pnames: Option<Vec<(Option<Rc<str>>, Type, Loc)>>) -> Result<()> {
        let mut pnames = pnames.unwrap_or_default();
        // K&R parameter declarations.
        if !self.is_p("{") {
            let mut decls: HashMap<Rc<str>, Type> = HashMap::new();
            while !self.is_p("{") {
                let s = self.decl_spec()?;
                loop {
                    let (t, n, l) = self.declarator(s.ty.clone())?;
                    let Some(n) = n else { return err(l, "expected parameter name") };
                    let t = self.adjust_param_type(t);
                    decls.insert(n, t);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p(";")?;
            }
            let mut params = Vec::new();
            for p in pnames.iter_mut() {
                let t = p.0.as_ref().and_then(|n| decls.get(n)).cloned().unwrap_or_else(Type::int);
                // K&R: promoted types are passed.
                p.1 = t.clone();
                params.push(t);
            }
            let ft = ty.func().unwrap();
            let nft = FuncType { ret: ft.ret.clone(), params, variadic: false, unprototyped: true, attrs: ft.attrs.clone() };
            ty = Type::new(TypeKind::Func(Rc::new(nft)));
        } else if ty.func().unwrap().unprototyped && pnames.is_empty() {
            // `f() { }` defines a function with no parameters.
            let ft = ty.func().unwrap();
            let nft = FuncType { ret: ft.ret.clone(), params: vec![], variadic: false, unprototyped: true, attrs: ft.attrs.clone() };
            ty = Type::new(TypeKind::Func(Rc::new(nft)));
        }
        let mut fid = self.declare_func(name.clone(), ty.clone(), spec, loc, true)?;
        // An inline definition provides no external symbol (C99 6.7.4): it serves its own
        // translation unit, and an external definition elsewhere is a separate function.
        let inline_def = spec.inline && spec.storage == Storage::None && !self.weak;
        if self.prog.funcs[fid].body.is_some() {
            let other_inline = self.prog.funcs[fid].inline_body;
            if other_inline && !inline_def && !self.weak {
                self.prog.funcs[fid].linkage = Linkage::Internal;
                self.prog.funcs[fid].inline_body = false;
                fid = self.add_func_entry(name.clone(), ty, loc, spec, true);
            } else if inline_def && !other_inline {
                fid = self.add_func_entry(name.clone(), ty, loc, spec, false);
            } else if self.weak || spec.inline || self.prog.funcs[fid].is_inline {
                // Duplicate inline definition from another TU: parse and discard.
                self.skip_braces()?;
                return Ok(());
            } else {
                return err(loc, format!("redefinition of '{}'", name));
            }
        }
        self.prog.funcs[fid].inline_body = inline_def;
        let ret_ty = self.prog.funcs[fid].ftype().ret.clone();
        self.fctx = Some(FuncCtx {
            id: fid,
            locals: vec![],
            labels: vec![],
            label_defined: vec![],
            label_map: HashMap::new(),
            label_uses: vec![],
            switches: vec![],
            loop_depth: 0,
            break_depth: 0,
            ret_ty,
        });
        self.push_scope();
        let mut params = Vec::new();
        // Use the merged function type for parameter types so that space variables are shared.
        let ptys = self.prog.funcs[fid].ftype().params.clone();
        for (i, (n, t, l)) in pnames.into_iter().enumerate() {
            let t = ptys.get(i).cloned().unwrap_or(t);
            let name = n.clone().unwrap_or_else(|| format!("__unnamed_param{}", i).into());
            let lid = self.new_local(name.clone(), t, l, true, None);
            if n.is_some() {
                self.declare(name, Entry::Local(lid));
            }
            params.push(lid);
        }
        // __func__
        let fname_bytes: Vec<u8> = name.as_bytes().iter().copied().chain(std::iter::once(0)).collect();
        let _ = fname_bytes;
        self.prog.funcs[fid].tu = self.tu;
        let body = self.compound_stmt_no_scope()?;
        self.pop_scope();
        let ctx = self.fctx.take().unwrap();
        for (lid, l) in &ctx.label_uses {
            if !ctx.label_defined[*lid] {
                return err(*l, format!("use of undeclared label '{}'", ctx.labels[*lid]));
            }
        }
        let f = &mut self.prog.funcs[fid];
        f.body = Some(body);
        f.locals = ctx.locals;
        f.params = params;
        f.labels = ctx.labels;
        f.loc = loc;
        if spec.inline && spec.storage != Storage::Static && spec.storage != Storage::Extern {
            // C99 inline definition: does not by itself provide an external definition.
        } else {
            f.has_external_def = true;
        }
        if self.nooverlay {
            f.nooverlay = true;
        }
        Ok(())
    }

    fn skip_braces(&mut self) -> Result<()> {
        self.expect_p("{")?;
        let mut depth = 1;
        while depth > 0 {
            if self.at_eof() {
                return err(self.loc(), "unexpected end of file");
            }
            if self.is_p("{") {
                depth += 1;
            } else if self.is_p("}") {
                depth -= 1;
            }
            self.pos += 1;
        }
        Ok(())
    }

    fn new_local(&mut self, name: Rc<str>, ty: Type, loc: Loc, is_param: bool, space: Option<Space>) -> LocalId {
        let ctx = self.fctx.as_mut().unwrap();
        ctx.locals.push(Local { name, ty, addr_taken: false, is_param, loc, space, register: false });
        ctx.locals.len() - 1
    }

    /// Block-scope declaration. Returns statements for initializers.
    fn local_decl(&mut self, out: &mut Vec<Stmt>) -> Result<()> {
        let spec = self.decl_spec()?;
        if self.eat_p(";") {
            return Ok(());
        }
        loop {
            let (ty, name, loc, fattrs, _) = self.declarator_full(spec.ty.clone())?;
            let mut spec2 = spec.clone();
            self.post_declarator_attrs(&mut spec2)?;
            let Some(name) = name else { return err(loc, "declarator requires an identifier") };
            if spec2.storage == Storage::Typedef {
                self.declare(name, Entry::Typedef(ty));
            } else if ty.is_func() {
                let ty = with_fattrs(&ty, merge_attrs(&spec2.fattrs, &fattrs));
                let mut s3 = spec2.clone();
                if s3.storage == Storage::None {
                    s3.storage = Storage::Extern;
                }
                self.declare_func(name, ty, &s3, loc, false)?;
            } else if spec2.storage == Storage::Extern {
                let space = self.object_space(&ty, &spec2);
                let gid = self.find_or_create_global(&name, &ty, Linkage::External, space, loc, true)?;
                self.declare(name, Entry::Global(gid));
            } else if spec2.storage == Storage::Static || spec2.special.is_some() || spec2.at.is_some() {
                // Static local: a global with internal linkage and a unique name.
                let fname = self.prog.funcs[self.fctx.as_ref().unwrap().id].name.clone();
                let uname: Rc<str> = format!("{}.{}.{}", fname, name, self.anon_counter).into();
                self.anon_counter += 1;
                let space = self.object_space(&ty, &spec2);
                let gid = self.prog.globals.len();
                self.prog.globals.push(Global {
                    name: uname,
                    ty: ty.clone(),
                    linkage: Linkage::Internal,
                    space,
                    init: None,
                    defined: true,
                    at: spec2.at,
                    loc,
                    addr_taken: false,
                    is_string: false,
                    tentative: true,
                    is_extern_decl: false,
                    volatile: ty.q.is_volatile,
                    tu: self.tu,
                });
                self.declare(name, Entry::Global(gid));
                if self.eat_p("=") {
                    self.static_init += 1;
                    let r = self.initializer(&ty);
                    self.static_init -= 1;
                    let (items, fty) = r?;
                    let data = self.eval_static_init(&fty, &items, loc)?;
                    let g = &mut self.prog.globals[gid];
                    g.ty = fty;
                    g.init = Some(data);
                    g.tentative = false;
                }
            } else {
                if self.prog.sizeof(&ty).is_none() && !matches!(ty.kind, TypeKind::Array(_, None)) {
                    return err(loc, format!("variable '{}' has incomplete type", name));
                }
                if ty.is_void() {
                    return err(loc, format!("variable '{}' declared void", name));
                }
                let space = ty.q.space.filter(|s| *s != Space::Data);
                let lid = self.new_local(name.clone(), ty.clone(), loc, false, space);
                if spec2.storage == Storage::Register {
                    self.fctx.as_mut().unwrap().locals[lid].register = true;
                }
                if self.eat_p("=") {
                    self.declare(name.clone(), Entry::Local(lid));
                    let (items, fty) = self.initializer(&ty)?;
                    self.fctx.as_mut().unwrap().locals[lid].ty = fty.clone();
                    let inits = self.local_init_list(&fty, items)?;
                    out.push(Stmt::InitLocal(lid, inits, loc));
                } else {
                    if matches!(ty.kind, TypeKind::Array(_, None)) {
                        return err(loc, format!("array size missing in '{}'", name));
                    }
                    self.declare(name, Entry::Local(lid));
                }
            }
            if self.eat_p(",") {
                continue;
            }
            self.expect_p(";")?;
            return Ok(());
        }
    }

    /// Create (or reuse) an anonymous global for a string literal.
    fn string_global(&mut self, bytes: &[u8], width: u8) -> GlobalId {
        let mut data = bytes.to_vec();
        data.extend(std::iter::repeat(0).take(width as usize));
        if let Some(g) = self.prog.string_pool.get(&(data.clone(), width)) {
            return *g;
        }
        let id = self.prog.globals.len();
        let mut elem = match width {
            2 => Type::uint(),
            4 => Type::ulong(),
            _ => Type::char(),
        };
        // A string literal has type char[] (not const), but lives in code space.
        elem.q.space = Some(Space::Code);
        let ty = Type::new(TypeKind::Array(Rc::new(elem), Some(data.len() as u32 / width as u32)));
        self.prog.globals.push(Global {
            name: format!("__str_{}", id).into(),
            ty,
            linkage: Linkage::Internal,
            space: Space::Code,
            init: Some(InitData { bytes: data.clone(), relocs: vec![] }),
            defined: true,
            at: None,
            loc: self.loc(),
            addr_taken: false,
            is_string: true,
            tentative: false,
            is_extern_decl: false,
            volatile: false,
            tu: self.tu,
        });
        self.prog.string_pool.insert((data, width), id);
        id
    }
}

trait TypeExt {
    fn unqual_keep_space(&self) -> Type;
}
impl TypeExt for Type {
    fn unqual_keep_space(&self) -> Type {
        let mut t = self.clone();
        t.q.is_const = false;
        t.q.is_volatile = false;
        t
    }
}

fn merge_attrs(a: &FuncAttrs, b: &FuncAttrs) -> FuncAttrs {
    FuncAttrs {
        interrupt: a.interrupt.or(b.interrupt),
        using: a.using.or(b.using),
        naked: a.naked || b.naked,
        critical: a.critical || b.critical,
        reentrant: a.reentrant || b.reentrant,
        noreturn: a.noreturn || b.noreturn,
        preserves: a.preserves.clone().or_else(|| b.preserves.clone()),
    }
}

fn with_fattrs(t: &Type, a: FuncAttrs) -> Type {
    match &t.kind {
        TypeKind::Func(f) => {
            let mut nf = (**f).clone();
            nf.attrs = merge_attrs(&nf.attrs, &a);
            Type::new(TypeKind::Func(Rc::new(nf)))
        }
        _ => t.clone(),
    }
}

pub(crate) fn loc_err(loc: Loc, msg: &str) -> crate::diag::Error {
    error(loc, msg)
}
