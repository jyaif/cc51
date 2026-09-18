//! C preprocessor.

pub mod lexer;

use crate::diag::{self, Error, Loc, Result, err, error};
pub use lexer::{PKind, PTok};

/// Source text as characters. Bytes that are not valid UTF-8 (a latin-1 source, say) are mapped
/// into the private use area so that string literals can recover the original byte.
pub fn decode_source(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let mut out = String::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                let rest = &bytes[i..];
                match std::str::from_utf8(&rest[..rest.len().min(4)]) {
                    Ok(s) => {
                        let c = s.chars().next().unwrap();
                        out.push(c);
                        i += c.len_utf8();
                    }
                    Err(e) if e.valid_up_to() > 0 => {
                        let s = std::str::from_utf8(&rest[..e.valid_up_to()]).unwrap();
                        out.push_str(s);
                        i += e.valid_up_to();
                    }
                    Err(_) => {
                        out.push(char::from_u32(0xE000 + bytes[i] as u32).unwrap());
                        i += 1;
                    }
                }
            }
            out
        }
    }
}
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug)]
struct Macro {
    params: Option<Vec<Rc<str>>>,
    variadic: bool,
    body: Vec<PTok>,
}

#[derive(Clone, Copy, PartialEq)]
enum CondCtx {
    Then,
    Elif,
    Else,
}

struct Cond {
    ctx: CondCtx,
    included: bool,
    /// Depth of the include stack when the #if was opened.
    file_depth: usize,
}

pub struct Preprocessor {
    macros: HashMap<Rc<str>, Rc<Macro>>,
    pub include_paths: Vec<PathBuf>,
    pub builtin_headers: fn(&str) -> Option<&'static str>,
    conds: Vec<Cond>,
    once: HashSet<PathBuf>,
    /// Include guard detection: file path -> guard macro.
    guards: HashMap<PathBuf, Rc<str>>,
    /// Reversed input token stack.
    input: Vec<PTok>,
    /// Stack of files being processed (path, dir-index used to find it).
    files: Vec<(PathBuf, Option<usize>)>,
    counter: u64,
    pub deps: Vec<PathBuf>,
    /// Honour `# N "file"` line markers (for preprocessed input).
    pub line_markers: bool,
}

fn mk(kind: PKind, text: &str, loc: Loc) -> PTok {
    PTok { kind, text: text.into(), loc, space: false, bol: false, hideset: None, noexpand: false }
}

fn eof_tok(loc: Loc) -> PTok {
    mk(PKind::Eof, "", loc)
}

impl Preprocessor {
    pub fn new(include_paths: Vec<PathBuf>, builtin_headers: fn(&str) -> Option<&'static str>) -> Self {
        let mut pp = Preprocessor {
            macros: HashMap::new(),
            include_paths,
            builtin_headers,
            conds: Vec::new(),
            once: HashSet::new(),
            guards: HashMap::new(),
            input: Vec::new(),
            files: Vec::new(),
            counter: 0,
            deps: Vec::new(),
            line_markers: false,
        };
        for (k, v) in [
            ("__STDC__", "1"),
            ("__STDC_VERSION__", "201112L"),
            ("__STDC_HOSTED__", "0"),
            ("__STDC_ISO_10646__", "201706L"),
            ("__STDC_UTF_16__", "1"),
            ("__STDC_UTF_32__", "1"),
            ("__STDC_NO_ATOMICS__", "1"),
            ("__STDC_NO_COMPLEX__", "1"),
            ("__STDC_NO_THREADS__", "1"),
            ("__STDC_NO_VLA__", "1"),
            ("__CC51__", "1"),
            ("__SDCC", "4_6_0"),
            ("SDCC", "460"),
            ("__SDCC_VERSION_MAJOR", "4"),
            ("__SDCC_VERSION_MINOR", "6"),
            ("__SDCC_VERSION_PATCH", "0"),
            ("__SDCC_mcs51", "1"),
            ("SDCC_mcs51", "1"),
            ("__mcs51", "1"),
            ("__SDCC_MODEL_SMALL", "1"),
            ("SDCC_MODEL_SMALL", "1"),
            ("__SDCC_CHAR_UNSIGNED", "1"),
            ("__SDCC_INT_LONG_REENT", "1"),
            ("__SDCC_FLOAT_REENT", "1"),
            ("__CHAR_BIT__", "8"),
            ("__SIZEOF_INT__", "2"),
            ("__SIZEOF_LONG__", "4"),
            ("__SIZEOF_LONG_LONG__", "8"),
            ("__SIZEOF_SHORT__", "2"),
            ("__SIZEOF_POINTER__", "2"),
        ] {
            pp.define(k, v);
        }
        pp
    }

    /// Define an object-like macro from a string (as with -D).
    pub fn define(&mut self, name: &str, value: &str) {
        let src = format!("#define {} {}\n", name, value);
        self.define_from_source(&src);
    }

    pub fn undef(&mut self, name: &str) {
        self.macros.remove(name);
    }

    /// Process a `#define` line given as source text (used for -D name(args)=...).
    pub fn define_from_source(&mut self, src: &str) {
        let fid = diag::add_file("<command-line>");
        let toks = lexer::tokenize(src, fid).expect("bad command-line define");
        let saved = std::mem::take(&mut self.input);
        self.input = toks.into_iter().rev().collect();
        let hash = self.input.pop().unwrap();
        let _ = self.directive(hash);
        self.input = saved;
    }

    pub fn preprocess_file(&mut self, path: &Path) -> Result<Vec<PTok>> {
        let text = std::fs::read(path).map_err(|e| diag::error_noloc(format!("cannot read {}: {}", path.display(), e)))?;
        let text = decode_source(&text);
        self.preprocess_source(&text, path)
    }

