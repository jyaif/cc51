//! Command-line 8051 simulator.
use cc51::sim::{Cpu, NullIo};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut file = None;
    let mut max_cycles: u64 = 100_000_000;
    let mut trace = false;
    let mut verbose = false;
    let mut watch: Option<usize> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--max-cycles" => {
                i += 1;
                max_cycles = args[i].parse().unwrap();
            }
            "--trace" => trace = true,
            "--watch" => {
                i += 1;
                let a = args[i].trim_start_matches("0x");
                watch = Some(usize::from_str_radix(a, 16).expect("--watch takes a hex internal RAM address"));
            }
            "-v" => verbose = true,
            f => file = Some(f.to_string()),
        }
        i += 1;
    }
    let file = file.expect("usage: sim51 [--max-cycles N] [--trace] [--watch ADDR] [-v] file.ihx|file.bin");
    let data = std::fs::read(&file).expect("cannot read file");
    let rom = if file.ends_with(".bin") { data } else { Cpu::load_hex(&String::from_utf8_lossy(&data)).unwrap() };
    let mut cpu = Cpu::new(rom);
    cpu.trace = trace;
    let mut io = NullIo;
    match watch {
        None => cpu.run(&mut io, max_cycles),
        Some(a) => {
            let mut last = cpu.st.iram[a];
            while !cpu.halted && cpu.st.cycles < max_cycles {
                let pc = cpu.st.pc;
                cpu.step(&mut io);
                if cpu.st.iram[a] != last {
                    last = cpu.st.iram[a];
                    eprintln!("watch {:#04x}: {:02x} written at pc {:04x} (cycle {})", a, last, pc, cpu.st.cycles);
                }
            }
            if !cpu.halted {
                cpu.halt_reason = "cycle limit".into();
            }
        }
    }
    use std::io::Write;
    std::io::stdout().write_all(&cpu.serial).unwrap();
    if verbose {
        eprintln!("halted: {} after {} cycles, max SP {:#04x}", cpu.halt_reason, cpu.st.cycles, cpu.max_sp);
    }
    if !cpu.halted || cpu.halt_reason != "self loop" {
        eprintln!("sim51: {}", cpu.halt_reason);
        std::process::exit(2);
    }
}
