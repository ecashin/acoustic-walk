use crossbeam_channel::{bounded, select, Sender};
use std::collections::VecDeque;
use std::fmt::{self, Display};
use std::time::{Duration, SystemTime};
use std::{io, path, thread};

pub const DEFAULT_N_ENTRIES: usize = 1024;

#[derive(Debug)]
struct Entry {
    buf: String,
    rel_time: Duration,
}

impl Display for Entry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2}s: {}", self.rel_time.as_secs_f32(), self.buf)
    }
}

fn start_trig_watcher(trigfile: path::PathBuf, trig_tx: Sender<bool>) {
    thread::Builder::new()
        .name("trig watcher".to_string())
        .spawn(move || {
            let mut last_state = trigfile.exists();
            trig_tx.send(last_state).unwrap();
            loop {
                thread::sleep(Duration::from_secs(1));
                let state = trigfile.exists();
                if state != last_state {
                    trig_tx.send(state).unwrap();
                    last_state = state;
                }
            }
        })
        .expect("spawning trig watcher");
}

fn start_line_getter(line_tx: Sender<Entry>) {
    let start = SystemTime::now();
    let stdin = io::stdin();
    let mut buf = String::new();
    thread::Builder::new()
        .name("line getter".to_string())
        .spawn(move || loop {
            buf.truncate(0);
            match stdin.read_line(&mut buf).ok() {
                Some(n) => {
                    if n == 0 {
                        eprintln!("line getter got zero read from stdin and exits now");
                        return;
                    }
                }
                None => println!("line getter got None from stdin"),
            }
            let rel_time = SystemTime::now()
                .duration_since(start)
                .expect("getting time in line getter");
            line_tx
                .send(Entry {
                    buf: buf.clone(),
                    rel_time,
                })
                .unwrap();
        })
        .expect("spawning line getter");
}

pub fn start(trigfile: path::PathBuf, n_entries: usize) {
    let mut ring: VecDeque<Entry> = VecDeque::with_capacity(n_entries);
    let mut quiet = true;
    let (trig_tx, trig_rx) = bounded(0);
    start_trig_watcher(trigfile, trig_tx);
    let (line_tx, line_rx) = bounded(0);
    start_line_getter(line_tx);
    loop {
        select! {
            recv(line_rx) -> entry_msg => {
                match entry_msg {
                    Ok(entry) => {
                        if !quiet {
                            print!("{}", entry);
                        }
                        ring.push_back(entry);
                        if ring.len() > n_entries {
                            ring.pop_front();
                        }
                    },
                    Err(e) => {
                        eprintln!("ringbuf received error from line getter: {}", e);
                        break
                    },
                };
            },
            recv(trig_rx) -> trig_msg => {
                match trig_msg {
                    Ok(trig) => {
                        quiet = !trig;
                        if !quiet {
                            for entry in ring.iter() {
                                print!("{}", entry);
                            }
                        }
                    },
                    Err(e) => panic!("ringbuf received error from trig watcher: {}", e),
                }
            },
        }
    }
}