    pub fn preprocess_source(&mut self, text: &str, path: &Path) -> Result<Vec<PTok>> {
        let fid = diag::add_file(&path.display().to_string());
        let mut toks = lexer::tokenize(text, fid)?;
        if self.line_markers {
            toks = apply_line_markers(toks);
        }
        self.push_file(toks, path.to_path_buf(), None, Loc { file: fid, line: 1, col: 1 });
        let mut out = Vec::new();
        self.run(&mut out, false)?;
        Ok(out)
    }

    fn push_file(&mut self, toks: Vec<PTok>, path: PathBuf, dir_idx: Option<usize>, loc: Loc) {
        let end_loc = toks.last().map(|t| t.loc).unwrap_or(loc);
        let mut e = eof_tok(end_loc);
        e.text = "file".into();
        self.input.push(e);
        for t in toks.into_iter().rev() {
            self.input.push(t);
        }
        self.files.push((path, dir_idx));
    }

    fn peek(&self) -> Option<&PTok> {
        self.input.last()
    }

    /// Main loop. If `stop_at_sentinel`, stop when an Eof token with text "sentinel" is reached.
    fn run(&mut self, out: &mut Vec<PTok>, stop_at_sentinel: bool) -> Result<()> {
        while let Some(tok) = self.input.pop() {
            if tok.kind == PKind::Eof {
                if &*tok.text == "sentinel" {
                    if stop_at_sentinel {
                        return Ok(());
                    }
                    continue;
                }
                // End of a file.
                if let Some(c) = self.conds.last() {
                    if c.file_depth == self.files.len() {
                        return err(tok.loc, "unterminated conditional directive");
                    }
                }
                self.files.pop();
                continue;
            }
            if !stop_at_sentinel && tok.bol && tok.is("#") {
                self.directive(tok)?;
                continue;
            }
            if tok.kind == PKind::Ident && self.expand_macro(&tok)? {
                continue;
            }
            out.push(tok);
        }
        Ok(())
    }

    /// Pop the remaining tokens of the current directive line.
    fn read_line(&mut self) -> Vec<PTok> {
        let mut v = Vec::new();
        while let Some(t) = self.peek() {
            if t.bol || t.kind == PKind::Eof {
                break;
            }
            v.push(self.input.pop().unwrap());
        }
        v
    }

    fn directive(&mut self, hash: PTok) -> Result<()> {
        let Some(name_tok) = self.peek().cloned() else { return Ok(()) };
        if name_tok.bol || name_tok.kind == PKind::Eof {
            return Ok(()); // null directive
        }
        self.input.pop();
        let name = name_tok.text.clone();
        match &*name {
            "include" | "include_next" => {
                let line = self.read_line();
                self.do_include(&name_tok, line, &*name == "include_next")
            }
            "define" => self.do_define(&name_tok),
            "undef" => {
                let line = self.read_line();
                if let Some(t) = line.first() {
                    self.macros.remove(&*t.text);
                }
                Ok(())
            }
            "if" => {
                let line = self.read_line();
                let v = self.eval_cond(&name_tok, line)?;
                self.push_cond(v);
                if !v {
                    self.skip()?;
                }
                Ok(())
            }
            "ifdef" | "ifndef" => {
                let line = self.read_line();
                let Some(t) = line.first() else { return err(name_tok.loc, "macro name missing") };
                let mut v = self.macros.contains_key(&*t.text);
                if &*name == "ifndef" {
                    v = !v;
                }
                self.push_cond(v);
                if !v {
                    self.skip()?;
                }
                Ok(())
            }
            "elif" | "elifdef" | "elifndef" | "else" => {
                let line = self.read_line();
                let Some(c) = self.conds.last_mut() else { return err(name_tok.loc, format!("stray #{}", name)) };
                if c.ctx == CondCtx::Else {
                    return err(name_tok.loc, format!("#{} after #else", name));
                }
                if &*name == "else" {
                    c.ctx = CondCtx::Else;
                } else {
                    c.ctx = CondCtx::Elif;
                }
                // We were in an included section; skip everything until #endif.
                if c.included {
                    self.skip()?;
                    return Ok(());
                }
                // We get here only when called from skip() found this directive; see skip().
                let _ = line;
                Ok(())
            }
            "endif" => {
                self.read_line();
                if self.conds.pop().is_none() {
                    return err(name_tok.loc, "stray #endif");
                }
                Ok(())
            }
            "line" => {
                self.read_line();
                Ok(())
            }
            "error" => {
                let line = self.read_line();
                err(name_tok.loc, format!("#error {}", join_tokens(&line)))
            }
            "warning" => {
                let line = self.read_line();
                diag::warn(name_tok.loc, format!("#warning {}", join_tokens(&line)));
                Ok(())
            }
            "pragma" => {
                let line = self.read_line();
                if line.first().map_or(false, |t| &*t.text == "once") {
                    if let Some((p, _)) = self.files.last() {
                        let p = p.clone();
                        self.once.insert(p);
                    }
                    return Ok(());
                }
                let text = join_tokens(&line);
                let mut t = mk(PKind::Pragma, &text, hash.loc);
                t.bol = true;
                self.input.push(t);
                // Emit directly: push to output via marker. We push it back and let run() output it,
                // but run() would treat it as a normal token (not '#'), which is what we want.
                Ok(())
            }
            "ident" | "sccs" | "assert" | "unassert" => {
                self.read_line();
                Ok(())
            }
            _ => {
                if name_tok.kind == PKind::Number {
                    // GNU line marker `# 12 "file"`
                    self.read_line();
                    return Ok(());
                }
                err(name_tok.loc, format!("invalid preprocessing directive #{}", name))
            }
        }
    }

