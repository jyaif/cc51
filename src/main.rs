use cc51::*;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut incs = Vec::new();
    let mut files = Vec::new();
    let mut defs = Vec::new();
    for a in &args {
        if let Some(p) = a.strip_prefix("-I") {
            incs.push(PathBuf::from(p));
        } else if let Some(d) = a.strip_prefix("-D") {
            defs.push(d.to_string());
        } else if a.starts_with('-') {
        } else {
            files.push(a.clone());
        }
    }
    let mut prog = ast::Program::new();
    for f in &files {
        let mut pp = pp::Preprocessor::new(incs.clone(), headers::get);
        for d in &defs {
            match d.split_once('=') {
                Some((k, v)) => pp.define(k, v),
                None => pp.define(d, "1"),
            }
        }
        if args.iter().any(|a| a == "-E") {
            let t = pp.preprocess_file(std::path::Path::new(f)).unwrap_or_else(|e| { eprintln!("{}", e); std::process::exit(1) });
            print!("{}", pp::tokens_to_text(&t));
            continue;
        }
        let r = pp.preprocess_file(std::path::Path::new(f)).and_then(|t| lex::convert(t)).and_then(|t| parse::parse_tu(t, &mut prog));
        if let Err(e) = r {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
    println!("{} globals, {} funcs", prog.globals.len(), prog.funcs.len());
}
