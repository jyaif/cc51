//! Parser for 8051 assembly text (SDCC/asxxxx-style syntax) used by inline assembly and runtime code.

use super::*;

pub struct AsmParser<'a> {
    /// Resolves a symbol name to a (possibly renamed) symbol, or a constant.
    pub resolve: &'a dyn Fn(&str) -> Option<SymRes>,
    /// Register bank (for arN names).
    pub bank: u8,
    /// Constants defined with `=` / `.equ`.
    equs: std::collections::HashMap<String, Expr>,
    here_counter: usize,
    here_used: bool,
    pub label_prefix: String,
}

pub enum SymRes {
    Sym(Rc<str>),
    Const(i64),
    Bit(i64),
}

fn builtin_dir(name: &str) -> Option<i64> {
    let up = name.to_ascii_uppercase();
    SFRS.iter().find(|(n, _)| *n == up).map(|(_, a)| *a)
}

fn builtin_bit(name: &str) -> Option<i64> {
    let up = name.to_ascii_uppercase();
    SBITS.iter().find(|(n, _)| *n == up).map(|(_, a)| *a)
}

#[derive(Clone, Debug)]
enum Val {
    E(Expr),
    Bit(i64),
}

impl<'a> AsmParser<'a> {
    pub fn new(resolve: &'a dyn Fn(&str) -> Option<SymRes>, bank: u8) -> Self {
        AsmParser { resolve, bank, equs: Default::default(), here_counter: 0, here_used: false, label_prefix: "__asm".into() }
    }

    pub fn parse(&mut self, text: &str) -> Result<Vec<Item>, String> {
        let mut items = Vec::new();
        for (ln, raw) in text.lines().enumerate() {
            self.parse_line(raw, &mut items).map_err(|e| format!("asm line {}: {}: '{}'", ln + 1, e, raw.trim()))?;
        }
        Ok(items)
    }

