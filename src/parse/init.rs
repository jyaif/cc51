//! Initializer parsing (designators, brace elision, string literals).

use super::*;

pub(crate) struct InitItem {
    pub offset: u32,
    pub bits: Option<(u8, u8)>,
    pub kind: InitKind,
}

pub(crate) enum InitKind {
    Expr(Expr),
    Bytes(Vec<u8>),
}

fn is_char_type(t: &Type) -> bool {
    matches!(t.kind, TypeKind::Int(IntKind::Char, _))
}

/// The string literal element width an array of `t` can be initialized from, if any.
fn str_elem_width(t: &Type) -> Option<u8> {
    match t.kind {
        TypeKind::Int(IntKind::Char, _) => Some(1),
        TypeKind::Int(IntKind::Int, _) | TypeKind::Int(IntKind::Short, _) => Some(2),
        TypeKind::Int(IntKind::Long, _) => Some(4),
        _ => None,
    }
}

impl<'a> Parser<'a> {
    pub(super) fn initializer(&mut self, ty: &Type) -> Result<(Vec<InitItem>, Type)> {
        let mut items = Vec::new();
        let n = self.init_value(ty, 0, None, &mut items)?;
        let fty = match &ty.kind {
            TypeKind::Array(e, None) => {
                let mut t = Type::new(TypeKind::Array(e.clone(), Some(n)));
                t.q = ty.q;
                t
            }
            _ => ty.clone(),
        };
        Ok((items, fty))
    }

    /// Initialize an object of type `ty` at `off`. Returns the number of array elements (for arrays).
    fn init_value(&mut self, ty: &Type, off: u32, bits: Option<(u8, u8)>, items: &mut Vec<InitItem>) -> Result<u32> {
        match &ty.kind {
            TypeKind::Array(elem, n) => {
                let elem = (**elem).clone();
                let n = *n;
                if self.pending_init.is_none() && str_elem_width(&elem).is_some() {
                    let ew = str_elem_width(&elem).unwrap();
                    // String literal initializer, optionally in braces.
                    let is_str = |t: &Tok| matches!(t, Tok::Str(_, w) if *w == ew);
                    let braced_str = self.is_p("{") && is_str(self.peek_at(1)) && matches!(self.peek_at(2), Tok::Punct("}") | Tok::Punct(","));
                    if braced_str {
                        self.pos += 1;
                    }
                    if let Some(Tok::Str(s, _)) = Some(self.peek().clone()).filter(|t| is_str(t)) {
                        self.pos += 1;
                        if braced_str {
                            self.eat_p(",");
                            self.expect_p("}")?;
                        }
                        let mut bytes = s.clone();
                        bytes.extend(std::iter::repeat(0).take(ew as usize));
                        let len = match n {
                            Some(n) => {
                                bytes.truncate(n as usize * ew as usize);
                                n
                            }
                            None => bytes.len() as u32 / ew as u32,
                        };
                        items.push(InitItem { offset: off, bits: None, kind: InitKind::Bytes(bytes) });
                        return Ok(len);
                    }
                    if braced_str {
                        self.pos -= 1;
                    }
                }
                if self.pending_init.is_none() && self.eat_p("{") {
                    let c = self.init_array(&elem, n, off, items, true)?;
                    self.expect_p("}")?;
                    return Ok(c);
                }
                self.init_array(&elem, n, off, items, false)
            }
            TypeKind::Record(rid) => {
                let rid = *rid;
                if self.pending_init.is_none() && self.eat_p("{") {
                    self.init_record(rid, off, items, true)?;
                    self.expect_p("}")?;
                    return Ok(1);
                }
                let e = match self.pending_init.take() {
                    Some(e) => e,
                    None => self.assign()?,
                };
                if e.ty.is_record() && e.ty.same(ty) {
                    items.push(InitItem { offset: off, bits: None, kind: InitKind::Expr(e) });
                    return Ok(1);
                }
                self.pending_init = Some(e);
                self.init_record(rid, off, items, false)?;
                Ok(1)
            }
            _ => {
                let e = if let Some(e) = self.pending_init.take() {
                    e
                } else if self.eat_p("{") {
                    if self.is_p("}") {
                        // `{}` zero initializer
                        self.pos += 1;
                        let z = Expr::int(0, Type::int(), self.loc());
                        let loc = z.loc;
                        let z = self.assign_conv(z, ty, loc)?;
                        items.push(InitItem { offset: off, bits, kind: InitKind::Expr(z) });
                        return Ok(1);
                    }
                    let mut sub = Vec::new();
                    self.init_value(ty, off, bits, &mut sub)?;
                    items.extend(sub);
                    self.eat_p(",");
                    // Excess scalars in braces are ignored with a warning.
                    while !self.is_p("}") && !self.at_eof() {
                        diag::warn(self.loc(), "excess elements in scalar initializer");
                        self.assign()?;
                        self.eat_p(",");
                    }
                    self.expect_p("}")?;
                    return Ok(1);
                } else {
                    self.assign()?
                };
                let loc = e.loc;
                let e = self.assign_conv(e, &ty.unqual_keep_space(), loc)?;
                items.push(InitItem { offset: off, bits, kind: InitKind::Expr(e) });
                Ok(1)
            }
        }
    }