    fn push_cond(&mut self, included: bool) {
        self.conds.push(Cond { ctx: CondCtx::Then, included, file_depth: self.files.len() });
    }

    /// Skip tokens of a false conditional group. Stops after consuming a directive that ends the group
    /// (#endif) or that starts an included group (#elif true / #else).
    fn skip(&mut self) -> Result<()> {
        let mut depth = 0;
        loop {
            let Some(tok) = self.input.pop() else { return Ok(()) };
            if tok.kind == PKind::Eof {
                self.input.push(tok);
                return Ok(());
            }
            if !(tok.bol && tok.is("#")) {
                continue;
            }
            let Some(d) = self.peek().cloned() else { continue };
            if d.bol {
                continue;
            }
            let dn = &*d.text;
            match dn {
                "if" | "ifdef" | "ifndef" => {
                    depth += 1;
                }
                "endif" => {
                    if depth == 0 {
                        self.input.pop();
                        self.read_line();
                        self.conds.pop();
                        return Ok(());
                    }
                    depth -= 1;
                }
                "elif" | "elifdef" | "elifndef" if depth == 0 => {
                    self.input.pop();
                    let line = self.read_line();
                    let c = self.conds.last_mut().unwrap();
                    if c.ctx == CondCtx::Else {
                        return err(d.loc, "#elif after #else");
                    }
                    c.ctx = CondCtx::Elif;
                    if c.included {
                        continue;
                    }
                    let v = match dn {
                        "elif" => self.eval_cond(&d, line)?,
                        "elifdef" => line.first().map_or(false, |t| self.macros.contains_key(&*t.text)),
                        _ => !line.first().map_or(false, |t| self.macros.contains_key(&*t.text)),
                    };
                    if v {
                        self.conds.last_mut().unwrap().included = true;
                        return Ok(());
                    }
                }
                "else" if depth == 0 => {
                    self.input.pop();
                    self.read_line();
                    let c = self.conds.last_mut().unwrap();
                    if c.ctx == CondCtx::Else {
                        return err(d.loc, "#else after #else");
                    }
                    c.ctx = CondCtx::Else;
                    if !c.included {
                        c.included = true;
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
    }

    fn do_define(&mut self, dtok: &PTok) -> Result<()> {
        let Some(name) = self.input.pop() else { return err(dtok.loc, "macro name missing") };
        if name.kind != PKind::Ident || name.bol {
            return err(name.loc, "macro name must be an identifier");
        }
        let mut params = None;
        let mut variadic = false;
        if let Some(t) = self.peek() {
            if t.is("(") && !t.space && !t.bol {
                self.input.pop();
                let mut ps: Vec<Rc<str>> = Vec::new();
                loop {
                    let Some(t) = self.input.pop() else { return err(name.loc, "bad macro parameter list") };
                    if t.bol {
                        return err(t.loc, "unterminated macro parameter list");
                    }
                    if t.is(")") && ps.is_empty() {
                        break;
                    }
                    if t.is("...") {
                        variadic = true;
                        ps.push("__VA_ARGS__".into());
                        let c = self.input.pop();
                        if !c.map_or(false, |c| c.is(")")) {
                            return err(t.loc, "expected ')' after '...'");
                        }
                        break;
                    }
                    if t.kind != PKind::Ident {
                        return err(t.loc, "expected parameter name");
                    }
                    let pname = t.text.clone();
                    let Some(n) = self.input.pop() else { return err(t.loc, "bad macro parameter list") };
                    if n.is("...") {
                        // GNU named variadic: `args...`
                        variadic = true;
                        ps.push(pname);
                        let c = self.input.pop();
                        if !c.map_or(false, |c| c.is(")")) {
                            return err(t.loc, "expected ')' after '...'");
                        }
                        break;
                    }
                    ps.push(pname);
                    if n.is(")") {
                        break;
                    }
                    if !n.is(",") {
                        return err(n.loc, "expected ',' in macro parameter list");
                    }
                }
                params = Some(ps);
            }
        }
        let mut body = self.read_line();
        if let Some(f) = body.first_mut() {
            f.space = false;
        }
        self.macros.insert(name.text.clone(), Rc::new(Macro { params, variadic, body }));
        Ok(())
    }

    fn find_include(&self, fname: &str, quoted: bool, next: bool) -> Option<(PathBuf, Option<usize>)> {
        let p = Path::new(fname);
        if p.is_absolute() {
            return if p.exists() { Some((p.to_path_buf(), None)) } else { None };
        }
        let mut start = 0;
        if next {
            if let Some((_, Some(idx))) = self.files.last() {
                start = idx + 1;
            }
        } else if quoted {
            if let Some((cur, _)) = self.files.last() {
                if let Some(dir) = cur.parent() {
                    let cand = dir.join(fname);
                    if cand.is_file() {
                        return Some((cand, None));
                    }
                }
            }
        }
        for (i, d) in self.include_paths.iter().enumerate().skip(start) {
            let cand = d.join(fname);
            if cand.is_file() {
                return Some((cand, Some(i)));
            }
        }
        if (self.builtin_headers)(fname).is_some() {
            return Some((PathBuf::from(format!("<builtin>/{}", fname)), Some(usize::MAX - 1)));
        }
        None
    }

    fn do_include(&mut self, dtok: &PTok, mut line: Vec<PTok>, next: bool) -> Result<()> {
        if line.is_empty() {
            return err(dtok.loc, "expected filename after #include");
        }
        if !(line[0].kind == PKind::Str) {
            // Macro-expanded form.
            line = self.expand_tokens(line)?;
            if line.is_empty() {
                return err(dtok.loc, "expected filename after #include");
            }
            if line[0].is("<") {
                let mut s = String::from("<");
                for t in &line[1..] {
                    if t.is(">") {
                        break;
                    }
                    if t.space && s.len() > 1 {
                        s.push(' ');
                    }
                    s.push_str(&t.text);
                }
                s.push('>');
                line = vec![mk(PKind::Str, &s, line[0].loc)];
            }
        }
        let text = line[0].text.to_string();
        let (fname, quoted) = if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
            (text[1..text.len() - 1].to_string(), true)
        } else if text.starts_with('<') && text.ends_with('>') {
            (text[1..text.len() - 1].to_string(), false)
        } else {
            return err(line[0].loc, "bad #include filename");
        };
        let Some((path, idx)) = self.find_include(&fname, quoted, next) else {
            return err(dtok.loc, format!("'{}' file not found", fname));
        };
        if self.once.contains(&path) {
            return Ok(());
        }
        if let Some(g) = self.guards.get(&path) {
            if self.macros.contains_key(g) {
                return Ok(());
            }
        }
        if self.files.len() > 200 {
            return err(dtok.loc, "#include nested too deeply");
        }
        let src = if let Some(b) = path.to_str().and_then(|s| s.strip_prefix("<builtin>/")) {
            (self.builtin_headers)(b).unwrap().to_string()
        } else {
            let bytes = std::fs::read(&path).map_err(|e| error(dtok.loc, format!("cannot read {}: {}", path.display(), e)))?;
            self.deps.push(path.clone());
            decode_source(&bytes)
        };
        let fid = diag::add_file(&path.display().to_string());
        let toks = lexer::tokenize(&src, fid)?;
        if let Some(g) = detect_guard(&toks) {
            self.guards.insert(path.clone(), g);
        }
        self.push_file(toks, path, idx, dtok.loc);
        Ok(())
    }

    /// Macro-expand a token list in isolation (no directives).
    fn expand_tokens(&mut self, toks: Vec<PTok>) -> Result<Vec<PTok>> {
        let mut s = eof_tok(Loc::default());
        s.text = "sentinel".into();
        self.input.push(s);
        for mut t in toks.into_iter().rev() {
            t.bol = false;
            self.input.push(t);
        }
        let mut out = Vec::new();
        self.run(&mut out, true)?;
        Ok(out)
    }

    fn expand_macro(&mut self, tok: &PTok) -> Result<bool> {
        if tok.in_hideset(&tok.text) || tok.noexpand {
            return Ok(false);
        }
        // Builtin macros.
        match &*tok.text {
            "__FILE__" => {
                let s = format!("\"{}\"", diag::file_name(tok.loc.file).replace('\\', "\\\\").replace('"', "\\\""));
                let mut t = mk(PKind::Str, &s, tok.loc);
                t.space = tok.space;
                self.input.push(t);
                return Ok(true);
            }
            "__LINE__" => {
                let mut t = mk(PKind::Number, &tok.loc.line.to_string(), tok.loc);
                t.space = tok.space;
                self.input.push(t);
                return Ok(true);
            }
            "__COUNTER__" => {
                let mut t = mk(PKind::Number, &self.counter.to_string(), tok.loc);
                self.counter += 1;
                t.space = tok.space;
                self.input.push(t);
                return Ok(true);
            }
            "__DATE__" | "__TIME__" => {
                let s = if &*tok.text == "__DATE__" { "\"Jan  1 2026\"" } else { "\"00:00:00\"" };
                let mut t = mk(PKind::Str, s, tok.loc);
                t.space = tok.space;
                self.input.push(t);
                return Ok(true);
            }
            _ => {}
        }
        let Some(m) = self.macros.get(&tok.text).cloned() else { return Ok(false) };
        match &m.params {
            None => {
                let mut hs: HashSet<Rc<str>> = tok.hideset.as_deref().cloned().unwrap_or_default();
                hs.insert(tok.text.clone());
                let hs = Rc::new(hs);
                let body = m.body.clone();
                let body = self.paste_only(&body)?;
                self.push_expansion(body, &hs, tok);
                Ok(true)
            }
            Some(params) => {
                // Look for '(' (possibly across newlines).
                let mut skipped = Vec::new();
                loop {
                    match self.peek() {
                        Some(t) if t.kind == PKind::Eof && &*t.text == "file" && false => {
                            skipped.push(self.input.pop().unwrap());
                        }
                        _ => break,
                    }
                }
                let is_paren = self.peek().map_or(false, |t| t.is("("));
                if !is_paren {
                    for t in skipped.into_iter().rev() {
                        self.input.push(t);
                    }
                    return Ok(false);
                }
                self.input.pop();
                let (args, rparen) = self.read_args(tok, params.len(), m.variadic)?;
                let hs1 = tok.hideset.as_deref().cloned().unwrap_or_default();
                let hs2 = rparen.hideset.as_deref().cloned().unwrap_or_default();
                let mut hs: HashSet<Rc<str>> = hs1.intersection(&hs2).cloned().collect();
                hs.insert(tok.text.clone());
                let hs = Rc::new(hs);
                let body = self.subst(&m, params, args)?;
                self.push_expansion(body, &hs, tok);
                Ok(true)
            }
        }
    }

    fn push_expansion(&mut self, body: Vec<PTok>, hs: &Rc<HashSet<Rc<str>>>, orig: &PTok) {
        let n = body.len();
        for (i, mut t) in body.into_iter().enumerate().rev() {
            let merged = match &t.hideset {
                None => hs.clone(),
                Some(h) => {
                    if h.is_subset(hs) {
                        hs.clone()
                    } else {
                        let mut m: HashSet<Rc<str>> = (**h).clone();
                        m.extend(hs.iter().cloned());
                        Rc::new(m)
                    }
                }
            };
            t.hideset = Some(merged);
            t.loc = orig.loc;
            t.bol = false;
            if i == 0 {
                t.space = orig.space;
            }
            let _ = n;
            self.input.push(t);
        }
        if n == 0 {
            // Preserve spacing information for the following token.
            if orig.space {
                if let Some(t) = self.input.last_mut() {
                    if !t.bol {
                        t.space = true;
                    }
                }
            }
        }
    }

    fn read_args(&mut self, mtok: &PTok, nparams: usize, variadic: bool) -> Result<(Vec<Vec<PTok>>, PTok)> {
        let mut args: Vec<Vec<PTok>> = Vec::new();
        let mut cur: Vec<PTok> = Vec::new();
        let mut depth = 0;
        loop {
            let Some(mut t) = self.input.pop() else { return err(mtok.loc, "unterminated macro invocation") };
            if t.kind == PKind::Eof {
                if &*t.text == "file" {
                    // Allow args to span the end of an included file? No: error.
                }
                return err(mtok.loc, format!("unterminated invocation of macro '{}'", mtok.text));
            }
            if t.bol && t.is("#") {
                // Directive inside macro args: process it (GCC behaviour).
                self.directive(t)?;
                continue;
            }
            if depth == 0 && t.is(")") {
                args.push(cur);
                let mut a = args;
                if a.len() == 1 && a[0].is_empty() && nparams == 0 {
                    a.clear();
                }
                if variadic && a.len() + 1 == nparams {
                    a.push(Vec::new());
                }
                if a.len() != nparams {
                    return err(mtok.loc, format!("macro '{}' expects {} arguments, got {}", mtok.text, nparams, a.len()));
                }
                return Ok((a, t));
            }
            if depth == 0 && t.is(",") && !(variadic && args.len() + 1 == nparams) {
                args.push(std::mem::take(&mut cur));
                continue;
            }
            if t.is("(") {
                depth += 1;
            } else if t.is(")") {
                depth -= 1;
            }
            if t.bol {
                t.bol = false;
                t.space = true;
            }
            cur.push(t);
        }
    }

    /// Handle `##` in an object-like macro body.
    fn paste_only(&mut self, body: &[PTok]) -> Result<Vec<PTok>> {
        if !body.iter().any(|t| t.is("##")) {
            return Ok(body.to_vec());
        }
        let mut out: Vec<PTok> = Vec::new();
        let mut i = 0;
        while i < body.len() {
            if body[i].is("##") && !out.is_empty() && i + 1 < body.len() {
                let l = out.pop().unwrap();
                out.push(paste(&l, &body[i + 1])?);
                i += 2;
                continue;
            }
            out.push(body[i].clone());
            i += 1;
        }
        Ok(out)
    }

    fn subst(&mut self, m: &Macro, params: &[Rc<str>], args: Vec<Vec<PTok>>) -> Result<Vec<PTok>> {
        let pidx = |t: &PTok| -> Option<usize> {
            if t.kind != PKind::Ident {
                return None;
            }
            params.iter().position(|p| **p == *t.text)
        };
        let va_idx = if m.variadic { Some(params.len() - 1) } else { None };
        let mut expanded: Vec<Option<Vec<PTok>>> = vec![None; args.len()];
        let body = &m.body;
        let mut out: Vec<PTok> = Vec::new();
        let mut i = 0;
        while i < body.len() {
            let t = &body[i];
            // Stringize
            if t.is("#") && i + 1 < body.len() {
                if let Some(p) = pidx(&body[i + 1]) {
                    let mut s = stringize(&args[p], t.loc);
                    s.space = t.space;
                    out.push(s);
                    i += 2;
                    continue;
                }
            }
            // __VA_OPT__(...)
            if &*t.text == "__VA_OPT__" && t.kind == PKind::Ident && m.variadic && i + 1 < body.len() && body[i + 1].is("(") {
                let mut depth = 0;
                let mut j = i + 1;
                let mut inner = Vec::new();
                loop {
                    if j >= body.len() {
                        return err(t.loc, "unterminated __VA_OPT__");
                    }
                    let b = &body[j];
                    if b.is("(") {
                        depth += 1;
                        if depth == 1 {
                            j += 1;
                            continue;
                        }
                    } else if b.is(")") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    inner.push(b.clone());
                    j += 1;
                }
                let va = va_idx.unwrap();
                if expanded[va].is_none() {
                    expanded[va] = Some(self.expand_tokens(args[va].clone())?);
                }
                if !expanded[va].as_ref().unwrap().is_empty() {
                    let sub = Macro { params: m.params.clone(), variadic: true, body: inner };
                    let r = self.subst(&sub, params, args.clone())?;
                    out.extend(r);
                }
                i = j + 1;
                continue;
            }
            // Token pasting: `## x`
            if t.is("##") {
                if i + 1 >= body.len() {
                    return err(t.loc, "'##' cannot appear at end of macro expansion");
                }
                let rhs = &body[i + 1];
                if let Some(p) = pidx(rhs) {
                    let arg = &args[p];
                    // GNU comma swallowing: `, ## __VA_ARGS__`
                    if Some(p) == va_idx && out.last().map_or(false, |l| l.is(",")) {
                        if arg.is_empty() {
                            out.pop();
                        } else {
                            out.extend(arg.iter().cloned());
                        }
                        i += 2;
                        continue;
                    }
                    if !arg.is_empty() {
                        if let Some(l) = out.pop() {
                            out.push(paste(&l, &arg[0])?);
                        } else {
                            out.push(arg[0].clone());
                        }
                        out.extend(arg[1..].iter().cloned());
                    }
                    i += 2;
                    continue;
                }
                if let Some(l) = out.pop() {
                    out.push(paste(&l, rhs)?);
                } else {
                    out.push(rhs.clone());
                }
                i += 2;
                continue;
            }
            if let Some(p) = pidx(t) {
                // Param followed by ## : use raw arg.
                if i + 1 < body.len() && body[i + 1].is("##") {
                    let arg = &args[p];
                    if arg.is_empty() {
                        // `x ## y` with empty x: result is y (raw if param).
                        if i + 2 < body.len() {
                            if let Some(p2) = pidx(&body[i + 2]) {
                                let a2 = &args[p2];
                                let mut a2c = a2.clone();
                                if let Some(f) = a2c.first_mut() {
                                    f.space = t.space;
                                }
                                out.extend(a2c);
                            } else {
                                let mut c = body[i + 2].clone();
                                c.space = t.space;
                                out.push(c);
                            }
                            i += 3;
                        } else {
                            i += 2;
                        }
                        continue;
                    }
                    let mut a = arg.clone();
                    a[0].space = t.space;
                    out.extend(a);
                    i += 1;
                    continue;
                }
                if expanded[p].is_none() {
                    expanded[p] = Some(self.expand_tokens(args[p].clone())?);
                }
                let mut e = expanded[p].clone().unwrap();
                if let Some(f) = e.first_mut() {
                    f.space = t.space;
                }
                out.extend(e);
                i += 1;
                continue;
            }
            out.push(t.clone());
            i += 1;
        }
        Ok(out)
    }

    fn eval_cond(&mut self, dtok: &PTok, line: Vec<PTok>) -> Result<bool> {
        // Handle `defined` and `__has_include` before expansion.
        let mut v = Vec::new();
        let mut i = 0;
        while i < line.len() {
            let t = &line[i];
            if t.kind == PKind::Ident && (&*t.text == "defined") {
                let (name, adv) = if i + 1 < line.len() && line[i + 1].is("(") {
                    if i + 3 < line.len() + 0 && line.get(i + 3).map_or(false, |x| x.is(")")) {
                        (line[i + 2].text.clone(), 4)
                    } else {
                        return err(t.loc, "bad defined() syntax");
                    }
                } else if i + 1 < line.len() {
                    (line[i + 1].text.clone(), 2)
                } else {
                    return err(t.loc, "macro name missing after 'defined'");
                };
                let d = self.macros.contains_key(&name)
                    || matches!(&*name, "__FILE__" | "__LINE__" | "__COUNTER__" | "__DATE__" | "__TIME__");
                v.push(mk(PKind::Number, if d { "1" } else { "0" }, t.loc));
                i += adv;
                continue;
            }
            if t.kind == PKind::Ident && (&*t.text == "__has_include" || &*t.text == "__has_include_next") {
                // __has_include("x") or __has_include(<x>)
                let mut j = i + 1;
                if j >= line.len() || !line[j].is("(") {
                    return err(t.loc, "expected '(' after __has_include");
                }
                j += 1;
                let (fname, quoted) = if j < line.len() && line[j].kind == PKind::Str && line[j].text.starts_with('"') {
                    let s = line[j].text.to_string();
                    j += 1;
                    (s[1..s.len() - 1].to_string(), true)
                } else {
                    let mut s = String::new();
                    if j < line.len() && line[j].is("<") {
                        j += 1;
                    }
                    while j < line.len() && !line[j].is(">") {
                        s.push_str(&line[j].text);
                        j += 1;
                    }
                    j += 1;
                    (s, false)
                };
                if j >= line.len() || !line[j].is(")") {
                    return err(t.loc, "expected ')' in __has_include");
                }
                let found = self.find_include(&fname, quoted, &*t.text == "__has_include_next").is_some();
                v.push(mk(PKind::Number, if found { "1" } else { "0" }, t.loc));
                i = j + 1;
                continue;
            }
            if t.kind == PKind::Ident && matches!(&*t.text, "__has_attribute" | "__has_builtin" | "__has_feature" | "__has_c_attribute" | "__has_extension") {
                let mut j = i + 1;
                let mut depth = 0;
                while j < line.len() {
                    if line[j].is("(") {
                        depth += 1;
                    } else if line[j].is(")") {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    j += 1;
                }
                v.push(mk(PKind::Number, "0", t.loc));
                i = j + 1;
                continue;
            }
            v.push(t.clone());
            i += 1;
        }
        let expanded = self.expand_tokens(v)?;
        let mut toks = Vec::new();
        for t in expanded {
            if t.kind == PKind::Ident {
                let val = match &*t.text {
                    "true" => "1",
                    _ => "0",
                };
                toks.push(mk(PKind::Number, val, t.loc));
            } else {
                toks.push(t);
            }
        }
        if toks.is_empty() {
            return err(dtok.loc, "#if with no expression");
        }
        let mut ev = CondEval { toks: &toks, pos: 0 };
        let r = ev.expr(dtok.loc)?;
        if ev.pos != toks.len() {
            return err(toks[ev.pos].loc, format!("unexpected token '{}' in preprocessor expression", toks[ev.pos].text));
        }
        Ok(r.v != 0)
    }
}

/// Remap token locations according to `# N "file"` markers and drop the markers.
fn apply_line_markers(toks: Vec<PTok>) -> Vec<PTok> {
    let mut out = Vec::with_capacity(toks.len());
    let mut map: Option<(u32, u32, u32)> = None; // (marker line, file id, line number at next line)
    let mut i = 0;
    let mut files: HashMap<String, u32> = HashMap::new();
    while i < toks.len() {
        let t = &toks[i];
        if t.bol && t.is("#") && i + 2 < toks.len() && toks[i + 1].kind == PKind::Number && !toks[i + 1].bol && toks[i + 2].kind == PKind::Str && !toks[i + 2].bol {
            let line: u32 = toks[i + 1].text.parse().unwrap_or(1);
            let name = toks[i + 2].text.trim_matches('"').to_string();
            let fid = *files.entry(name.clone()).or_insert_with(|| diag::add_file(&name));
            map = Some((t.loc.line, fid, line));
            i += 3;
            // Skip anything else on the marker line.
            while i < toks.len() && !toks[i].bol {
                i += 1;
            }
            continue;
        }
        let mut t = t.clone();
        if let Some((ml, fid, base)) = map {
            if t.loc.line > ml {
                t.loc = Loc { file: fid, line: base + (t.loc.line - ml - 1), col: t.loc.col };
            }
        }
        out.push(t);
        i += 1;
    }
    out
}

fn detect_guard(toks: &[PTok]) -> Option<Rc<str>> {
    // #ifndef X / #define X ... #endif at the very end with nothing after.
    if toks.len() < 6 {
        return None;
    }
    if !(toks[0].is("#") && toks[0].bol && &*toks[1].text == "ifndef" && toks[2].kind == PKind::Ident) {
        return None;
    }
    let g = toks[2].text.clone();
    if !(toks[3].is("#") && toks[3].bol && &*toks[4].text == "define" && toks[5].text == g) {
        return None;
    }
    let n = toks.len();
    if !(toks[n - 2].is("#") && toks[n - 2].bol && &*toks[n - 1].text == "endif") {
        return None;
    }
    // Make sure the final #endif closes the first #ifndef.
    let mut depth = 0;
    let mut i = 0;
    while i < n {
        if toks[i].is("#") && toks[i].bol && i + 1 < n {
            match &*toks[i + 1].text {
                "if" | "ifdef" | "ifndef" => depth += 1,
                "endif" => {
                    depth -= 1;
                    if depth == 0 && i + 2 != n {
                        return None;
                    }
                }
                "else" | "elif" | "elifdef" | "elifndef" => {
                    if depth == 1 {
                        return None;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    Some(g)
}

fn paste(l: &PTok, r: &PTok) -> Result<PTok> {
    let s = format!("{}{}", l.text, r.text);
    let fid = l.loc.file;
    let toks = lexer::tokenize(&s, fid).map_err(|_| error(l.loc, format!("pasting \"{}\" and \"{}\" does not give a valid preprocessing token", l.text, r.text)))?;
    if toks.len() != 1 {
        return err(l.loc, format!("pasting \"{}\" and \"{}\" does not give a valid preprocessing token", l.text, r.text));
    }
    let mut t = toks.into_iter().next().unwrap();
    t.loc = l.loc;
    t.space = l.space;
    t.bol = false;
    t.hideset = l.hideset.clone();
    Ok(t)
}

fn stringize(toks: &[PTok], loc: Loc) -> PTok {
    let mut s = String::from("\"");
    for (i, t) in toks.iter().enumerate() {
        if i > 0 && t.space {
            s.push(' ');
        }
        if t.kind == PKind::Str || t.kind == PKind::Char {
            for c in t.text.chars() {
                if c == '"' || c == '\\' {
                    s.push('\\');
                }
                s.push(c);
            }
        } else {
            s.push_str(&t.text);
        }
    }
    s.push('"');
    mk(PKind::Str, &s, loc)
}

pub fn join_tokens(toks: &[PTok]) -> String {
    let mut s = String::new();
    for (i, t) in toks.iter().enumerate() {
        if i > 0 && t.space {
            s.push(' ');
        }
        s.push_str(&t.text);
    }
    s
}

/// Render tokens back to text (for -E).
pub fn tokens_to_text(toks: &[PTok]) -> String {
    let mut s = String::new();
    let mut last_line = 0;
    let mut last_file = u32::MAX;
    let mut after_pragma = false;
    for t in toks {
        let new_line = t.loc.file != last_file || t.bol || t.kind == PKind::Pragma || after_pragma || t.loc.line != last_line;
        after_pragma = t.kind == PKind::Pragma;
        if new_line {
            if !s.is_empty() {
                s.push('\n');
            }
            if t.loc.file != last_file || t.loc.line != last_line + 1 {
                if t.loc.file != last_file || t.loc.line > last_line + 8 || t.loc.line < last_line {
                    s.push_str(&format!("# {} \"{}\"\n", t.loc.line, diag::file_name(t.loc.file)));
                } else {
                    for _ in last_line + 1..t.loc.line {
                        s.push('\n');
                    }
                }
            }
            last_file = t.loc.file;
            last_line = t.loc.line;
            if t.kind == PKind::Pragma {
                s.push_str("#pragma ");
                s.push_str(&t.text);
                continue;
            }
        } else if t.space {
            s.push(' ');
        }
        if t.kind == PKind::AsmBlock {
            // Keep raw assembly on its own lines.
            s.push_str(&t.text);
            last_line += t.text.matches('\n').count() as u32;
            continue;
        }
        s.push_str(&t.text);
    }
    s.push('\n');
    s
}

// ---------------------------------------------------------------------------
// #if expression evaluator

#[derive(Clone, Copy)]
struct PV {
    v: i128,
    unsigned: bool,
}

struct CondEval<'a> {
    toks: &'a [PTok],
    pos: usize,
}

impl<'a> CondEval<'a> {
    fn peek(&self) -> Option<&'a PTok> {
        self.toks.get(self.pos)
    }
    fn is(&self, s: &str) -> bool {
        self.peek().map_or(false, |t| t.kind == PKind::Punct && &*t.text == s)
    }
    fn norm(v: PV) -> PV {
        if v.unsigned {
            PV { v: (v.v as u64) as i128, unsigned: true }
        } else {
            PV { v: (v.v as i64) as i128, unsigned: false }
        }
    }
    fn expr(&mut self, loc: Loc) -> Result<PV> {
        let mut v = self.cond(loc)?;
        while self.is(",") {
            self.pos += 1;
            v = self.cond(loc)?;
        }
        Ok(v)
    }
    fn cond(&mut self, loc: Loc) -> Result<PV> {
        let c = self.binary(0, loc)?;
        if self.is("?") {
            self.pos += 1;
            let a = self.expr(loc)?;
            if !self.is(":") {
                return err(loc, "expected ':' in preprocessor expression");
            }
            self.pos += 1;
            let b = self.cond(loc)?;
            let u = a.unsigned || b.unsigned;
            let r = if c.v != 0 { a } else { b };
            return Ok(Self::norm(PV { v: r.v, unsigned: u }));
        }
        Ok(c)
    }
    fn prec(op: &str) -> Option<u8> {
        Some(match op {
            "||" => 1,
            "&&" => 2,
            "|" => 3,
            "^" => 4,
            "&" => 5,
            "==" | "!=" => 6,
            "<" | ">" | "<=" | ">=" => 7,
            "<<" | ">>" => 8,
            "+" | "-" => 9,
            "*" | "/" | "%" => 10,
            _ => return None,
        })
    }
    fn binary(&mut self, min: u8, loc: Loc) -> Result<PV> {
        let mut l = self.unary(loc)?;
        loop {
            let Some(t) = self.peek() else { break };
            if t.kind != PKind::Punct {
                break;
            }
            let Some(p) = Self::prec(&t.text) else { break };
            if p <= min {
                break;
            }
            let op = t.text.clone();
            self.pos += 1;
            let r = self.binary(p, loc)?;
            l = Self::apply(&op, l, r, loc)?;
        }
        Ok(l)
    }
    fn apply(op: &str, l: PV, r: PV, loc: Loc) -> Result<PV> {
        let u = l.unsigned || r.unsigned;
        let (a, b) = (Self::norm(PV { v: l.v, unsigned: u }).v, Self::norm(PV { v: r.v, unsigned: u }).v);
        let bool_ = |x: bool| PV { v: x as i128, unsigned: false };
        Ok(match op {
            "||" => bool_(l.v != 0 || r.v != 0),
            "&&" => bool_(l.v != 0 && r.v != 0),
            "|" => Self::norm(PV { v: a | b, unsigned: u }),
            "^" => Self::norm(PV { v: a ^ b, unsigned: u }),
            "&" => Self::norm(PV { v: a & b, unsigned: u }),
            "==" => bool_(a == b),
            "!=" => bool_(a != b),
            "<" => bool_(a < b),
            ">" => bool_(a > b),
            "<=" => bool_(a <= b),
            ">=" => bool_(a >= b),
            "<<" => Self::norm(PV { v: a.wrapping_shl((r.v & 63) as u32), unsigned: l.unsigned }),
            ">>" => Self::norm(PV { v: Self::norm(PV { v: l.v, unsigned: l.unsigned }).v >> ((r.v & 63) as u32), unsigned: l.unsigned }),
            "+" => Self::norm(PV { v: a.wrapping_add(b), unsigned: u }),
            "-" => Self::norm(PV { v: a.wrapping_sub(b), unsigned: u }),
            "*" => Self::norm(PV { v: a.wrapping_mul(b), unsigned: u }),
            "/" | "%" => {
                if b == 0 {
                    return err(loc, "division by zero in preprocessor expression");
                }
                Self::norm(PV { v: if op == "/" { a / b } else { a % b }, unsigned: u })
            }
            _ => unreachable!(),
        })
    }
    fn unary(&mut self, loc: Loc) -> Result<PV> {
        let Some(t) = self.peek() else { return err(loc, "unexpected end of preprocessor expression") };
        if t.kind == PKind::Punct {
            match &*t.text {
                "+" => {
                    self.pos += 1;
                    return self.unary(loc);
                }
                "-" => {
                    self.pos += 1;
                    let v = self.unary(loc)?;
                    return Ok(Self::norm(PV { v: -v.v, unsigned: v.unsigned }));
                }
                "~" => {
                    self.pos += 1;
                    let v = self.unary(loc)?;
                    return Ok(Self::norm(PV { v: !v.v, unsigned: v.unsigned }));
                }
                "!" => {
                    self.pos += 1;
                    let v = self.unary(loc)?;
                    return Ok(PV { v: (v.v == 0) as i128, unsigned: false });
                }
                "(" => {
                    self.pos += 1;
                    let v = self.expr(loc)?;
                    if !self.is(")") {
                        return err(loc, "expected ')' in preprocessor expression");
                    }
                    self.pos += 1;
                    return Ok(v);
                }
                _ => {}
            }
        }
        self.pos += 1;
        match t.kind {
            PKind::Number => parse_pp_number(&t.text).ok_or_else(|| error(t.loc, format!("invalid integer '{}' in preprocessor expression", t.text))),
            PKind::Char => {
                let (v, _) = crate::lex::parse_char_literal(&t.text, t.loc)?;
                Ok(PV { v: v as i128, unsigned: false })
            }
            _ => err(t.loc, format!("unexpected token '{}' in preprocessor expression", t.text)),
        }
    }
}

fn parse_pp_number(s: &str) -> Option<PV> {
    let (v, unsigned) = crate::lex::parse_int_literal(s)?;
    Some(PV { v: v as i128, unsigned: unsigned || v > i64::MAX as u64 })
}

pub fn fmt_err(e: &Error) -> String {
    e.to_string()
}