    fn parse_line(&mut self, raw: &str, items: &mut Vec<Item>) -> Result<(), String> {
        // Strip comments (not inside quotes).
        let mut line = String::new();
        let mut in_q: Option<char> = None;
        for c in raw.chars() {
            if let Some(q) = in_q {
                line.push(c);
                if c == q {
                    in_q = None;
                }
                continue;
            }
            if c == ';' {
                break;
            }
            if c == '"' || c == '\'' {
                in_q = Some(c);
            }
            line.push(c);
        }
        let mut s = line.trim();
        // Labels
        loop {
            let Some(colon) = s.find(':') else { break };
            let name = s[..colon].trim();
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$' || c == '.') {
                break;
            }
            let rest = s[colon + 1..].trim_start_matches(':');
            items.push(Item::Label(self.label_name(name)));
            s = rest.trim();
        }
        if s.is_empty() {
            return Ok(());
        }
        let (word, rest) = match s.find(|c: char| c.is_whitespace()) {
            Some(i) => (&s[..i], s[i..].trim()),
            None => (s, ""),
        };
        let lw = word.to_ascii_lowercase();
        // Equates: NAME = value / NAME .equ value
        if let Some(eq) = s.find('=') {
            let name = s[..eq].trim();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !s[eq..].starts_with("==") {
                let e = self.expr_str(s[eq + 1..].trim())?;
                self.equs.insert(name.to_string(), e);
                return Ok(());
            }
        }
        if rest.to_ascii_lowercase().starts_with(".equ") {
            let e = self.expr_str(rest[4..].trim())?;
            self.equs.insert(word.to_string(), e);
            return Ok(());
        }
        match lw.as_str() {
            ".area" | ".globl" | ".module" | ".optsdcc" | ".even" | ".odd" | ".include" | ".title" | ".sbttl" | ".page" | ".list" | ".nlist" | ".local" => {
                return Ok(());
            }
            ".db" | ".byte" | ".fcb" | "db" | ".ascii" | ".str" | ".asciz" | ".strz" => {
                let mut v = Vec::new();
                for part in split_args(rest) {
                    let p = part.trim();
                    if (p.starts_with('"') && p.ends_with('"') && p.len() >= 2) || ((lw.starts_with(".asci") || lw.starts_with(".str")) && p.len() >= 2) {
                        let inner = &p[1..p.len() - 1];
                        for b in unescape(inner).bytes() {
                            v.push(Expr::num(b as i64));
                        }
                    } else {
                        v.push(self.expr_str(p)?);
                    }
                }
                if lw == ".asciz" || lw == ".strz" {
                    v.push(Expr::num(0));
                }
                items.push(Item::Db(v));
                return Ok(());
            }
            ".dw" | ".word" | ".fdb" | "dw" => {
                let mut v = Vec::new();
                for part in split_args(rest) {
                    v.push(self.expr_str(part.trim())?);
                }
                // asxxxx 8051 .dw is big-endian.
                items.push(Item::Dw(v));
                return Ok(());
            }
            ".ds" | ".blkb" | ".rmb" | "ds" => {
                let e = self.expr_str(rest)?;
                let n = e.const_val().ok_or("non-constant .ds")?;
                items.push(Item::Ds(n as u32));
                return Ok(());
            }
            _ => {}
        }
        let mn = Mn::from_name(&lw).ok_or_else(|| format!("unknown mnemonic '{}'", word))?;
        let args: Vec<String> = split_args(rest).into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
        self.here_counter += 1;
        self.here_used = false;
        let insn = self.insn(mn, &args)?;
        if self.here_used {
            items.push(Item::Label(self.here_name().into()));
        }
        items.push(Item::Insn(insn));
        Ok(())
    }

    /// SDCC local labels (00101$) are local to their asm block.
    fn local_label(&self, name: &str) -> Option<String> {
        (name.ends_with('$') && name.starts_with(|c: char| c.is_ascii_digit())).then(|| format!("{}${}", self.label_prefix, name))
    }

    fn label_name(&self, name: &str) -> Rc<str> {
        if let Some(l) = self.local_label(name) {
            return l.into();
        }
        match (self.resolve)(name) {
            Some(SymRes::Sym(s)) => s,
            _ => name.into(),
        }
    }

    fn insn(&mut self, mn: Mn, args: &[String]) -> Result<Insn, String> {
        let mut ops = Vec::new();
        let n = args.len();
        for (i, a) in args.iter().enumerate() {
            let ctx = match mn {
                Mn::Acall | Mn::Ajmp | Mn::Lcall | Mn::Ljmp | Mn::Sjmp | Mn::Call | Mn::Jc | Mn::Jnc | Mn::Jz | Mn::Jnz => Ctx::Code,
                Mn::Jmp => {
                    if a.to_ascii_lowercase().replace(' ', "") == "@a+dptr" {
                        Ctx::Dir
                    } else {
                        Ctx::Code
                    }
                }
                Mn::Jb | Mn::Jnb | Mn::Jbc => {
                    if i == 0 {
                        Ctx::Bit
                    } else {
                        Ctx::Code
                    }
                }
                Mn::Cjne | Mn::Djnz if i == n - 1 => Ctx::Code,
                Mn::Setb | Mn::Clr | Mn::Cpl => Ctx::Bit,
                Mn::Mov | Mn::Anl | Mn::Orl => {
                    let other = if i == 0 { args.get(1) } else { args.get(0) };
                    if other.map_or(false, |o| o.eq_ignore_ascii_case("c")) { Ctx::Bit } else { Ctx::Dir }
                }
                _ => Ctx::Dir,
            };
            ops.push(self.operand(a, ctx)?);
        }
        // Normalise `jmp label` / `call label` to relaxable forms.
        Ok(Insn::new(mn, ops))
    }

    fn operand(&mut self, a: &str, ctx: Ctx) -> Result<Op, String> {
        let l = a.to_ascii_lowercase().replace(' ', "");
        match l.as_str() {
            "a" if ctx != Ctx::Code => return Ok(Op::A),
            "ab" => return Ok(Op::AB),
            "c" if ctx != Ctx::Code => return Ok(Op::C),
            "dptr" => return Ok(Op::Dptr),
            "@dptr" => return Ok(Op::AtDptr),
            "@a+dptr" => return Ok(Op::AtADptr),
            "@a+pc" => return Ok(Op::AtAPc),
            "@r0" => return Ok(Op::AtR(0)),
            "@r1" => return Ok(Op::AtR(1)),
            _ => {}
        }
        if l.len() == 2 && l.starts_with('r') && ctx != Ctx::Code {
            if let Some(d) = l[1..].parse::<u8>().ok().filter(|d| *d < 8) {
                return Ok(Op::R(d));
            }
        }
        if let Some(rest) = a.trim().strip_prefix('#') {
            return Ok(Op::Imm(self.expr_str(rest)?));
        }
        match ctx {
            Ctx::Code => Ok(Op::Code(self.expr_str(a)?)),
            Ctx::Bit => match self.value(a)? {
                Val::Bit(b) => Ok(Op::Bit(Expr::num(b))),
                Val::E(e) => Ok(Op::Bit(e)),
            },
            Ctx::Dir => match self.value(a)? {
                Val::Bit(_) => Err(format!("bit '{}' used where a byte address is expected", a)),
                Val::E(e) => Ok(Op::Dir(e)),
            },
        }
    }

    fn value(&mut self, s: &str) -> Result<Val, String> {
        let t = s.trim();
        // name.bit
        if let Some(dot) = t.rfind('.') {
            let (base, bit) = (&t[..dot], &t[dot + 1..]);
            if !base.is_empty() && bit.len() == 1 && bit.chars().all(|c| c.is_ascii_digit()) {
                let b: i64 = bit.parse().unwrap();
                let e = self.expr_str(base)?;
                let v = e.const_val().ok_or("bit of non-constant address")?;
                let addr = if (0x20..0x30).contains(&v) {
                    (v - 0x20) * 8 + b
                } else {
                    v + b
                };
                return Ok(Val::Bit(addr));
            }
        }
        if let Some(b) = builtin_bit(t) {
            if builtin_dir(t).is_none() {
                return Ok(Val::Bit(b));
            }
        }
        if let Some(SymRes::Bit(b)) = (self.resolve)(t) {
            return Ok(Val::Bit(b));
        }
        Ok(Val::E(self.expr_str(t)?))
    }

    pub fn expr_str(&mut self, s: &str) -> Result<Expr, String> {
        let mut toks = tokenize(s)?;
        for t in &mut toks {
            if let T::Name(n) = t {
                if let Some(l) = self.local_label(n) {
                    *n = l;
                }
            }
        }
        let mut p = ExprP { toks: &toks, pos: 0, parser: self };
        let e = p.expr(0)?;
        if p.pos != toks.len() {
            return Err(format!("unexpected '{}' in expression", toks[p.pos]));
        }
        Ok(e)
    }

    fn here_name(&self) -> String {
        format!("{}$here{}", self.label_prefix, self.here_counter)
    }

    fn lookup_name(&mut self, name: &str) -> Result<Expr, String> {
        if name == "$" {
            self.here_used = true;
            return Ok(Expr::sym(&self.here_name().into()));
        }
        if let Some(e) = self.equs.get(name) {
            return Ok(e.clone());
        }
        if let Some(d) = builtin_dir(name) {
            return Ok(Expr::num(d));
        }
        let ln = name.to_ascii_lowercase();
        if ln.len() == 3 && ln.starts_with("ar") {
            if let Some(d) = ln[2..].parse::<i64>().ok().filter(|d| *d < 8) {
                return Ok(Expr::num(self.bank as i64 * 8 + d));
            }
        }
        if let Some(b) = builtin_bit(name) {
            return Ok(Expr::num(b));
        }
        match (self.resolve)(name) {
            Some(SymRes::Sym(s)) => Ok(Expr::sym(&s)),
            Some(SymRes::Const(c)) => Ok(Expr::num(c)),
            Some(SymRes::Bit(b)) => Ok(Expr::num(b)),
            None => Ok(Expr::sym(&name.into())),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Ctx {
    Dir,
    Bit,
    Code,
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('0') => out.push('\0'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn split_args(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut depth = 0;
    let mut in_q: Option<char> = None;
    for c in s.chars() {
        if let Some(q) = in_q {
            cur.push(c);
            if c == q {
                in_q = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_q = Some(c);
                cur.push(c);
            }
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                v.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        v.push(cur);
    }
    v
}

#[derive(Clone, Debug, PartialEq)]
enum T {
    Num(i64),
    Name(String),
    P(&'static str),
}

impl fmt::Display for T {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            T::Num(n) => write!(f, "{}", n),
            T::Name(n) => write!(f, "{}", n),
            T::P(p) => write!(f, "{}", p),
        }
    }
}

fn parse_num(s: &str) -> Option<i64> {
    let l = s.to_ascii_lowercase();
    if let Some(h) = l.strip_prefix("0x") {
        return i64::from_str_radix(h, 16).ok();
    }
    if let Some(h) = l.strip_prefix("0b") {
        return i64::from_str_radix(h, 2).ok();
    }
    if let Some(h) = l.strip_prefix("0h") {
        return i64::from_str_radix(h, 16).ok();
    }
    if let Some(h) = l.strip_suffix('h') {
        if h.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            return i64::from_str_radix(h, 16).ok();
        }
    }
    if let Some(b) = l.strip_suffix('b') {
        if b.chars().all(|c| c == '0' || c == '1') && !b.is_empty() {
            return i64::from_str_radix(b, 2).ok();
        }
    }
    if l.starts_with('0') && l.len() > 1 && l.chars().all(|c| c.is_ascii_digit()) {
        // asxxxx treats leading-zero numbers as decimal unless radix given; keep decimal.
        return l.parse().ok();
    }
    l.parse().ok()
}

fn tokenize(s: &str) -> Result<Vec<T>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut v = Vec::new();
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
            continue;
        }
        if ch == '\'' {
            // character constant 'x' or 'x
            if i + 1 < c.len() {
                v.push(T::Num(c[i + 1] as i64));
                i += 2;
                if i < c.len() && c[i] == '\'' {
                    i += 1;
                }
                continue;
            }
        }
        if ch == '$' && (i + 1 >= c.len() || !c[i + 1].is_ascii_hexdigit()) {
            v.push(T::Name("$".into()));
            i += 1;
            continue;
        }
        if ch == '$' {
            let st = i + 1;
            i += 1;
            while i < c.len() && c[i].is_ascii_hexdigit() {
                i += 1;
            }
            let h: String = c[st..i].iter().collect();
            v.push(T::Num(i64::from_str_radix(&h, 16).map_err(|e| e.to_string())?));
            continue;
        }
        if ch.is_ascii_digit() {
            let st = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '$') {
                i += 1;
            }
            let w: String = c[st..i].iter().collect();
            if w.ends_with('$') {
                // SDCC local label like 00101$
                v.push(T::Name(w));
                continue;
            }
            v.push(T::Num(parse_num(&w).ok_or_else(|| format!("bad number '{}'", w))?));
            continue;
        }
        if ch.is_ascii_alphabetic() || ch == '_' || ch == '.' {
            let st = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_' || c[i] == '.' || c[i] == '$') {
                i += 1;
            }
            v.push(T::Name(c[st..i].iter().collect()));
            continue;
        }
        let two: String = c[i..(i + 2).min(c.len())].iter().collect();
        let p = match two.as_str() {
            "<<" => Some("<<"),
            ">>" => Some(">>"),
            _ => None,
        };
        if let Some(p) = p {
            v.push(T::P(p));
            i += 2;
            continue;
        }
        let p = match ch {
            '+' => "+",
            '-' => "-",
            '*' => "*",
            '/' => "/",
            '%' => "%",
            '&' => "&",
            '|' => "|",
            '^' => "^",
            '~' => "~",
            '(' => "(",
            ')' => ")",
            '<' => "<",
            '>' => ">",
            '!' => "!",
            _ => return Err(format!("unexpected character '{}'", ch)),
        };
        v.push(T::P(p));
        i += 1;
    }
    Ok(v)
}

