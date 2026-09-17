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
  --size             print code size and RAM usage
  -w                 disable warnings
Accepts SDCC-style options (-mmcs51, --std-*, --model-small) for compatibility.";

struct Args {
    libdirs: Vec<PathBuf>,
    libs: Vec<String>,
    incs: Vec<PathBuf>,
    defs: Vec<(String, Option<String>)>,
    files: Vec<String>,
    out: Option<String>,
    compile_only: bool,
    preprocess_only: bool,
    opts: cg::program::Options,
    map: Option<String>,
    lst: Option<String>,
    print_size: bool,
    /// `--std-cXX` (ISO) or `--std-sdccXX` (SDCC extensions) selected on the command line.
    std_pragma: Option<&'static str>,
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
        libdirs: vec![],
        libs: vec![],
        incs: vec![],
        defs: vec![],
        files: vec![],
        out: None,
        compile_only: false,
        preprocess_only: false,
        opts: Default::default(),
        map: None,
        lst: None,
        print_size: false,
        std_pragma: None,
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
            "--size" => a.print_size = true,
            "--dump-ir-raw" => a.opts.dump_ir_raw = true,
            "-w" => diag::set_warnings(false),
            "-I" => a.incs.push(PathBuf::from(next())),
            "-L" => a.libdirs.push(PathBuf::from(next())),
            "-l" => a.libs.push(next()),
            "-k" => a.libdirs.push(PathBuf::from(next())),
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
                } else if let Some(p) = s.strip_prefix("-L") {
                    a.libdirs.push(PathBuf::from(p));
                } else if let Some(p) = s.strip_prefix("-l") {
                    a.libs.push(p.to_string());
                } else if let Some(d) = s.strip_prefix("-D") {
                    a.defs.push(match d.split_once('=') {
                        Some((k, v)) => (k.to_string(), Some(v.to_string())),
                        None => (d.to_string(), None),
                    });
                } else if let Some(u) = s.strip_prefix("-U") {
                    a.defs.push((format!("-{}", u), None));
                } else if let Some(std) = s.strip_prefix("--std-") {
                    a.std_pragma = Some(if std.starts_with("sdcc") { "std_sdcc11" } else { "std_c11" });
                } else if s.starts_with("-m") || s.starts_with("--model") || s == "--no-xinit-opt" || s.starts_with("-W") || s == "--debug" || s == "-g" {
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
    let mut toks = pp.preprocess_file(path)?;
    if let (Some(std), Some(first)) = (a.std_pragma, toks.first()) {
        let mut t = first.clone();
        t.kind = pp::PKind::Pragma;
        t.text = std.into();
        toks.insert(0, t);
    }
    Ok(toks)
}

fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("{}", e);
    exit(1)
}

/// `sdar`-compatible archiver: `sdar -rc lib.lib objs...`
fn ar_main(args: &[String]) {
    let mut it = args.iter();
    let Some(opts) = it.next() else {
        eprintln!("usage: sdar -rc <library> <objects...>");
        exit(1)
    };
    let opts = opts.trim_start_matches('-');
    if !opts.contains('r') && !opts.contains('q') {
        eprintln!("sdar (cc51): only -r/-q is supported");
        exit(1);
    }
    let Some(lib) = it.next() else {
        eprintln!("sdar: missing library name");
        exit(1)
    };
    let mut out = String::new();
    if !opts.contains('c') || Path::new(lib).exists() {
        if let Ok(old) = std::fs::read_to_string(lib) {
            out.push_str(&old);
        }
    }
    for o in it {
        let data = std::fs::read_to_string(o).unwrap_or_else(|e| fail(format!("sdar: cannot read {}: {}", o, e)));
        if !data.starts_with(OBJ_MAGIC) {
            fail(format!("sdar: {} is not a cc51 object", o));
        }
        out.push_str(&data);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    std::fs::write(lib, out).unwrap_or_else(|e| fail(format!("sdar: cannot write {}: {}", lib, e)));
}

/// `makebin`-compatible Intel HEX to binary converter.
fn makebin_main(args: &[String]) {
    let mut pack = false;
    let mut size: Option<u32> = None;
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => pack = true,
            "-s" => {
                i += 1;
                size = Some(parse_num(&args[i]));
            }
            a => files.push(a.to_string()),
        }
        i += 1;
    }
    let text = match files.first() {
        Some(f) => std::fs::read_to_string(f).unwrap_or_else(|e| fail(format!("makebin: {}: {}", f, e))),
        None => {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).unwrap();
            s
        }
    };
    let mut image: Vec<u8> = vec![0xff; size.unwrap_or(0x8000) as usize];
    let mut last = 0usize;
    for line in text.lines() {
        let Some(l) = line.trim().strip_prefix(':') else { continue };
        let b: Vec<u8> = (0..l.len() / 2).filter_map(|k| u8::from_str_radix(&l[2 * k..2 * k + 2], 16).ok()).collect();
        if b.len() < 5 || b[3] != 0 {
            continue;
        }
        let n = b[0] as usize;
        let addr = ((b[1] as usize) << 8) | b[2] as usize;
        if addr + n > image.len() {
            image.resize(addr + n, 0xff);
        }
        image[addr..addr + n].copy_from_slice(&b[4..4 + n]);
        last = last.max(addr + n);
    }
    if pack {
        image.truncate(last);
    }
    match files.get(1) {
        Some(o) => std::fs::write(o, &image).unwrap_or_else(|e| fail(format!("makebin: {}: {}", o, e))),
        None => std::io::Write::write_all(&mut std::io::stdout(), &image).unwrap(),
    }
}

