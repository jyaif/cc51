//! Source locations and diagnostics.

use std::cell::RefCell;
use std::fmt;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Loc {
    pub file: u32,
    pub line: u32,
    pub col: u32,
}

pub struct SourceFile {
    pub name: String,
}

thread_local! {
    static FILES: RefCell<Vec<SourceFile>> = RefCell::new(Vec::new());
    static WARNINGS: RefCell<bool> = RefCell::new(true);
}

pub fn add_file(name: &str) -> u32 {
    FILES.with(|f| {
        let mut f = f.borrow_mut();
        f.push(SourceFile { name: name.to_string() });
        (f.len() - 1) as u32
    })
}

pub fn file_name(id: u32) -> String {
    FILES.with(|f| f.borrow().get(id as usize).map(|s| s.name.clone()).unwrap_or_else(|| "<unknown>".into()))
}

pub fn set_warnings(on: bool) {
    WARNINGS.with(|w| *w.borrow_mut() = on);
}

impl fmt::Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}:{}:{}", file_name(self.file), self.line, self.col)
    }
}

#[derive(Debug, Clone)]
pub struct Error {
    pub loc: Option<Loc>,
    pub msg: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.loc {
            Some(l) => write!(f, "{}: error: {}", l, self.msg),
            None => write!(f, "error: {}", self.msg),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn err<T>(loc: Loc, msg: impl Into<String>) -> Result<T> {
    Err(Error { loc: Some(loc), msg: msg.into() })
}

pub fn error(loc: Loc, msg: impl Into<String>) -> Error {
    Error { loc: Some(loc), msg: msg.into() }
}

pub fn error_noloc(msg: impl Into<String>) -> Error {
    Error { loc: None, msg: msg.into() }
}

pub fn warn(loc: Loc, msg: impl AsRef<str>) {
    if WARNINGS.with(|w| *w.borrow()) {
        eprintln!("{}: warning: {}", loc, msg.as_ref());
    }
}
