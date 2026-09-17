use cc51::*;
use std::path::{Path, PathBuf};
use std::process::exit;

const USAGE: &str = "usage: cc51 [options] files...
  -o <file>          output file (.hex/.ihx: Intel HEX, .bin: binary; with -c: object file)
  -c                 compile only (produce object files)
  -E                 preprocess only
  -I<dir>            add include directory
  -D<name>[=<val>]   define macro
  -U<name>           undefine macro
  -O0 / -O / -O2     optimization level (default -O2)
  --code-loc <addr>  code start address (default 0)
  --code-size <n>    code size limit (default 0x10000)
  --iram-size <n>    internal RAM size (default 256)
  --xram-loc <addr>  external RAM start (default 0)
  --xram-size <n>    external RAM size
  --map <file>       write a map file
  --lst <file>       write a listing file
  --dump-ir          print optimized IR and allocation
  -w                 disable warnings
Accepts SDCC-style options (-mmcs51, --std-*, --model-small) for compatibility.";

struct Args {
    incs: Vec<PathBuf>,
    defs: Vec<(String, Option<String>)>,
    files: Vec<String>,
    out: Option<String>,
    compile_only: bool,
    preprocess_only: bool,
    opts: cg::program::Options,
    map: Option<String>,
    lst: Option<String>,
}

fn parse_num(s: &str) -> u32 {
    let s = s.trim();
    let r = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(h, 16)
    } else {
        s.parse::<u32>()
    };
    r.unwrap_or_else(|_| {
        eprintln!("cc51: invalid number '{}'", s);
        exit(1)
    })
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut a = Args {
        incs: vec![],
        defs: vec![],
        files: vec![],
        out: None,
        compile_only: false,
        preprocess_only: false,
        opts: Default::default(),
        map: None,
        lst: None,
    };
    let mut i = 0;
    while i < argv.len() {
        let s = &argv[i];
        let mut next = || {
            i += 1;
            argv.get(i).cloned().unwrap_or_else(|| {
                eprintln!("cc51: missing argument for {}", s);
                exit(1)
            })
        };
        match s.as_str() {
            "-h" | "--help" => {
                println!("{}", USAGE);
                exit(0)
            }
            "-o" => a.out = Some(next()),
            "-c" => a.compile_only = true,
            "-E" => a.preprocess_only = true,
            "-O0" => a.opts.opt = 0,
            "-O" | "-O1" | "-O2" | "-Os" | "-O3" | "--opt-code-size" | "--opt-code-speed" => a.opts.opt = 2,
            "--code-loc" => a.opts.code_start = parse_num(&next()),
            "--code-size" => a.opts.code_size = parse_num(&next()),
            "--iram-size" => a.opts.iram_size = parse_num(&next()),
            "--xram-loc" => a.opts.xram_start = parse_num(&next()),
            "--xram-size" => a.opts.xram_size = parse_num(&next()),
            "--map" => a.map = Some(next()),
            "--lst" => a.lst = Some(next()),
            "--dump-ir" => a.opts.dump_ir = true,
            "--dump-ir-raw" => a.opts.dump_ir_raw = true,
            "-w" => diag::set_warnings(false),
            "-I" => a.incs.push(PathBuf::from(next())),
            "-D" => {
                let d = next();
                a.defs.push(match d.split_once('=') {
                    Some((k, v)) => (k.to_string(), Some(v.to_string())),
                    None => (d, None),
                })
            }
            _ => {
                if let Some(p) = s.strip_prefix("-I") {
                    a.incs.push(PathBuf::from(p));
                } else if let Some(d) = s.strip_prefix("-D") {
                    a.defs.push(match d.split_once('=') {
                        Some((k, v)) => (k.to_string(), Some(v.to_string())),
                        None => (d.to_string(), None),
                    });
                } else if let Some(u) = s.strip_prefix("-U") {
                    a.defs.push((format!("-{}", u), None));
                } else if s.starts_with("-m") || s.starts_with("--std") || s.starts_with("--model") || s == "--no-xinit-opt" || s.starts_with("-W") || s == "--debug" || s == "-g" {
                    // accepted for compatibility
                } else if s.starts_with('-') {
                    eprintln!("cc51: unknown option '{}'", s);
                    exit(1);
                } else {
                    a.files.push(s.clone());
                }
            }
        }
        i += 1;
    }
    a
}

const OBJ_MAGIC: &str = "CC51OBJ 1\n";

