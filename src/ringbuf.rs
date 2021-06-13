use std::io;
use std::path;
use std::time::{Duration, SystemTime};

pub const DEFAULT_N_ENTRIES: usize = 1024;

#[derive(Debug)]
struct Entry {
    buf: String,
    rel_time: Duration,
}

fn empty_n(n: usize) -> Vec<Entry> {
    let mut v: Vec<_> = Vec::new();
    for _ in 0..n {
        v.push(Entry {
            buf: String::from(""),
            rel_time: Duration::new(0, 0),
        });
    }
    v
}

pub fn start(trigfile: path::PathBuf, n_entries: usize) {
    let stdin = io::stdin();
    let mut ring: Vec<Entry> = empty_n(n_entries);
    let mut pos = 0;
    let mut quiet = true;
    let start = SystemTime::now();
    loop {
        ring[pos].buf.truncate(0);
        ring[pos].rel_time = SystemTime::now()
            .duration_since(start)
            .expect("calculating duration");
        stdin.read_line(&mut ring[pos].buf).ok();
        if quiet && trigfile.exists() {
            quiet = false;
            for i in pos + 1..n_entries + pos {
                let i = i % n_entries;
                print!("{:?}: {}", &ring[i].rel_time, &ring[i].buf);
            }
        }
        if !quiet {
            print!("{:?}: {}", &ring[pos].rel_time, &ring[pos].buf);
            if !trigfile.exists() {
                quiet = true;
            }
        }
        pos = (pos + 1) % n_entries;
    }
}
