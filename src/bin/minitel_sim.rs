//! Minitel (NFZ330-like) board simulation for differential testing of the dino game.
//! Writes a trace of video-chip register writes; keyboard input is scripted by keyboard-scan count
//! so that the trace is independent of code speed.

use cc51::sim::{Cpu, CpuState, Io};
use std::io::Write;

struct Minitel {
    trace: Vec<(u16, u8)>,
    r0_reads: u64,
    latched: u8,
    shift_idx: u8,
    latches: u64,
    last_p1: u8,
    script: Vec<(u64, u64, u8, u8)>, // (from latch, to latch, row, col)
    video_base: u16,
}

impl Minitel {
    fn row_state(&self, row: u8) -> u8 {
        let mut v = 0xffu8;
        for &(from, to, r, c) in &self.script {
            if r == row && self.latches >= from && self.latches < to {
                v &= !(1 << c);
            }
        }
        v
    }
}

impl Io for Minitel {
    fn xread(&mut self, _cpu: &CpuState, addr: u16) -> Option<u8> {
        if addr >= self.video_base && addr < self.video_base + 16 {
            if addr == self.video_base {
                self.r0_reads += 1;
                let vsync = (self.r0_reads / 7) % 2 == 1;
                return Some(if vsync { 0x04 } else { 0x00 });
            }
            return Some(0);
        }
        None
    }
    fn xwrite(&mut self, _cpu: &CpuState, addr: u16, v: u8) -> bool {
        self.trace.push((addr, v));
        true
    }
    fn port_read(&mut self, cpu: &CpuState, port: u8) -> Option<u8> {
        if port == 0x90 {
            let latch = cpu.sfr[0x10];
            let bit = (self.latched >> (7 - self.shift_idx.min(7))) & 1;
            return Some((latch & !0x40) | (bit << 6));
        }
        None
    }
    fn port_write(&mut self, _cpu: &CpuState, port: u8, v: u8) {
        if port == 0x90 {
            let rising = (v & 0x20 != 0) && (self.last_p1 & 0x20 == 0);
            if rising {
                if v & 0x10 != 0 {
                    self.latched = self.row_state(v & 0x0f);
                    self.shift_idx = 0;
                    self.latches += 1;
                } else {
                    self.shift_idx = self.shift_idx.saturating_add(1);
                }
            }
            self.last_p1 = v;
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = &args[0];
    let out = &args[1];
    let max_cycles: u64 = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(20_000_000);
    let data = std::fs::read(file).expect("read");
    let rom = if file.ends_with(".bin") { data } else { Cpu::load_hex(&String::from_utf8_lossy(&data)).unwrap() };
    let mut cpu = Cpu::new(rom);
    cpu.stop_on_self_loop = false;
    // Script: SPACE to start, UP presses to jump, eventually collide.
    let mut script = Vec::new();
    let mut t = 2000;
    while t < 10_000_000 {
        script.push((t, t + 100, 0, 7));
        t += 3001;
    }
    let mut t = 2400;
    while t < 10_000_000 {
        script.push((t, t + 6, 8, 7));
        t += 97 + (t % 13);
    }
    let mut io = Minitel { trace: Vec::new(), r0_reads: 0, latched: 0xff, shift_idx: 0, latches: 0, last_p1: 0xff, script, video_base: 0xdf20 };
    cpu.run(&mut io, max_cycles);
    let mut f = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    for (a, v) in &io.trace {
        writeln!(f, "{:04x} {:02x}", a, v).unwrap();
    }
    eprintln!(
        "{}: {} writes, {} latches, {} r0 reads, {} cycles, max SP {:#04x}, halt: {}",
        file,
        io.trace.len(),
        io.latches,
        io.r0_reads,
        cpu.st.cycles,
        cpu.max_sp,
        cpu.halt_reason
    );
}