fn preprocess(a: &Args, path: &Path) -> Result<Vec<pp::PTok>, diag::Error> {
    let mut pp = pp::Preprocessor::new(a.incs.clone(), headers::get);
    for (k, v) in &a.defs {
        if let Some(u) = k.strip_prefix('-') {
            pp.undef(u);
            continue;
        }
        match v {
            Some(v) => {
                if let Some((name, params)) = k.split_once('(') {
                    pp.define_from_source(&format!("#define {}({} {}\n", name, params, v));
                } else {
                    pp.define(k, v)
                }
            }
            None => pp.define(k, "1"),
        }
    }
    pp.preprocess_file(path)
}

fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("{}", e);
    exit(1)
}

fn main() {
    let a = parse_args();
    if a.files.is_empty() {
        eprintln!("{}", USAGE);
        exit(1);
    }
    if a.preprocess_only {
        for f in &a.files {
            let t = preprocess(&a, Path::new(f)).unwrap_or_else(|e| fail(e));
            print!("{}", pp::tokens_to_text(&t));
        }
        return;
    }
    if a.compile_only {
        // An object file holds the preprocessed source; code generation happens at link time.
        for f in &a.files {
            let t = preprocess(&a, Path::new(f)).unwrap_or_else(|e| fail(e));
            // Check syntax and types now so errors are reported early.
            let toks = lex::convert(t.clone()).unwrap_or_else(|e| fail(e));
            let mut prog = ast::Program::new();
            parse::parse_tu(toks, &mut prog).unwrap_or_else(|e| fail(e));
            let text = format!("{}{}", OBJ_MAGIC, pp::tokens_to_text(&t));
            let out = match (&a.out, a.files.len()) {
                (Some(o), 1) => o.clone(),
                _ => Path::new(f).with_extension("rel").to_string_lossy().into_owned(),
            };
            std::fs::write(&out, text).unwrap_or_else(|e| fail(format!("cannot write {}: {}", out, e)));
        }
        return;
    }
    let mut prog = ast::Program::new();
    let mut tus: Vec<Vec<pp::PTok>> = Vec::new();
    for f in &a.files {
        let path = Path::new(f);
        let data = std::fs::read(path).unwrap_or_else(|e| fail(format!("cannot read {}: {}", f, e)));
        let toks = if data.starts_with(OBJ_MAGIC.as_bytes()) {
            let text = String::from_utf8_lossy(&data[OBJ_MAGIC.len()..]).into_owned();
            let mut pp = pp::Preprocessor::new(vec![], headers::get);
            pp.preprocess_source(&text, path).unwrap_or_else(|e| fail(e))
        } else if f.ends_with(".lib") || f.ends_with(".a") {
            // Library: concatenated objects.
            let text = String::from_utf8_lossy(&data).into_owned();
            for part in text.split(OBJ_MAGIC).filter(|p| !p.trim().is_empty()) {
                let mut pp = pp::Preprocessor::new(vec![], headers::get);
                tus.push(pp.preprocess_source(part, path).unwrap_or_else(|e| fail(e)));
            }
            continue;
        } else {
            preprocess(&a, path).unwrap_or_else(|e| fail(e))
        };
        tus.push(toks);
    }
    // Parse everything. If pointer address-space inference finds generic (3-byte) pointers, parse again
    // with that knowledge so that all sizes and layouts are consistent.
    let mut converted: Vec<Vec<lex::Token>> = Vec::new();
    for t in tus {
        converted.push(lex::convert(t).unwrap_or_else(|e| fail(e)));
    }
    let mut hint = std::collections::HashSet::new();
    for _ in 0..4 {
        prog = ast::Program::new();
        prog.spaces.generic_hint = hint.clone();
        for t in &converted {
            parse::parse_tu(t.clone(), &mut prog).unwrap_or_else(|e| fail(e));
        }
        let n = prog.spaces.parent.len() as u32;
        let new_hint: std::collections::HashSet<u32> = (0..n).filter(|&v| prog.spaces.resolve(v).is_none() || hint.contains(&v)).collect();
        if new_hint == hint {
            break;
        }
        hint = new_hint;
        diag::set_warnings(false);
    }
    let out = cg::program::compile(&prog, &a.opts).unwrap_or_else(|e| fail(format!("error: {}", e)));
    let out_path = a.out.clone().unwrap_or_else(|| {
        Path::new(&a.files[0]).with_extension("ihx").to_string_lossy().into_owned()
    });
    if out_path.ends_with(".bin") {
        std::fs::write(&out_path, &out.image).unwrap_or_else(|e| fail(e));
    } else {
        std::fs::write(&out_path, &out.hex).unwrap_or_else(|e| fail(e));
    }
    if let Some(m) = &a.map {
        std::fs::write(m, &out.map).unwrap_or_else(|e| fail(e));
    }
    if let Some(l) = &a.lst {
        std::fs::write(l, &out.listing).unwrap_or_else(|e| fail(e));
    }
    eprintln!("code: {} bytes, ram: {:#04x}", out.code_size, out.ram_used);
}
