//! Expression parsing and type checking.

use super::*;
use crate::parse::consteval::{fold_binary, fold_cast, fold_unary, norm_int};

pub(super) enum LSpace {
    Concrete(Space),
    Var(SpaceVar),
}

fn binop_of(p: &str) -> Option<(BinOp, u8)> {
    Some(match p {
        "||" => (BinOp::LogOr, 1),
        "&&" => (BinOp::LogAnd, 2),
        "|" => (BinOp::Or, 3),
        "^" => (BinOp::Xor, 4),
        "&" => (BinOp::And, 5),
        "==" => (BinOp::Eq, 6),
        "!=" => (BinOp::Ne, 6),
        "<" => (BinOp::Lt, 7),
        ">" => (BinOp::Gt, 7),
        "<=" => (BinOp::Le, 7),
        ">=" => (BinOp::Ge, 7),
        "<<" => (BinOp::Shl, 8),
        ">>" => (BinOp::Shr, 8),
        "+" => (BinOp::Add, 9),
        "-" => (BinOp::Sub, 9),
        "*" => (BinOp::Mul, 10),
        "/" => (BinOp::Div, 10),
        "%" => (BinOp::Mod, 10),
        _ => return None,
    })
}

fn compound_op(p: &str) -> Option<BinOp> {
    Some(match p {
        "+=" => BinOp::Add,
        "-=" => BinOp::Sub,
        "*=" => BinOp::Mul,
        "/=" => BinOp::Div,
        "%=" => BinOp::Mod,
        "<<=" => BinOp::Shl,
        ">>=" => BinOp::Shr,
        "&=" => BinOp::And,
        "|=" => BinOp::Or,
        "^=" => BinOp::Xor,
        _ => return None,
    })
}

impl<'a> Parser<'a> {
    pub(super) fn const_int_expr(&mut self) -> Result<i64> {
        let loc = self.loc();
        let e = self.cond_expr()?;
        match self.eval_const(&e) {
            Some(ConstVal::Int(v)) => Ok(v),
            Some(ConstVal::Float(f)) => Ok(f as i64),
            _ => err(loc, "expression is not an integer constant expression"),
        }
    }

    pub(super) fn expr(&mut self) -> Result<Expr> {
        let mut e = self.assign()?;
        while self.is_p(",") {
            let loc = self.loc();
            self.pos += 1;
            let r = self.assign()?;
            let r = self.rval(r);
            let ty = r.ty.clone();
            e = Expr::new(ExprKind::Comma(Box::new(e), Box::new(r)), ty, loc);
        }
        Ok(e)
    }

    pub(super) fn assign(&mut self) -> Result<Expr> {
        let lhs = self.cond_expr()?;
        let loc = self.loc();
        if self.is_p("=") {
            self.pos += 1;
            let rhs = self.assign()?;
            self.check_lvalue(&lhs, loc)?;
            if lhs.ty.is_array() {
                return err(loc, "array type is not assignable");
            }
            let rhs = self.assign_conv(rhs, &lhs.ty, loc)?;
            let ty = lhs.ty.unqual_keep_space();
            return Ok(Expr::new(ExprKind::Assign(Box::new(lhs), Box::new(rhs)), ty, loc));
        }
        if let Tok::Punct(p) = self.peek().clone() {
            if let Some(op) = compound_op(p) {
                self.pos += 1;
                let rhs = self.assign()?;
                self.check_lvalue(&lhs, loc)?;
                return self.compound_assign(op, lhs, rhs, loc);
            }
        }
        Ok(lhs)
    }

    fn compound_assign(&mut self, op: BinOp, lhs: Expr, rhs: Expr, loc: Loc) -> Result<Expr> {
        let rhs = self.rval(rhs);
        let lty = lhs.ty.clone();
        if lty.is_pointer() {
            if !matches!(op, BinOp::Add | BinOp::Sub) || !rhs.ty.is_integer() {
                return err(loc, "invalid operands to compound assignment");
            }
            let size = self.elem_size(&lty, loc)?;
            let mut r = self.scale_index(rhs, size);
            if op == BinOp::Sub {
                r = self.negate(r);
            }
            let ty = lty.unqual_keep_space();
            return Ok(Expr::new(ExprKind::CompoundAssign(BinOp::Add, Box::new(lhs), Box::new(r), ty.clone()), ty, loc));
        }
        if !lty.is_arith() || !rhs.ty.is_arith() {
            return err(loc, "invalid operands to compound assignment");
        }
        // Determine the operation type.
        let (op_ty, rhs) = match op {
            BinOp::Shl | BinOp::Shr => {
                let pt = self.promoted_type(&lty);
                let r = self.promote(rhs);
                (pt, r)
            }
            _ => {
                let pl = self.promoted_type(&lty);
                let pr = self.promoted_expr_type(&rhs);
                let common = self.common_type(&pl, &pr);
                let r = self.conv(rhs, &common);
                (common, r)
            }
        };
        if matches!(op, BinOp::Mod | BinOp::Shl | BinOp::Shr | BinOp::And | BinOp::Or | BinOp::Xor) && op_ty.is_float() {
            return err(loc, "invalid operands to compound assignment");
        }
        let ty = lty.unqual_keep_space();
        Ok(Expr::new(ExprKind::CompoundAssign(op, Box::new(lhs), Box::new(rhs), op_ty), ty, loc))
    }

    pub(super) fn cond_expr(&mut self) -> Result<Expr> {
        let c = self.binary(0)?;
        if !self.is_p("?") {
            return Ok(c);
        }
        let loc = self.loc();
        self.pos += 1;
        let c = self.rval(c);
        self.check_scalar(&c, loc)?;
        // GNU `a ?: b`
        if self.is_p(":") {
            self.pos += 1;
            let b = self.cond_expr()?;
            let b = self.rval(b);
            let (a2, b2, ty) = self.cond_types(c.clone(), b, loc)?;
            let _ = a2;
            // Evaluate c once: use a comma with a temp is complex; restrict to side-effect-free c.
            return Ok(Expr::new(ExprKind::Cond(Box::new(c.clone()), Box::new(self.conv(c, &ty)), Box::new(b2)), ty, loc));
        }
        let a = self.expr()?;
        self.expect_p(":")?;
        let b = self.cond_expr()?;
        let a = self.rval(a);
        let b = self.rval(b);
        let (a, b, ty) = self.cond_types(a, b, loc)?;
        if let Some(v) = c.as_int() {
            // Keep both arms for type purposes but fold.
            let chosen = if v != 0 { a } else { b };
            let mut chosen = chosen;
            chosen.ty = ty;
            return Ok(chosen);
        }
        Ok(Expr::new(ExprKind::Cond(Box::new(c), Box::new(a), Box::new(b)), ty, loc))
    }

