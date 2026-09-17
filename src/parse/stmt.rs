//! Statement parsing.

use super::*;
use crate::parse::consteval::fold_cast;

impl<'a> Parser<'a> {
    pub(super) fn compound_stmt_no_scope(&mut self) -> Result<Stmt> {
        self.expect_p("{")?;
        let mut stmts = Vec::new();
        while !self.eat_p("}") {
            if self.at_eof() {
                return err(self.loc(), "expected '}'");
            }
            self.block_item(&mut stmts)?;
        }
        Ok(Stmt::Block(stmts))
    }

    fn compound_stmt(&mut self) -> Result<Stmt> {
        self.push_scope();
        let r = self.compound_stmt_no_scope();
        self.pop_scope();
        r
    }

    pub(super) fn block_item(&mut self, out: &mut Vec<Stmt>) -> Result<()> {
        if let Tok::Pragma(p) = self.peek().clone() {
            self.pos += 1;
            self.handle_pragma(&p);
            return Ok(());
        }
        if self.is_kw(Kw::StaticAssert) {
            return self.static_assert();
        }
        let is_label = matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Punct(":"));
        if self.is_typename() && !is_label {
            // `__extension__` may precede an expression too.
            if self.is_kw(Kw::Extension) && !self.is_typename_tok(&self.peek_at(1).clone()) {
                let s = self.stmt()?;
                out.push(s);
                return Ok(());
            }
            return self.local_decl(out);
        }
        let s = self.stmt()?;
        out.push(s);
        Ok(())
    }

    fn cond(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let e = self.expr()?;
        let e = self.rval(e);
        if !e.ty.is_scalar() {
            return err(loc, format!("statement requires expression of scalar type ('{}' given)", e.ty));
        }
        Ok(e)
    }

    fn label_id(&mut self, name: &Rc<str>) -> LabelId {
        let ctx = self.fctx.as_mut().unwrap();
        if let Some(l) = ctx.label_map.get(name) {
            return *l;
        }
        let id = ctx.labels.len();
        ctx.labels.push(name.clone());
        ctx.label_defined.push(false);
        ctx.label_map.insert(name.clone(), id);
        id
    }

    fn new_label(&mut self, name: &str) -> LabelId {
        let ctx = self.fctx.as_mut().unwrap();
        let id = ctx.labels.len();
        ctx.labels.push(format!(".{}{}", name, id).into());
        ctx.label_defined.push(true);
        id
    }

    fn sub_stmt(&mut self) -> Result<Stmt> {
        // Sub-statements get their own scope (C99).
        self.push_scope();
        let r = self.stmt();
        self.pop_scope();
        r
    }

    pub(super) fn stmt(&mut self) -> Result<Stmt> {
        let loc = self.loc();
        match self.peek().clone() {
            Tok::Punct("{") => self.compound_stmt(),
            Tok::Punct(";") => {
                self.pos += 1;
                Ok(Stmt::Empty)
            }
            Tok::Pragma(p) => {
                self.pos += 1;
                self.handle_pragma(&p);
                Ok(Stmt::Empty)
            }
            Tok::Ident(name) if matches!(self.peek_at(1), Tok::Punct(":")) => {
                self.pos += 2;
                let id = self.label_id(&name);
                let ctx = self.fctx.as_mut().unwrap();
                if ctx.label_defined[id] {
                    return err(loc, format!("redefinition of label '{}'", name));
                }
                ctx.label_defined[id] = true;
                while self.is_kw(Kw::Attribute) {
                    self.skip_attribute(None)?;
                }
                let s = if self.is_p("}") {
                    Stmt::Empty
                } else if self.is_typename() {
                    // C23 allows labels before declarations.
                    let mut v = Vec::new();
                    self.local_decl(&mut v)?;
                    Stmt::Block(v)
                } else {
                    self.stmt()?
                };
                Ok(Stmt::Label(id, Box::new(s)))
            }
            Tok::Kw(k) => self.keyword_stmt(k, loc),
            _ => {
                let e = self.expr()?;
                self.expect_p(";")?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn keyword_stmt(&mut self, k: Kw, loc: Loc) -> Result<Stmt> {
        match k {
            Kw::If => {
                self.pos += 1;
                self.expect_p("(")?;
                let c = self.cond()?;
                self.expect_p(")")?;
                let t = self.sub_stmt()?;
                let e = if self.eat_kw(Kw::Else) { Some(Box::new(self.sub_stmt()?)) } else { None };
                Ok(Stmt::If(c, Box::new(t), e))
            }
            Kw::While => {
                self.pos += 1;
                self.expect_p("(")?;
                let c = self.cond()?;
                self.expect_p(")")?;
                self.enter_loop();
                let b = self.sub_stmt();
                self.leave_loop();
                Ok(Stmt::While(c, Box::new(b?)))
            }
            Kw::Do => {
                self.pos += 1;
                self.enter_loop();
                let b = self.sub_stmt();
                self.leave_loop();
                let b = b?;
                if !self.eat_kw(Kw::While) {
                    return err(self.loc(), "expected 'while' in do/while loop");
                }
                self.expect_p("(")?;
                let c = self.cond()?;
                self.expect_p(")")?;
                self.expect_p(";")?;
                Ok(Stmt::DoWhile(Box::new(b), c))
            }
            Kw::For => {
                self.pos += 1;
                self.expect_p("(")?;
                self.push_scope();
                let init = if self.eat_p(";") {
                    None
                } else if self.is_typename() {
                    let mut v = Vec::new();
                    self.local_decl(&mut v)?;
                    Some(Box::new(Stmt::Block(v)))
                } else {
                    let e = self.expr()?;
                    self.expect_p(";")?;
                    Some(Box::new(Stmt::Expr(e)))
                };
                let c = if self.is_p(";") { None } else { Some(self.cond()?) };
                self.expect_p(";")?;
                let step = if self.is_p(")") { None } else { Some(self.expr()?) };
                self.expect_p(")")?;
                self.enter_loop();
                let b = self.sub_stmt();
                self.leave_loop();
                self.pop_scope();
                Ok(Stmt::For(init, c, step, Box::new(b?)))
            }
            Kw::Switch => {
                self.pos += 1;
                self.expect_p("(")?;
                let e = self.expr()?;
                let e = self.rval(e);
                if !e.ty.is_integer() {
                    return err(loc, "statement requires expression of integer type");
                }
                self.expect_p(")")?;
                let e = self.promote(e);
                let ty = e.ty.clone();
                self.fctx.as_mut().unwrap().switches.push(SwitchCtx { cases: vec![], default: None, ty });
                self.fctx.as_mut().unwrap().break_depth += 1;
                let b = self.sub_stmt();
                self.fctx.as_mut().unwrap().break_depth -= 1;
                let sw = self.fctx.as_mut().unwrap().switches.pop().unwrap();
                Ok(Stmt::Switch(e, Box::new(b?), sw.cases, sw.default))
            }
            Kw::Case => {
                self.pos += 1;
                if self.fctx.as_ref().unwrap().switches.is_empty() {
                    return err(loc, "'case' statement not in switch statement");
                }
                let lo = self.const_int_expr()?;
                let hi = if self.eat_p("...") { self.const_int_expr()? } else { lo };
                self.expect_p(":")?;
                let label = self.new_label("case");
                let sw_ty = self.fctx.as_ref().unwrap().switches.last().unwrap().ty.clone();
                let mut v = lo;
                loop {
                    let cv = fold_cast(v, &Type::llong(), &sw_ty);
                    let sw = self.fctx.as_mut().unwrap().switches.last_mut().unwrap();
                    if sw.cases.iter().any(|(x, _)| *x == cv) {
                        return err(loc, format!("duplicate case value '{}'", cv));
                    }
                    sw.cases.push((cv, label));
                    if v >= hi {
                        break;
                    }
                    v += 1;
                }
                let s = if self.is_p("}") { Stmt::Empty } else { self.stmt()? };
                Ok(Stmt::Label(label, Box::new(s)))
            }
            Kw::Default => {
                self.pos += 1;
                self.expect_p(":")?;
                if self.fctx.as_ref().unwrap().switches.is_empty() {
                    return err(loc, "'default' statement not in switch statement");
                }
                let label = self.new_label("default");
                let sw = self.fctx.as_mut().unwrap().switches.last_mut().unwrap();
                if sw.default.is_some() {
                    return err(loc, "multiple default labels in one switch");
                }
                sw.default = Some(label);
                let s = if self.is_p("}") { Stmt::Empty } else { self.stmt()? };
                Ok(Stmt::Label(label, Box::new(s)))
            }
            Kw::Break => {
                self.pos += 1;
                self.expect_p(";")?;
                let ctx = self.fctx.as_ref().unwrap();
                if ctx.break_depth == 0 {
                    return err(loc, "'break' statement not in loop or switch statement");
                }
                Ok(Stmt::Break)
            }
            Kw::Continue => {
                self.pos += 1;
                self.expect_p(";")?;
                if self.fctx.as_ref().unwrap().loop_depth == 0 {
                    return err(loc, "'continue' statement not in loop statement");
                }
                Ok(Stmt::Continue)
            }
            Kw::Return => {
                self.pos += 1;
                let ret_ty = self.fctx.as_ref().unwrap().ret_ty.clone();
                if self.eat_p(";") {
                    if !ret_ty.is_void() {
                        diag::warn(loc, "non-void function should return a value");
                    }
                    return Ok(Stmt::Return(None, loc));
                }
                let e = self.expr()?;
                self.expect_p(";")?;
                if ret_ty.is_void() {
                    let e = self.rval(e);
                    if !e.ty.is_void() {
                        diag::warn(loc, "void function should not return a value");
                    }
                    return Ok(Stmt::Block(vec![Stmt::Expr(e), Stmt::Return(None, loc)]));
                }
                let e = self.assign_conv(e, &ret_ty.unqual_keep_space(), loc)?;
                Ok(Stmt::Return(Some(e), loc))
            }
            Kw::Goto => {
                self.pos += 1;
                if self.is_p("*") {
                    return err(loc, "computed goto is not supported");
                }
                let name = self.expect_ident()?;
                self.expect_p(";")?;
                let id = self.label_id(&name);
                self.fctx.as_mut().unwrap().label_uses.push((id, loc));
                Ok(Stmt::Goto(id, loc))
            }
            Kw::Critical => {
                self.pos += 1;
                let b = self.stmt()?;
                Ok(Stmt::Critical(Box::new(b)))
            }
            Kw::AsmBegin => {
                self.pos += 1;
                let text = match self.next().tok {
                    Tok::Asm(s) => s,
                    _ => return err(loc, "expected assembly block"),
                };
                if !self.eat_kw(Kw::AsmEnd) {
                    return err(self.loc(), "expected __endasm");
                }
                self.eat_p(";");
                Ok(Stmt::Asm(text, loc))
            }
            Kw::AsmFn => {
                self.pos += 1;
                self.expect_p("(")?;
                let mut text = Vec::new();
                while let Tok::Str(s) = self.peek().clone() {
                    self.pos += 1;
                    text.extend(s);
                }
                self.expect_p(")")?;
                self.expect_p(";")?;
                Ok(Stmt::Asm(String::from_utf8_lossy(&text).into_owned(), loc))
            }
            _ => {
                let e = self.expr()?;
                self.expect_p(";")?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn enter_loop(&mut self) {
        let c = self.fctx.as_mut().unwrap();
        c.loop_depth += 1;
        c.break_depth += 1;
    }
    fn leave_loop(&mut self) {
        let c = self.fctx.as_mut().unwrap();
        c.loop_depth -= 1;
        c.break_depth -= 1;
    }
}