struct ExprP<'b, 'a> {
    toks: &'b [T],
    pos: usize,
    parser: &'b mut AsmParser<'a>,
}

impl<'b, 'a> ExprP<'b, 'a> {
    fn prec(p: &str) -> Option<u8> {
        Some(match p {
            "|" => 1,
            "^" => 2,
            "&" => 3,
            "<<" | ">>" => 4,
            "+" | "-" => 5,
            "*" | "/" | "%" => 6,
            _ => return None,
        })
    }
    fn expr(&mut self, min: u8) -> Result<Expr, String> {
        let mut l = self.unary()?;
        while let Some(T::P(p)) = self.toks.get(self.pos) {
            let Some(pr) = Self::prec(p) else { break };
            if pr <= min {
                break;
            }
            let p = *p;
            self.pos += 1;
            let r = self.expr(pr)?;
            l = combine(p, l, r)?;
        }
        Ok(l)
    }
    fn unary(&mut self) -> Result<Expr, String> {
        let t = self.toks.get(self.pos).cloned().ok_or("unexpected end of expression")?;
        self.pos += 1;
        match t {
            T::Num(n) => Ok(Expr::num(n)),
            T::Name(n) => self.parser.lookup_name(&n),
            T::P("(") => {
                let e = self.expr(0)?;
                if self.toks.get(self.pos) != Some(&T::P(")")) {
                    return Err("expected ')'".into());
                }
                self.pos += 1;
                Ok(e)
            }
            T::P("-") => {
                let e = self.unary()?;
                let v = e.const_val().ok_or("negation of symbol")?;
                Ok(Expr::num(-v))
            }
            T::P("+") => self.unary(),
            T::P("~") => {
                let e = self.unary()?;
                let v = e.const_val().ok_or("complement of symbol")?;
                Ok(Expr::num(!v))
            }
            T::P("<") => Ok(self.unary()?.lo()),
            T::P(">") => Ok(self.unary()?.hi()),
            other => Err(format!("unexpected '{}' in expression", other)),
        }
    }
}