    fn cond_types(&mut self, a: Expr, b: Expr, loc: Loc) -> Result<(Expr, Expr, Type)> {
        if a.ty.is_arith() && b.ty.is_arith() {
            if a.ty.is_bit() && b.ty.is_bit() {
                return Ok((a, b, Type::bit()));
            }
            let (pa, pb) = (self.promoted_expr_type(&a), self.promoted_expr_type(&b));
            let t = self.common_type(&pa, &pb);
            let a = self.conv(a, &t);
            let b = self.conv(b, &t);
            return Ok((a, b, t));
        }
        if a.ty.is_void() || b.ty.is_void() {
            return Ok((a, b, Type::void()));
        }
        if a.ty.is_pointer() && b.ty.is_pointer() {
            let at = a.ty.clone();
            let bt = b.ty.clone();
            self.unify(&at, &bt);
            // Prefer non-void pointee; merge qualifiers.
            let (pa, pb) = (at.pointee().unwrap().clone(), bt.pointee().unwrap().clone());
            let mut ty = if pa.is_void() { at.clone() } else if pb.is_void() { bt.clone() } else { at.clone() };
            if let TypeKind::Pointer(p, v) = &ty.kind {
                let mut p2 = (**p).clone();
                p2.q.is_const |= pa.q.is_const || pb.q.is_const;
                p2.q.is_volatile |= pa.q.is_volatile || pb.q.is_volatile;
                ty = Type::new(TypeKind::Pointer(Rc::new(p2), *v));
            }
            let a = self.conv(a, &ty);
            let b = self.conv(b, &ty);
            return Ok((a, b, ty));
        }
        if a.ty.is_pointer() && self.is_null_const(&b) {
            let t = a.ty.clone();
            let b = self.conv(b, &t);
            return Ok((a, b, t));
        }
        if b.ty.is_pointer() && self.is_null_const(&a) {
            let t = b.ty.clone();
            let a = self.conv(a, &t);
            return Ok((a, b, t));
        }
        if a.ty.is_pointer() && b.ty.is_integer() {
            diag::warn(loc, "pointer/integer type mismatch in conditional expression");
            let t = a.ty.clone();
            let b = self.conv(b, &t);
            return Ok((a, b, t));
        }
        if b.ty.is_pointer() && a.ty.is_integer() {
            diag::warn(loc, "pointer/integer type mismatch in conditional expression");
            let t = b.ty.clone();
            let a = self.conv(a, &t);
            return Ok((a, b, t));
        }
        if a.ty.is_record() && b.ty.is_record() && a.ty.same(&b.ty) {
            let t = a.ty.clone();
            return Ok((a, b, t));
        }
        err(loc, format!("incompatible operand types ('{}' and '{}') in conditional expression", a.ty, b.ty))
    }

    fn binary(&mut self, min: u8) -> Result<Expr> {
        let mut lhs = self.cast_expr()?;
        loop {
            let Tok::Punct(p) = self.peek().clone() else { break };
            let Some((op, prec)) = binop_of(p) else { break };
            if prec <= min {
                break;
            }
            let loc = self.loc();
            self.pos += 1;
            let rhs = self.binary(prec)?;
            lhs = self.make_binary(op, lhs, rhs, loc)?;
        }
        Ok(lhs)
    }