fn main() {
    let argv0 = std::env::args().next().unwrap_or_default();
    let prog = Path::new(&argv0).file_name().and_then(|s| s.to_str()).unwrap_or("cc51").to_string();
    let rest: Vec<String> = std::env::args().skip(1).collect();
    if prog.starts_with("sdar") || rest.first().map_or(false, |a| a == "--ar") {
        let args = if prog.starts_with("sdar") { rest } else { rest[1..].to_vec() };
        ar_main(&args);
        return;
    }
    if prog.starts_with("makebin") || rest.first().map_or(false, |a| a == "--makebin") {
        let args = if prog.starts_with("makebin") { rest } else { rest[1..].to_vec() };
        makebin_main(&args);
        return;
    }
    if rest.iter().any(|a| a == "--version" || a == "-v") {
        println!("SDCC : mcs51 4.6.0 #0 (cc51 {}) (compatible)", env!("CARGO_PKG_VERSION"));
        println!("published under GNU General Public License (GPL)");
        return;
    }
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
    // Library members are weak (like archive members, only used when not already defined).
    let mut tu_weak: Vec<bool> = Vec::new();
    let mut seen_libs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut inputs = a.files.clone();
    for l in &a.libs {
        let names = [l.clone(), format!("{}.lib", l), format!("lib{}.lib", l)];
        let found = a.libdirs.iter().chain(std::iter::once(&PathBuf::from("."))).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file());
        match found {
            Some(p) => {
                let data = std::fs::read(&p).unwrap_or_default();
                if data.starts_with(OBJ_MAGIC.as_bytes()) {
                    inputs.push(p.to_string_lossy().into_owned());
                }
            }
            None => {
                // SDCC system libraries (mcs51, libsdcc, ...) are provided by the built-in runtime.
            }
        }
    }
    for f in &inputs {
        let path = Path::new(f);
        let data = std::fs::read(path).unwrap_or_else(|e| fail(format!("cannot read {}: {}", f, e)));
        if data.starts_with(OBJ_MAGIC.as_bytes()) {
            // Object file or library (concatenated objects).
            let is_lib = f.ends_with(".lib") || f.ends_with(".a");
            if is_lib && !seen_libs.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())) {
                continue;
            }
            let text = String::from_utf8_lossy(&data).into_owned();
            for part in text.split(OBJ_MAGIC).filter(|p| !p.trim().is_empty()) {
                let mut pp = pp::Preprocessor::new(vec![], headers::get);
                pp.line_markers = true;
                tus.push(pp.preprocess_source(part, path).unwrap_or_else(|e| fail(e)));
                tu_weak.push(is_lib);
            }
            continue;
        }
        let toks = preprocess(&a, path).unwrap_or_else(|e| fail(e));
        tu_weak.push(false);
        tus.push(toks);
    }
    // Parse everything. If pointer address-space inference finds generic (3-byte) pointers, parse again
    // with that knowledge so that all sizes and layouts are consistent.
    let mut converted: Vec<Vec<lex::Token>> = Vec::new();
    for t in tus {
        converted.push(lex::convert(t).unwrap_or_else(|e| fail(e)));
    }
    // The C library is a weak unit parsed last.
    let libc_toks = {
        let mut pp = pp::Preprocessor::new(vec![], headers::get);
        let t = pp.preprocess_source(headers::LIBC, Path::new("<libc>")).unwrap_or_else(|e| fail(e));
        lex::convert(t).unwrap_or_else(|e| fail(e))
    };
    let mut hint = std::collections::HashSet::new();
    for _ in 0..4 {
        prog = ast::Program::new();
        prog.spaces.generic_hint = hint.clone();
        // Strong units first, then library members.
        for (t, _) in converted.iter().zip(&tu_weak).filter(|(_, w)| !**w) {
            parse::parse_tu(t.clone(), &mut prog).unwrap_or_else(|e| fail(e));
        }
        for (t, _) in converted.iter().zip(&tu_weak).filter(|(_, w)| **w) {
            parse::parse_tu_ex(t.clone(), &mut prog, true).unwrap_or_else(|e| fail(e));
        }
        parse::parse_tu_ex(libc_toks.clone(), &mut prog, true).unwrap_or_else(|e| fail(e));
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
    if a.print_size {
        eprintln!("code: {} bytes, ram: {:#04x}", out.code_size, out.ram_used);
    }
}