fn combine(p: &str, l: Expr, r: Expr) -> Result<Expr, String> {
    if let (Some(a), Some(b)) = (l.const_val(), r.const_val()) {
        return Ok(Expr::num(match p {
            "+" => a + b,
            "-" => a - b,
            "*" => a * b,
            "/" => {
                if b == 0 {
                    return Err("division by zero".into());
                }
                a / b
            }
            "%" => {
                if b == 0 {
                    return Err("division by zero".into());
                }
                a % b
            }
            "&" => a & b,
            "|" => a | b,
            "^" => a ^ b,
            "<<" => a << b,
            ">>" => a >> b,
            _ => unreachable!(),
        }));
    }
    match (p, l.part, r.const_val()) {
        ("+", Part::Val, Some(b)) => Ok(l.add(b)),
        ("-", Part::Val, Some(b)) => Ok(l.add(-b)),
        (">>", Part::Val, Some(8)) => Ok(l.hi()),
        (">>", Part::Val, Some(16)) => Ok(l.b2()),
        ("&", _, Some(0xff)) => Ok(if l.part == Part::Val { l.lo() } else { l }),
        _ => {
            if let (Some(a), "+", Part::Val) = (l.const_val(), p, r.part) {
                return Ok(r.add(a));
            }
            if let (Some(a), Some(s), Some(rs)) = (Some(()), l.sym.as_ref(), r.sym.as_ref()) {
                let _ = a;
                if p == "-" && s == rs && l.part == Part::Val && r.part == Part::Val {
                    return Ok(Expr::num(l.off - r.off));
                }
            }
            Err(format!("unsupported relocatable expression ({} {} {})", l, p, r))
        }
    }
}