    fn at_designator(&self) -> bool {
        self.is_p("[") || (self.is_p(".") && matches!(self.peek_at(1), Tok::Ident(_)))
    }

    /// Consume the comma after an element. Returns false if the list ends here.
    fn list_continue(&mut self, braced: bool) -> bool {
        if braced {
            if self.eat_p(",") {
                return !self.is_p("}");
            }
            return false;
        }
        // Unbraced (brace elision): continue only if a plain element follows.
        if self.is_p(",") && !matches!(self.peek_at(1), Tok::Punct("}")) {
            let save = self.pos;
            self.pos += 1;
            let desig = self.at_designator();
            self.pos = save;
            return !desig;
        }
        false
    }

    fn init_array(&mut self, elem: &Type, n: Option<u32>, off: u32, items: &mut Vec<InitItem>, braced: bool) -> Result<u32> {
        let esz = self.prog.sizeof(elem).ok_or_else(|| error(self.loc(), "array has incomplete element type"))?;
        let mut idx: u32 = 0;
        let mut count: u32 = 0;
        if braced && self.is_p("}") {
            return Ok(0);
        }
        loop {
            if let Some(n) = n {
                if idx >= n && !(braced && self.is_p("[")) {
                    if braced {
                        // Excess elements.
                        if !self.is_p("}") {
                            diag::warn(self.loc(), "excess elements in array initializer");
                            while !self.is_p("}") && !self.at_eof() {
                                let mut dummy = Vec::new();
                                self.init_value(elem, 0, None, &mut dummy)?;
                                if !self.eat_p(",") {
                                    break;
                                }
                            }
                        }
                    }
                    break;
                }
            }
            if braced && self.is_p("[") {
                self.pos += 1;
                let lo = self.const_int_expr()? as u32;
                let hi = if self.eat_p("...") { self.const_int_expr()? as u32 } else { lo };
                self.expect_p("]")?;
                let start_items = items.len();
                self.designated_rest(elem, off + lo * esz, None, items)?;
                // GNU range: replicate.
                if hi > lo {
                    let new: Vec<(u32, Option<(u8, u8)>, InitKind)> = items[start_items..]
                        .iter()
                        .map(|it| {
                            (
                                it.offset,
                                it.bits,
                                match &it.kind {
                                    InitKind::Expr(e) => InitKind::Expr(e.clone()),
                                    InitKind::Bytes(b) => InitKind::Bytes(b.clone()),
                                },
                            )
                        })
                        .collect();
                    for k in lo + 1..=hi {
                        for (o, b, kd) in &new {
                            let kd = match kd {
                                InitKind::Expr(e) => InitKind::Expr(e.clone()),
                                InitKind::Bytes(b) => InitKind::Bytes(b.clone()),
                            };
                            items.push(InitItem { offset: o + (k - lo) * esz, bits: *b, kind: kd });
                        }
                    }
                }
                idx = hi + 1;
                count = count.max(idx);
            } else {
                self.init_value(elem, off + idx * esz, None, items)?;
                idx += 1;
                count = count.max(idx);
            }
            if !self.list_continue(braced) {
                break;
            }
            if !braced {
                self.pos += 1; // the comma
            }
        }
        Ok(count)
    }

    fn record_fields(&self, rid: RecordId) -> Vec<(usize, Field)> {
        self.prog.records[rid]
            .fields
            .iter()
            .enumerate()
            .filter(|(_, f)| f.name.is_some() || (f.bits.is_none() && f.ty.is_record()))
            .map(|(i, f)| (i, f.clone()))
            .collect()
    }