    pub(super) fn make_binary(&mut self, op: BinOp, lhs: Expr, rhs: Expr, loc: Loc) -> Result<Expr> {
        let l = self.rval(lhs);
        let r = self.rval(rhs);
        match op {
            BinOp::LogAnd | BinOp::LogOr => {
                self.check_scalar(&l, loc)?;
                self.check_scalar(&r, loc)?;
                if let (Some(a), Some(b)) = (l.as_int(), r.as_int()) {
                    let v = if op == BinOp::LogAnd { a != 0 && b != 0 } else { a != 0 || b != 0 };
                    return Ok(Expr::int(v as i64, Type::int(), loc));
                }
                if let Some(a) = l.as_int() {
                    // Short-circuit folding.
                    if op == BinOp::LogAnd && a == 0 {
                        return Ok(Expr::int(0, Type::int(), loc));
                    }
                    if op == BinOp::LogOr && a != 0 {
                        return Ok(Expr::int(1, Type::int(), loc));
                    }
                }
                Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), Type::int(), loc))
            }
            BinOp::Add | BinOp::Sub if l.ty.is_pointer() || r.ty.is_pointer() => {
                if l.ty.is_pointer() && r.ty.is_integer() {
                    let size = self.elem_size(&l.ty, loc)?;
                    let mut idx = self.scale_index(r, size);
                    if op == BinOp::Sub {
                        idx = self.negate(idx);
                    }
                    let ty = l.ty.clone();
                    return Ok(self.ptr_add(l, idx, ty, loc));
                }
                if op == BinOp::Add && r.ty.is_pointer() && l.ty.is_integer() {
                    let size = self.elem_size(&r.ty, loc)?;
                    let idx = self.scale_index(l, size);
                    let ty = r.ty.clone();
                    return Ok(self.ptr_add(r, idx, ty, loc));
                }
                if op == BinOp::Sub && l.ty.is_pointer() && r.ty.is_pointer() {
                    let size = self.elem_size(&l.ty, loc)?;
                    let (lt, rt) = (l.ty.clone(), r.ty.clone());
                    self.unify(&lt, &rt);
                    return Ok(Expr::new(ExprKind::PtrDiff(Box::new(l), Box::new(r), size.max(1)), Type::int(), loc));
                }
                err(loc, format!("invalid operands to binary expression ('{}' and '{}')", l.ty, r.ty))
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                if l.ty.is_arith() && r.ty.is_arith() {
                    let (l, r, _) = self.arith_conv(l, r);
                    if let (Some(a), Some(b)) = (l.as_int(), r.as_int()) {
                        let v = fold_binary(op, a, b, &l.ty).unwrap_or(0);
                        return Ok(Expr::int(v, Type::int(), loc));
                    }
                    return Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), Type::int(), loc));
                }
                if l.ty.is_pointer() && r.ty.is_pointer() {
                    let (lt, rt) = (l.ty.clone(), r.ty.clone());
                    self.unify(&lt, &rt);
                    let r = self.conv(r, &lt);
                    return Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), Type::int(), loc));
                }
                if l.ty.is_pointer() && r.ty.is_integer() {
                    if !self.is_null_const(&r) {
                        diag::warn(loc, "comparison between pointer and integer");
                    }
                    let t = l.ty.clone();
                    let r = self.conv(r, &t);
                    return Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), Type::int(), loc));
                }
                if r.ty.is_pointer() && l.ty.is_integer() {
                    if !self.is_null_const(&l) {
                        diag::warn(loc, "comparison between pointer and integer");
                    }
                    let t = r.ty.clone();
                    let l = self.conv(l, &t);
                    return Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), Type::int(), loc));
                }
                err(loc, format!("invalid operands to comparison ('{}' and '{}')", l.ty, r.ty))
            }
            BinOp::Shl | BinOp::Shr => {
                if !l.ty.is_integer() || !r.ty.is_integer() {
                    return err(loc, format!("invalid operands to shift ('{}' and '{}')", l.ty, r.ty));
                }
                let l = self.promote(l);
                let r = self.promote(r);
                let ty = l.ty.clone();
                if let (Some(a), Some(b)) = (l.as_int(), r.as_int()) {
                    if let Some(v) = fold_binary(op, a, b, &ty) {
                        return Ok(Expr::int(v, ty, loc));
                    }
                }
                Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), ty, loc))
            }
            _ => {
                if !l.ty.is_arith() || !r.ty.is_arith() {
                    return err(loc, format!("invalid operands to binary expression ('{}' and '{}')", l.ty, r.ty));
                }
                if matches!(op, BinOp::Mod | BinOp::And | BinOp::Or | BinOp::Xor) && (l.ty.is_float() || r.ty.is_float()) {
                    return err(loc, "invalid operands to binary expression (floating point)");
                }
                let (l, r, ty) = self.arith_conv(l, r);
                if let (Some(a), Some(b)) = (l.as_int(), r.as_int()) {
                    if let Some(v) = fold_binary(op, a, b, &ty) {
                        return Ok(Expr::int(v, ty, loc));
                    }
                }
                if let (ExprKind::Float(a), ExprKind::Float(b)) = (&l.kind, &r.kind) {
                    let v = match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        BinOp::Div => a / b,
                        _ => return Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), ty, loc)),
                    };
                    return Ok(Expr::new(ExprKind::Float(v), ty, loc));
                }
                Ok(Expr::new(ExprKind::Binary(op, Box::new(l), Box::new(r)), ty, loc))
            }
        }
    }

    fn ptr_add(&mut self, p: Expr, idx: Expr, ty: Type, loc: Loc) -> Expr {
        if idx.as_int() == Some(0) {
            return p;
        }
        // Fold (p + a) + b.
        if let ExprKind::PtrAdd(inner, i2) = &p.kind {
            if let (Some(a), Some(b)) = (i2.as_int(), idx.as_int()) {
                let v = norm_int(a + b, &Type::int());
                return Expr::new(ExprKind::PtrAdd(inner.clone(), Box::new(Expr::int(v, Type::int(), loc))), ty, loc);
            }
        }
        Expr::new(ExprKind::PtrAdd(Box::new(p), Box::new(idx)), ty, loc)
    }

    fn negate(&mut self, e: Expr) -> Expr {
        let loc = e.loc;
        let ty = e.ty.clone();
        if let Some(v) = e.as_int() {
            return Expr::int(norm_int(v.wrapping_neg(), &ty), ty, loc);
        }
        Expr::new(ExprKind::Unary(UnOp::Neg, Box::new(e)), ty, loc)
    }

    fn elem_size(&mut self, ptr_ty: &Type, loc: Loc) -> Result<u32> {
        let p = ptr_ty.pointee().unwrap();
        if p.is_void() || p.is_func() {
            return Ok(1);
        }
        match self.prog.sizeof(p) {
            Some(s) => Ok(s),
            None => err(loc, "arithmetic on a pointer to an incomplete type"),
        }
    }

    /// Convert an index to int and multiply by the element size.
    fn scale_index(&mut self, idx: Expr, size: u32) -> Expr {
        let loc = idx.loc;
        let it = if idx.ty.is_signed() { Type::int() } else { Type::uint() };
        let idx = self.conv(idx, &it);
        let idx = self.conv(idx, &Type::int());
        if size == 1 {
            return idx;
        }
        if let Some(v) = idx.as_int() {
            return Expr::int(norm_int(v.wrapping_mul(size as i64), &Type::int()), Type::int(), loc);
        }
        Expr::new(ExprKind::Binary(BinOp::Mul, Box::new(idx), Box::new(Expr::int(size as i64, Type::int(), loc))), Type::int(), loc)
    }

    fn cast_expr(&mut self) -> Result<Expr> {
        if self.is_p("(") && self.is_typename_tok(&self.peek_at(1).clone()) {
            let loc = self.loc();
            let save = self.pos;
            self.pos += 1;
            let ty = self.type_name()?;
            self.expect_p(")")?;
            if self.is_p("{") {
                // Compound literal.
                self.pos = save;
                return self.unary();
            }
            let e = self.cast_expr()?;
            return self.explicit_cast(e, ty, loc);
        }
        self.unary()
    }

    pub(super) fn explicit_cast(&mut self, e: Expr, ty: Type, loc: Loc) -> Result<Expr> {
        let e = self.rval(e);
        if ty.is_void() {
            return Ok(Expr::new(ExprKind::Cast(Box::new(e)), ty, loc));
        }
        if !ty.is_scalar() {
            if ty.same(&e.ty) {
                return Ok(e);
            }
            return err(loc, format!("cannot cast to non-scalar type '{}'", ty));
        }
        if !e.ty.is_scalar() {
            return err(loc, format!("cannot cast from type '{}'", e.ty));
        }
        if e.ty.is_pointer() && ty.is_pointer() {
            let explicit_space = ty.pointee().unwrap().q.space.is_some();
            if !explicit_space {
                let (a, b) = (e.ty.clone(), ty.clone());
                // Only unify the outer level: the pointee type may be reinterpreted.
                if let (Some(va), Some(vb)) = (a.space_var(), b.space_var()) {
                    if !a.is_func_ptr() && !b.is_func_ptr() {
                        self.prog.spaces.union(va, vb);
                    }
                }
                if let (Some(pa), Some(pb)) = (a.pointee(), b.pointee()) {
                    if pa.is_pointer() && pb.is_pointer() {
                        let (pa, pb) = (pa.clone(), pb.clone());
                        self.unify(&pa, &pb);
                    }
                }
            }
        }
        let mut ty = ty;
        // Casts produce rvalues without qualifiers.
        ty.q.is_const = false;
        ty.q.is_volatile = false;
        Ok(self.conv_raw(e, &ty))
    }

    fn unary(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let t = self.peek().clone();
        match t {
            Tok::Punct("+") => {
                self.pos += 1;
                let e = self.cast_expr()?;
                let e = self.rval(e);
                if !e.ty.is_arith() {
                    return err(loc, "invalid argument type to unary '+'");
                }
                Ok(self.promote(e))
            }
            Tok::Punct("-") => {
                self.pos += 1;
                let e = self.cast_expr()?;
                let e = self.rval(e);
                if !e.ty.is_arith() {
                    return err(loc, "invalid argument type to unary '-'");
                }
                let e = self.promote(e);
                let ty = e.ty.clone();
                if let Some(v) = e.as_int() {
                    return Ok(Expr::int(fold_unary(UnOp::Neg, v, &ty), ty, loc));
                }
                if let ExprKind::Float(f) = e.kind {
                    return Ok(Expr::new(ExprKind::Float(-f), ty, loc));
                }
                Ok(Expr::new(ExprKind::Unary(UnOp::Neg, Box::new(e)), ty, loc))
            }
            Tok::Punct("~") => {
                self.pos += 1;
                let e = self.cast_expr()?;
                let e = self.rval(e);
                if !e.ty.is_integer() {
                    return err(loc, "invalid argument type to unary '~'");
                }
                let e = self.promote(e);
                let ty = e.ty.clone();
                if let Some(v) = e.as_int() {
                    return Ok(Expr::int(fold_unary(UnOp::BitNot, v, &ty), ty, loc));
                }
                Ok(Expr::new(ExprKind::Unary(UnOp::BitNot, Box::new(e)), ty, loc))
            }
            Tok::Punct("!") => {
                self.pos += 1;
                let e = self.cast_expr()?;
                let e = self.rval(e);
                self.check_scalar(&e, loc)?;
                if let Some(v) = e.as_int() {
                    return Ok(Expr::int((v == 0) as i64, Type::int(), loc));
                }
                Ok(Expr::new(ExprKind::Unary(UnOp::LogNot, Box::new(e)), Type::int(), loc))
            }
            Tok::Punct("&") => {
                self.pos += 1;
                let e = self.cast_expr()?;
                self.addr_of(e, loc)
            }
            Tok::Punct("&&") => err(loc, "address of label is not supported"),
            Tok::Punct("*") => {
                self.pos += 1;
                let e = self.cast_expr()?;
                self.deref(e, loc)
            }
            Tok::Punct("++") | Tok::Punct("--") => {
                self.pos += 1;
                let e = self.unary()?;
                self.incdec(e, t == Tok::Punct("++"), false, loc)
            }
            Tok::Kw(Kw::Sizeof) => {
                self.pos += 1;
                let ty = if self.is_p("(") && self.is_typename_tok(&self.peek_at(1).clone()) {
                    let save = self.pos;
                    self.pos += 1;
                    let ty = self.type_name()?;
                    self.expect_p(")")?;
                    if self.is_p("{") {
                        self.pos = save;
                        self.in_sizeof += 1;
                        let e = self.unary();
                        self.in_sizeof -= 1;
                        e?.ty
                    } else {
                        ty
                    }
                } else {
                    self.in_sizeof += 1;
                    let e = self.unary();
                    self.in_sizeof -= 1;
                    let e = e?;
                    if let ExprKind::Global(g) = e.kind {
                        if self.prog.globals[g].is_string {
                            // keep
                        }
                    }
                    e.ty
                };
                if ty.is_bit() {
                    return Ok(Expr::int(1, Type::uint(), loc));
                }
                match self.prog.sizeof(&ty) {
                    Some(s) if !ty.is_func() => Ok(Expr::int(s as i64, Type::uint(), loc)),
                    _ => err(loc, format!("invalid application of 'sizeof' to incomplete type '{}'", ty)),
                }
            }
            Tok::Kw(Kw::Alignof) => {
                self.pos += 1;
                if self.is_p("(") && self.is_typename_tok(&self.peek_at(1).clone()) {
                    self.pos += 1;
                    self.type_name()?;
                    self.expect_p(")")?;
                } else {
                    self.in_sizeof += 1;
                    let e = self.unary();
                    self.in_sizeof -= 1;
                    e?;
                }
                Ok(Expr::int(1, Type::uint(), loc))
            }
            Tok::Kw(Kw::Extension) => {
                self.pos += 1;
                self.cast_expr()
            }
            _ => self.postfix(),
        }
    }

    pub(super) fn incdec(&mut self, e: Expr, inc: bool, post: bool, loc: Loc) -> Result<Expr> {
        self.check_lvalue(&e, loc)?;
        let delta = if e.ty.is_pointer() {
            self.elem_size(&e.ty.clone(), loc)? as i64
        } else if e.ty.is_arith() {
            1
        } else {
            return err(loc, "cannot increment value of this type");
        };
        let delta = if inc { delta } else { -delta };
        let ty = e.ty.unqual_keep_space();
        Ok(Expr::new(ExprKind::IncDec(Box::new(e), delta, post), ty, loc))
    }

    pub(super) fn lvalue_space(&self, e: &Expr) -> LSpace {
        match &e.kind {
            ExprKind::Global(g) => LSpace::Concrete(self.prog.globals[*g].space),
            ExprKind::Local(l) => {
                let ctx = self.fctx.as_ref().unwrap();
                LSpace::Concrete(ctx.locals[*l].space.unwrap_or(Space::Data))
            }
            ExprKind::Deref(p) => match &p.ty.kind {
                TypeKind::Pointer(_, v) => LSpace::Var(*v),
                _ => LSpace::Concrete(Space::Data),
            },
            ExprKind::Member(b, _) => self.lvalue_space(b),
            ExprKind::StmtExpr(_, Some(e)) => self.lvalue_space(e),
            _ => LSpace::Concrete(Space::Data),
        }
    }

    /// A pointer type that is always generic (several target spaces).
    pub(super) fn generic_ptr_to(&mut self, t: Type) -> Type {
        let v = self.prog.spaces.new_var(Some(Space::Data));
        self.prog.spaces.add_space(v, Space::Code);
        self.prog.spaces.add_space(v, Space::Xdata);
        Type::new(TypeKind::Pointer(Rc::new(t), v))
    }

    fn mark_addr_taken(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Global(g) => self.prog.globals[*g].addr_taken = true,
            ExprKind::Local(l) => {
                if let Some(ctx) = self.fctx.as_mut() {
                    ctx.locals[*l].addr_taken = true;
                }
            }
            ExprKind::Member(b, _) => self.mark_addr_taken(&b.clone()),
            ExprKind::StmtExpr(_, Some(e)) => self.mark_addr_taken(&e.clone()),
            ExprKind::Func(f) => self.prog.funcs[*f].addr_taken = true,
            _ => {}
        }
    }

    pub(super) fn addr_of(&mut self, e: Expr, loc: Loc) -> Result<Expr> {
        if let ExprKind::Func(_) = e.kind {
            self.mark_addr_taken(&e);
            let ty = self.ptr_to_in(e.ty.clone(), Space::Code);
            return Ok(Expr::new(ExprKind::AddrOf(Box::new(e)), ty, loc));
        }
        if let ExprKind::Deref(p) = e.kind {
            // &*p == p (with the pointee type of the lvalue)
            let p = *p;
            return Ok(p);
        }
        if !self.is_lvalue(&e) {
            return err(loc, "cannot take the address of an rvalue");
        }
        if e.ty.is_bit() {
            return err(loc, "cannot take the address of a bit variable");
        }
        if let ExprKind::Member(_, fi) = &e.kind {
            if let ExprKind::Member(b, _) = &e.kind {
                if let TypeKind::Record(r) = b.ty.kind {
                    if self.prog.records[r].fields[*fi].bits.is_some() {
                        return err(loc, "cannot take the address of a bit-field");
                    }
                }
            }
        }
        if let ExprKind::Global(g) = &e.kind {
            if self.prog.globals[*g].space == Space::Sfr || self.prog.globals[*g].space == Space::Sbit {
                return err(loc, "cannot take the address of a special function register");
            }
        }
        if self.in_sizeof == 0 {
            self.mark_addr_taken(&e);
        }
        let ty = match self.lvalue_space(&e) {
            LSpace::Concrete(s) => self.ptr_to_in(e.ty.clone(), s),
            LSpace::Var(v) => Type::new(TypeKind::Pointer(Rc::new(e.ty.clone()), v)),
        };
        Ok(Expr::new(ExprKind::AddrOf(Box::new(e)), ty, loc))
    }

    pub(super) fn deref(&mut self, e: Expr, loc: Loc) -> Result<Expr> {
        let e = self.rval(e);
        match &e.ty.kind {
            TypeKind::Pointer(p, _) => {
                let p = (**p).clone();
                if p.is_func() {
                    // *fp == fp
                    return Ok(e);
                }
                if p.is_void() {
                    diag::warn(loc, "dereferencing 'void *' pointer");
                }
                Ok(Expr::new(ExprKind::Deref(Box::new(e)), p, loc))
            }
            _ => err(loc, format!("indirection requires pointer operand ('{}' invalid)", e.ty)),
        }
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            let loc = self.loc();
            if self.eat_p("[") {
                let idx = self.expr()?;
                self.expect_p("]")?;
                let sum = self.make_binary(BinOp::Add, e, idx, loc)?;
                if !sum.ty.is_pointer() {
                    return err(loc, "subscripted value is not an array or pointer");
                }
                e = self.deref(sum, loc)?;
                continue;
            }
            if self.eat_p("(") {
                e = self.call(e, loc)?;
                continue;
            }
            if self.eat_p(".") {
                let name = self.expect_ident()?;
                e = self.member(e, &name, loc)?;
                continue;
            }
            if self.eat_p("->") {
                let name = self.expect_ident()?;
                let d = self.deref(e, loc)?;
                e = self.member(d, &name, loc)?;
                continue;
            }
            if self.eat_p("++") {
                e = self.incdec(e, true, true, loc)?;
                continue;
            }
            if self.eat_p("--") {
                e = self.incdec(e, false, true, loc)?;
                continue;
            }
            return Ok(e);
        }
    }

    fn member(&mut self, e: Expr, name: &str, loc: Loc) -> Result<Expr> {
        let TypeKind::Record(rid) = e.ty.kind else {
            return err(loc, format!("member reference base type '{}' is not a structure or union", e.ty));
        };
        if !self.prog.records[rid].complete {
            return err(loc, "member access into incomplete type");
        }
        // Search including anonymous members.
        let path = self.find_member(rid, name).ok_or_else(|| error(loc, format!("no member named '{}'", name)))?;
        let mut cur = e;
        for idx in path {
            let TypeKind::Record(r) = cur.ty.kind else { unreachable!() };
            let f = &self.prog.records[r].fields[idx];
            let mut fty = f.ty.clone();
            fty.q.is_const |= cur.ty.q.is_const;
            fty.q.is_volatile |= cur.ty.q.is_volatile;
            if fty.q.space.is_none() {
                fty.q.space = cur.ty.q.space;
            }
            cur = Expr::new(ExprKind::Member(Box::new(cur), idx), fty, loc);
        }
        Ok(cur)
    }

    fn find_member(&self, rid: RecordId, name: &str) -> Option<Vec<usize>> {
        for (i, f) in self.prog.records[rid].fields.iter().enumerate() {
            match &f.name {
                Some(n) if &**n == name => return Some(vec![i]),
                None => {
                    if let TypeKind::Record(sub) = f.ty.kind {
                        if let Some(mut p) = self.find_member(sub, name) {
                            p.insert(0, i);
                            return Some(p);
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn call(&mut self, callee: Expr, loc: Loc) -> Result<Expr> {
        let callee = if let ExprKind::Func(_) = callee.kind { callee } else { self.rval(callee) };
        let Some(ft) = callee.ty.func().cloned() else {
            return err(loc, format!("called object type '{}' is not a function or function pointer", callee.ty));
        };
        let mut args = Vec::new();
        // Variadic arguments explicitly cast to a char type are passed unpromoted (SDCC).
        let mut char_cast = Vec::new();
        if !self.is_p(")") {
            loop {
                let starts_with_cast = ft.variadic && args.len() >= ft.params.len() && self.is_p("(") && self.is_typename_tok(&self.peek_at(1).clone());
                let a = self.assign()?;
                // An explicit cast suppresses the SDCC-specific promotion of the argument.
                let char_arg = !self.iso_std && a.ty.is_integer() && self.prog.size(&a.ty) == 1 && !a.ty.is_bool();
                let pinned_ptr = a.ty.space_var().map_or(false, |v| self.prog.spaces.is_pinned(v));
                char_cast.push(starts_with_cast && (char_arg || pinned_ptr));
                args.push(a);
                if !self.eat_p(",") {
                    break;
                }
            }
        }
        self.expect_p(")")?;
        if !ft.unprototyped {
            if args.len() < ft.params.len() || (args.len() > ft.params.len() && !ft.variadic) {
                return err(loc, format!("function call expects {} arguments, got {}", ft.params.len(), args.len()));
            }
        }
        let mut out = Vec::new();
        for (i, a) in args.into_iter().enumerate() {
            if i < ft.params.len() {
                let pt = ft.params[i].clone();
                out.push(self.assign_conv(a, &pt, loc)?);
            } else {
                let a = self.rval(a);
                let a = if a.ty.is_float() {
                    self.conv(a, &Type::new(TypeKind::Float))
                } else if a.ty.is_integer() && !char_cast[i] {
                    self.promote(a)
                } else if a.ty.is_pointer() && !a.ty.is_func_ptr() && !char_cast[i] {
                    // Data pointers are passed as generic pointers unless explicitly cast (SDCC).
                    let g = self.generic_ptr_to(a.ty.pointee().unwrap().clone());
                    self.conv(a, &g)
                } else {
                    a
                };
                out.push(a);
            }
        }
        let ret = ft.ret.clone();
        Ok(Expr::new(ExprKind::Call(Box::new(callee), out), ret, loc))
    }

    fn primary(&mut self) -> Result<Expr> {
        let loc = self.loc();
        let t = self.next();
        match t.tok {
            Tok::Int(v, ty) => {
                use crate::lex::IntLitTy as L;
                let ty = match ty {
                    L::Int | L::Char => Type::int(),
                    L::UInt => Type::uint(),
                    L::Long => Type::long(),
                    L::ULong => Type::ulong(),
                    L::LongLong => Type::llong(),
                    L::ULongLong => Type::ullong(),
                };
                let v = if matches!(ty.kind, TypeKind::Int(IntKind::Int, true)) && matches!(t.tok, Tok::Int(_, L::Char)) {
                    v as i64
                } else {
                    v as i64
                };
                Ok(Expr::int(norm_int(v, &ty), ty, loc))
            }
            Tok::Float(v, is_f) => {
                let _ = is_f;
                Ok(Expr::new(ExprKind::Float(v), Type::new(TypeKind::Float), loc))
            }
            Tok::Str(bytes, width) => {
                let g = self.string_global(&bytes, width);
                let ty = self.prog.globals[g].ty.clone();
                Ok(Expr::new(ExprKind::Global(g), ty, loc))
            }
            Tok::Punct("(") => {
                if self.is_p("{") {
                    // GNU statement expression.
                    return self.stmt_expr(loc);
                }
                if self.is_typename() {
                    // Compound literal.
                    let ty = self.type_name()?;
                    self.expect_p(")")?;
                    return self.compound_literal(ty, loc);
                }
                let e = self.expr()?;
                self.expect_p(")")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                if &*name == "__func__" || &*name == "__FUNCTION__" {
                    let fname = self.fctx.as_ref().map(|c| self.prog.funcs[c.id].name.to_string()).unwrap_or_default();
                    let g = self.string_global(fname.as_bytes(), 1);
                    let ty = self.prog.globals[g].ty.clone();
                    return Ok(Expr::new(ExprKind::Global(g), ty, loc));
                }
                match self.lookup(&name).cloned() {
                    Some(Entry::Local(l)) => {
                        let ty = self.fctx.as_ref().unwrap().locals[l].ty.clone();
                        Ok(Expr::new(ExprKind::Local(l), ty, loc))
                    }
                    Some(Entry::Global(g)) => {
                        let ty = self.prog.globals[g].ty.clone();
                        Ok(Expr::new(ExprKind::Global(g), ty, loc))
                    }
                    Some(Entry::Func(f)) => {
                        let ty = self.prog.funcs[f].ty.clone();
                        Ok(Expr::new(ExprKind::Func(f), ty, loc))
                    }
                    Some(Entry::EnumConst(v)) => Ok(Expr::int(v, Type::int(), loc)),
                    Some(Entry::Typedef(_)) => err(loc, format!("unexpected type name '{}'", name)),
                    None if matches!(&*name, "true" | "false" | "nullptr") => {
                        // C23 constants, unless the program declares them itself (C89 code does).
                        Ok(Expr::int((&*name == "true") as i64, Type::int(), loc))
                    }
                    None => {
                        if self.is_p("(") {
                            if let Some(e) = self.builtin_call(&name, loc)? {
                                return Ok(e);
                            }
                            diag::warn(loc, format!("implicit declaration of function '{}'", name));
                            let ft = FuncType { ret: Type::int(), params: vec![], variadic: false, unprototyped: true, attrs: FuncAttrs::default() };
                            let spec = DeclSpec {
                                ty: Type::int(),
                                storage: Storage::Extern,
                                inline: false,
                                noreturn: false,
                                at: None,
                                fattrs: FuncAttrs::default(),
                                special: None,
                            };
                            let fid = self.declare_func(name.clone(), Type::new(TypeKind::Func(Rc::new(ft))), &spec, loc, false)?;
                            let ty = self.prog.funcs[fid].ty.clone();
                            return Ok(Expr::new(ExprKind::Func(fid), ty, loc));
                        }
                        err(loc, format!("use of undeclared identifier '{}'", name))
                    }
                }
            }
            Tok::Kw(Kw::Generic) => self.generic_selection(loc),
            Tok::Kw(Kw::BuiltinOffsetof) => {
                self.expect_p("(")?;
                let ty = self.type_name()?;
                self.expect_p(",")?;
                let mut off: u32 = 0;
                let mut cur = ty;
                loop {
                    let name = self.expect_ident()?;
                    let TypeKind::Record(rid) = cur.kind else { return err(loc, "offsetof on non-record type") };
                    let path = self.find_member(rid, &name).ok_or_else(|| error(loc, format!("no member named '{}'", name)))?;
                    let mut r = rid;
                    for idx in path {
                        let f = &self.prog.records[r].fields[idx];
                        off += f.offset;
                        cur = f.ty.clone();
                        if let TypeKind::Record(nr) = cur.kind {
                            r = nr;
                        }
                    }
                    loop {
                        if self.eat_p("[") {
                            let i = self.const_int_expr()?;
                            self.expect_p("]")?;
                            let Some(e) = cur.pointee().cloned() else { return err(loc, "subscript of non-array in offsetof") };
                            off += (i as u32) * self.prog.size(&e);
                            cur = e;
                        } else {
                            break;
                        }
                    }
                    if !self.eat_p(".") {
                        break;
                    }
                }
                self.expect_p(")")?;
                Ok(Expr::int(off as i64, Type::uint(), loc))
            }
            Tok::Kw(Kw::BuiltinVaArg) => {
                self.expect_p("(")?;
                let ap = self.assign()?;
                self.expect_p(",")?;
                let ty = self.type_name()?;
                self.expect_p(")")?;
                // Pointers travel through variable arguments as generic pointers.
                if let TypeKind::Pointer(p, v) = &ty.kind {
                    if !p.is_func() {
                        self.prog.spaces.add_space(*v, Space::Data);
                        self.prog.spaces.add_space(*v, Space::Code);
                    }
                }
                Ok(Expr::new(ExprKind::Builtin(Builtin::VaArg, vec![ap]), ty, loc))
            }
            _ => {
                self.pos -= 1;
                err(loc, format!("expected expression but found {}", self.describe()))
            }
        }
    }

    fn builtin_call(&mut self, name: &str, loc: Loc) -> Result<Option<Expr>> {
        match name {
            "__builtin_expect" => {
                self.expect_p("(")?;
                let e = self.assign()?;
                self.expect_p(",")?;
                self.assign()?;
                self.expect_p(")")?;
                Ok(Some(e))
            }
            "__builtin_constant_p" => {
                self.expect_p("(")?;
                self.in_sizeof += 1;
                let e = self.assign();
                self.in_sizeof -= 1;
                let e = e?;
                self.expect_p(")")?;
                let v = matches!(self.eval_const(&e), Some(ConstVal::Int(_)));
                Ok(Some(Expr::int(v as i64, Type::int(), loc)))
            }
            // Whole-program query used by the library: does any variadic call pass a float?
            "__builtin_float_varargs" => {
                self.expect_p("(")?;
                self.expect_p(")")?;
                let v = self.prog.has_float_varargs();
                Ok(Some(Expr::int(v as i64, Type::int(), loc)))
            }
            "__builtin_inff" | "__builtin_huge_valf" | "__builtin_inf" | "__builtin_huge_val" => {
                self.expect_p("(")?;
                self.expect_p(")")?;
                Ok(Some(Expr::new(ExprKind::Float(f64::INFINITY), Type::new(TypeKind::Float), loc)))
            }
            "__builtin_nanf" | "__builtin_nan" => {
                self.expect_p("(")?;
                while !self.is_p(")") && !self.at_eof() {
                    self.pos += 1;
                }
                self.expect_p(")")?;
                Ok(Some(Expr::new(ExprKind::Float(f64::NAN), Type::new(TypeKind::Float), loc)))
            }
            "__builtin_unreachable" => {
                self.expect_p("(")?;
                self.expect_p(")")?;
                Ok(Some(Expr::new(ExprKind::Cast(Box::new(Expr::int(0, Type::int(), loc))), Type::void(), loc)))
            }
            "__builtin_va_start" | "va_start" => {
                self.expect_p("(")?;
                let ap = self.assign()?;
                self.expect_p(",")?;
                let _last = self.assign()?;
                self.expect_p(")")?;
                Ok(Some(Expr::new(ExprKind::Builtin(Builtin::VaStart, vec![ap]), Type::void(), loc)))
            }
            "__builtin_va_end" => {
                self.expect_p("(")?;
                let ap = self.assign()?;
                self.expect_p(")")?;
                Ok(Some(Expr::new(ExprKind::Builtin(Builtin::VaEnd, vec![ap]), Type::void(), loc)))
            }
            "__builtin_va_copy" => {
                self.expect_p("(")?;
                let d = self.assign()?;
                self.expect_p(",")?;
                let s = self.assign()?;
                self.expect_p(")")?;
                let ty = d.ty.clone();
                let s = self.assign_conv(s, &ty, loc)?;
                Ok(Some(Expr::new(ExprKind::Assign(Box::new(d), Box::new(s)), ty, loc)))
            }
            _ => Ok(None),
        }
    }

    fn generic_selection(&mut self, loc: Loc) -> Result<Expr> {
        self.expect_p("(")?;
        self.in_sizeof += 1;
        let ctrl = self.assign();
        self.in_sizeof -= 1;
        let ctrl = self.rval(ctrl?);
        let cty = ctrl.ty.unqual();
        let mut chosen = None;
        let mut default = None;
        while self.eat_p(",") {
            if self.eat_kw(Kw::Default) {
                self.expect_p(":")?;
                default = Some(self.assign()?);
                continue;
            }
            let t = self.type_name()?;
            self.expect_p(":")?;
            let e = self.assign()?;
            // Pointer types also have to agree on their target space.
            let same_space = match (t.space_var(), cty.space_var()) {
                (Some(a), Some(b)) => {
                    let sp = |v| self.prog.spaces.is_pinned(v).then(|| self.prog.spaces.resolve(v)).flatten();
                    sp(a) == sp(b)
                }
                _ => true,
            };
            if chosen.is_none() && same_space && t.same(&cty) && t.is_integer() == cty.is_integer() && t.is_signed() == cty.is_signed() {
                chosen = Some(e);
            }
        }
        self.expect_p(")")?;
        chosen.or(default).ok_or_else(|| error(loc, "controlling expression type not compatible with any generic association"))
    }

    fn stmt_expr(&mut self, loc: Loc) -> Result<Expr> {
        if self.fctx.is_none() {
            return err(loc, "statement expression not allowed at file scope");
        }
        self.expect_p("{")?;
        self.push_scope();
        let mut stmts = Vec::new();
        let mut last: Option<Expr> = None;
        while !self.is_p("}") {
            if let Some(e) = last.take() {
                stmts.push(Stmt::Expr(e));
            }
            // Expression statement at the end gives the value.
            if !self.is_typename() && !matches!(self.peek(), Tok::Kw(_)) && !self.is_p("{") && !self.is_p(";") && !(matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Punct(":"))) {
                let e = self.expr()?;
                self.expect_p(";")?;
                last = Some(e);
                continue;
            }
            self.block_item(&mut stmts)?;
        }
        self.expect_p("}")?;
        self.expect_p(")")?;
        self.pop_scope();
        let (ty, last) = match last {
            Some(e) => {
                let e = self.rval(e);
                (e.ty.clone(), Some(Box::new(e)))
            }
            None => (Type::void(), None),
        };
        Ok(Expr::new(ExprKind::StmtExpr(stmts, last), ty, loc))
    }

    fn compound_literal(&mut self, ty: Type, loc: Loc) -> Result<Expr> {
        if self.fctx.is_none() || self.in_sizeof > 0 && self.fctx.is_none() {
            // File scope: anonymous global.
            let (items, fty) = self.initializer(&ty)?;
            let data = self.eval_static_init(&fty, &items, loc)?;
            let id = self.prog.globals.len();
            let space = if fty.q.is_const { Space::Code } else { Space::Data };
            self.prog.globals.push(Global {
                name: format!("__compound_literal_{}", id).into(),
                ty: fty.clone(),
                linkage: Linkage::Internal,
                space,
                init: Some(data),
                defined: true,
                at: None,
                loc,
                addr_taken: false,
                is_string: false,
                tentative: false,
                is_extern_decl: false,
                volatile: false,
                tu: self.tu,
            });
            return Ok(Expr::new(ExprKind::Global(id), fty, loc));
        }
        let name: Rc<str> = format!("__compound_literal_{}", self.anon_counter).into();
        self.anon_counter += 1;
        let lid = self.new_local(name, ty.clone(), loc, false, None);
        let (items, fty) = self.initializer(&ty)?;
        self.fctx.as_mut().unwrap().locals[lid].ty = fty.clone();
        let inits = self.local_init_list(&fty, items)?;
        let v = Expr::new(ExprKind::Local(lid), fty.clone(), loc);
        Ok(Expr::new(ExprKind::StmtExpr(vec![Stmt::InitLocal(lid, inits, loc)], Some(Box::new(v))), fty, loc))
    }

    // ------------------------------------------------------------------
    // Conversions

    pub(super) fn is_lvalue(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Global(_) | ExprKind::Local(_) | ExprKind::Deref(_) => true,
            ExprKind::Member(b, _) => self.is_lvalue(b),
            ExprKind::StmtExpr(_, Some(v)) => matches!(v.kind, ExprKind::Local(_)) && v.ty.is_record() || matches!(v.kind, ExprKind::Local(_)),
            _ => false,
        }
    }

    fn check_lvalue(&self, e: &Expr, loc: Loc) -> Result<()> {
        if !self.is_lvalue(e) {
            return err(loc, "expression is not assignable");
        }
        if e.ty.q.is_const {
            diag::warn(loc, "assignment to const-qualified object");
        }
        Ok(())
    }

    fn check_scalar(&self, e: &Expr, loc: Loc) -> Result<()> {
        if !e.ty.is_scalar() {
            return err(loc, format!("used type '{}' where arithmetic or pointer type is required", e.ty));
        }
        Ok(())
    }

    /// Array/function decay.
    pub(super) fn rval(&mut self, e: Expr) -> Expr {
        match &e.ty.kind {
            TypeKind::Array(elem, _) => {
                let loc = e.loc;
                let elem = (**elem).clone();
                if self.in_sizeof == 0 {
                    self.mark_addr_taken(&e);
                }
                let ty = match self.lvalue_space(&e) {
                    LSpace::Concrete(s) => self.ptr_to_in(elem, s),
                    LSpace::Var(v) => Type::new(TypeKind::Pointer(Rc::new(elem), v)),
                };
                Expr::new(ExprKind::AddrOf(Box::new(e)), ty, loc)
            }
            TypeKind::Func(_) => {
                let loc = e.loc;
                if self.in_sizeof == 0 {
                    self.mark_addr_taken(&e);
                }
                let ty = self.ptr_to_in(e.ty.clone(), Space::Code);
                Expr::new(ExprKind::AddrOf(Box::new(e)), ty, loc)
            }
            _ => e,
        }
    }

    pub(super) fn is_null_const(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Int(0) => e.ty.is_integer(),
            ExprKind::Cast(inner) => e.ty.pointee().map_or(false, |p| p.is_void()) && self.is_null_const(inner),
            _ => false,
        }
    }

    pub(super) fn promoted_type(&self, t: &Type) -> Type {
        match t.int_kind() {
            Some((k, s)) if Type::rank(k) < Type::rank(IntKind::Int) => {
                if !s && Type::int_size(k) == Type::int_size(IntKind::Int) {
                    Type::uint()
                } else {
                    Type::int()
                }
            }
            Some((k, s)) => Type::intk(k, s),
            None => t.unqual(),
        }
    }

    /// Promoted type of an expression. A bit-field narrower than int promotes to int,
    /// whatever its declared type.
    pub(super) fn promoted_expr_type(&self, e: &Expr) -> Type {
        if let ExprKind::Member(b, fi) = &e.kind {
            if let TypeKind::Record(rid) = b.ty.kind {
                if let Some((_, w)) = self.prog.records[rid].fields[*fi].bits {
                    if e.ty.is_integer() && !e.ty.is_bool() && (w as u32) < 8 * Type::int_size(IntKind::Int) {
                        return Type::int();
                    }
                }
            }
        }
        self.promoted_type(&e.ty)
    }

    pub(super) fn promote(&mut self, e: Expr) -> Expr {
        let t = self.promoted_expr_type(&e);
        self.conv(e, &t)
    }

    pub(super) fn common_type(&self, a: &Type, b: &Type) -> Type {
        if a.is_float() || b.is_float() {
            return Type::new(TypeKind::Float);
        }
        let (ka, sa) = a.int_kind().unwrap();
        let (kb, sb) = b.int_kind().unwrap();
        if ka == kb && sa == sb {
            return Type::intk(ka, sa);
        }
        if sa == sb {
            return if Type::rank(ka) >= Type::rank(kb) { Type::intk(ka, sa) } else { Type::intk(kb, sb) };
        }
        let (uk, sk) = if !sa { (ka, kb) } else { (kb, ka) };
        if Type::rank(uk) >= Type::rank(sk) {
            return Type::intk(uk, false);
        }
        if Type::int_size(sk) > Type::int_size(uk) {
            return Type::intk(sk, true);
        }
        Type::intk(sk, false)
    }

    pub(super) fn arith_conv(&mut self, l: Expr, r: Expr) -> (Expr, Expr, Type) {
        let pl = self.promoted_expr_type(&l);
        let pr = self.promoted_expr_type(&r);
        let t = self.common_type(&pl, &pr);
        let l = self.conv(l, &t);
        let r = self.conv(r, &t);
        (l, r, t)
    }

    /// Implicit conversion (no qualifier changes needed).
    pub(super) fn conv(&mut self, e: Expr, ty: &Type) -> Expr {
        // Pointers in distinct space classes (pinned) need an explicit conversion.
        let distinct_spaces = match (e.ty.space_var(), ty.space_var()) {
            (Some(a), Some(b)) => self.prog.spaces.find_const(a) != self.prog.spaces.find_const(b),
            _ => false,
        };
        if e.ty.same(ty) && e.ty.is_signed() == ty.is_signed() && !e.ty.is_array() && !distinct_spaces {
            if let (TypeKind::Enum(..), TypeKind::Int(..)) | (TypeKind::Int(..), TypeKind::Enum(..)) = (&e.ty.kind, &ty.kind) {
                // fallthrough to relabel type
            } else {
                return e;
            }
        }
        let mut ty = ty.clone();
        ty.q = Quals::default();
        self.conv_raw(e, &ty)
    }

    fn conv_raw(&mut self, e: Expr, ty: &Type) -> Expr {
        let loc = e.loc;
        if ty.is_void() {
            return Expr::new(ExprKind::Cast(Box::new(e)), ty.clone(), loc);
        }
        if let Some(v) = e.as_int() {
            if ty.is_integer() {
                return Expr::int(fold_cast(v, &e.ty, ty), ty.clone(), loc);
            }
            if ty.is_float() {
                let f = if e.ty.is_signed() { v as f64 } else { v as u64 as f64 };
                return Expr::new(ExprKind::Float(f), ty.clone(), loc);
            }
        }
        if let ExprKind::Float(f) = e.kind {
            if ty.is_integer() {
                let v = if ty.is_bool() { (f != 0.0) as i64 } else { f as i64 };
                return Expr::int(norm_int(v, ty), ty.clone(), loc);
            }
            if ty.is_float() {
                return Expr::new(ExprKind::Float(f), ty.clone(), loc);
            }
        }
        // Collapse nested integer casts where the inner one is a widening that doesn't matter.
        Expr::new(ExprKind::Cast(Box::new(e)), ty.clone(), loc)
    }

    /// Conversion as if by assignment.
    pub(super) fn assign_conv(&mut self, e: Expr, ty: &Type, loc: Loc) -> Result<Expr> {
        let e = self.rval(e);
        if ty.is_record() {
            if !e.ty.same(ty) {
                return err(loc, format!("assigning to '{}' from incompatible type '{}'", ty, e.ty));
            }
            return Ok(e);
        }
        if ty.is_pointer() {
            if e.ty.is_pointer() {
                let et = e.ty.clone();
                let pa = ty.pointee().unwrap();
                let pb = et.pointee().unwrap();
                if !pa.is_void() && !pb.is_void() && !self.prog.compatible(pa, pb) && !(pa.is_integer() && pb.is_integer() && Type::int_size(pa.int_kind().unwrap().0) == Type::int_size(pb.int_kind().unwrap().0)) {
                    diag::warn(loc, format!("incompatible pointer types assigning to '{}' from '{}'", ty, et));
                }
                self.unify(ty, &et);
                return Ok(self.conv(e, ty));
            }
            if e.ty.is_integer() {
                if !self.is_null_const(&e) {
                    diag::warn(loc, format!("incompatible integer to pointer conversion assigning to '{}'", ty));
                }
                return Ok(self.conv(e, ty));
            }
            return err(loc, format!("assigning to '{}' from incompatible type '{}'", ty, e.ty));
        }
        if ty.is_arith() {
            if e.ty.is_pointer() {
                if !ty.is_bool() {
                    diag::warn(loc, format!("incompatible pointer to integer conversion assigning to '{}'", ty));
                }
                return Ok(self.conv(e, ty));
            }
            if !e.ty.is_arith() {
                return err(loc, format!("assigning to '{}' from incompatible type '{}'", ty, e.ty));
            }
            return Ok(self.conv(e, ty));
        }
        err(loc, format!("assigning to '{}' from incompatible type '{}'", ty, e.ty))
    }
}
