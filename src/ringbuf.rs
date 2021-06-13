use std::io;
use std::path;

pub const DEFAULT_N_ENTRIES: usize = 1024;

fn empty_n(n: usize) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    for _ in 0..n {
        v.push(String::from(""));
    }
    v
}

pub fn start(trigfile: path::PathBuf, n_entries: usize) {
    let stdin = io::stdin();
    let mut ring: Vec<String> = empty_n(n_entries);
    let mut pos = 0;
    let mut quiet = true;
    loop {
        ring[pos].truncate(0);
        stdin.read_line(&mut ring[pos]).ok();
        if quiet && trigfile.exists() {
            quiet = false;
            for i in pos + 1..n_entries + pos {
                print!("1: {}", &ring[i % n_entries]);
            }
        }
        if !quiet {
            print!("2: {}", &ring[pos]);
            if !trigfile.exists() {
                quiet = true;
            }
        }
        pos = (pos + 1) % n_entries;
    }
}