    fn init_record(&mut self, rid: RecordId, off: u32, items: &mut Vec<InitItem>, braced: bool) -> Result<()> {
        if !self.prog.records[rid].complete {
            return err(self.loc(), "initializer for incomplete type");
        }
        let fields = self.record_fields(rid);
        let is_union = self.prog.records[rid].is_union;
        let mut fi = 0usize;
        if braced && self.is_p("}") {
            return Ok(());
        }
        loop {
            if braced && self.is_p(".") && matches!(self.peek_at(1), Tok::Ident(_)) {
                self.pos += 1;
                let name = self.expect_ident()?;
                let path = self.find_member_path(rid, &name).ok_or_else(|| error(self.loc(), format!("field designator '{}' does not refer to any field", name)))?;
                // Position after the top-level field.
                let top = path[0];
                fi = fields.iter().position(|(i, _)| *i == top).unwrap_or(0);
                let (fty, foff, fbits) = self.path_info(rid, &path);
                self.designated_rest(&fty, off + foff, fbits, items)?;
                fi += 1;
            } else {
                if fi >= fields.len() || (is_union && fi >= 1) {
                    if braced && !self.is_p("}") {
                        diag::warn(self.loc(), "excess elements in struct initializer");
                        while !self.is_p("}") && !self.at_eof() {
                            self.assign()?;
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                    }
                    break;
                }
                let (_, f) = &fields[fi];
                let f = f.clone();
                self.init_value(&f.ty, off + f.offset, f.bits, items)?;
                fi += 1;
            }
            if !braced && (fi >= fields.len() || is_union) {
                break;
            }
            if !self.list_continue(braced) {
                break;
            }
            if !braced {
                self.pos += 1;
            }
        }
        Ok(())
    }

    fn find_member_path(&self, rid: RecordId, name: &str) -> Option<Vec<usize>> {
        for (i, f) in self.prog.records[rid].fields.iter().enumerate() {
            match &f.name {
                Some(n) if &**n == name => return Some(vec![i]),
                None => {
                    if let TypeKind::Record(sub) = f.ty.kind {
                        if let Some(mut p) = self.find_member_path(sub, name) {
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

    fn path_info(&self, rid: RecordId, path: &[usize]) -> (Type, u32, Option<(u8, u8)>) {
        let mut r = rid;
        let mut off = 0;
        let mut ty = Type::int();
        let mut bits = None;
        for &i in path {
            let f = &self.prog.records[r].fields[i];
            off += f.offset;
            ty = f.ty.clone();
            bits = f.bits;
            if let TypeKind::Record(nr) = ty.kind {
                r = nr;
            }
        }
        (ty, off, bits)
    }

    /// After a designator selected a sub-object: more designators, then `=` and the value.
    fn designated_rest(&mut self, ty: &Type, off: u32, bits: Option<(u8, u8)>, items: &mut Vec<InitItem>) -> Result<()> {
        if self.is_p("[") {
            let TypeKind::Array(elem, _) = &ty.kind else { return err(self.loc(), "array designator on non-array type") };
            let elem = (**elem).clone();
            let esz = self.prog.size(&elem);
            self.pos += 1;
            let i = self.const_int_expr()? as u32;
            self.expect_p("]")?;
            return self.designated_rest(&elem, off + i * esz, None, items);
        }
        if self.is_p(".") {
            let TypeKind::Record(rid) = ty.kind else { return err(self.loc(), "field designator on non-record type") };
            self.pos += 1;
            let name = self.expect_ident()?;
            let path = self.find_member_path(rid, &name).ok_or_else(|| error(self.loc(), format!("no member named '{}'", name)))?;
            let (fty, foff, fbits) = self.path_info(rid, &path);
            return self.designated_rest(&fty, off + foff, fbits, items);
        }
        if !self.eat_p("=") {
            // GNU obsolete syntax `[i] value` without '='.
        }
        // Designated aggregate member initialized without braces: allow brace elision.
        self.init_value(ty, off, bits, items)?;
        Ok(())
    }

    pub(super) fn local_init_list(&mut self, ty: &Type, items: Vec<InitItem>) -> Result<Vec<LocalInit>> {
        let mut out = Vec::new();
        let scalar = ty.is_scalar();
        if !scalar {
            out.push(LocalInit::Zero);
        }
        for it in items {
            match it.kind {
                InitKind::Bytes(b) => out.push(LocalInit::Bytes(it.offset, b)),
                InitKind::Expr(e) => {
                    if e.ty.is_record() {
                        out.push(LocalInit::Aggregate(it.offset, e));
                    } else {
                        out.push(LocalInit::Scalar(it.offset, it.bits, e));
                    }
                }
            }
        }
        Ok(out)
    }
}
