use probability::prelude::*;
// use rand::prelude::*;
use rand_distr::Distribution;
use rand_distr::Dirichlet;
use std::{env, path, thread};
use walkdir::WalkDir;

const N_PRODUCERS: u32 = 10;

fn describe_wav(path: path::PathBuf) -> Option<WavDesc> {
    if let Ok(reader) = hound::WavReader::open(&path) {
        let spec = reader.spec();
        let n_samples = reader.len();
        Some(WavDesc { path, spec, n_samples })
    } else {
        None
    }
}

fn do_work(
    worker_id: u32,
    paths_rx: chan::Receiver<path::PathBuf>,
    wdescs_tx: chan::Sender<Option<WavDesc>>,
    done_tx: chan::Sender<u32>,
) {
    for path in paths_rx {
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if let Some(ext) = ext.to_str() {
                    if ext.eq_ignore_ascii_case("wav") {
                        if let Some(wdesc) = describe_wav(path) {
                            println!("worker:{} sending for {:?}", worker_id, &wdesc.path);
                            wdescs_tx.send(Some(wdesc));
                        }
                    }
                }
            }
        }
    }
    wdescs_tx.send(None);
    done_tx.send(worker_id);
}

#[derive(Debug)]
struct WavDesc {
    path: path::PathBuf,
    spec: hound::WavSpec,
    n_samples: u32,
}

fn select_one(wavs: Vec<WavDesc>) {
    if wavs.len() == 0 {
        return
    }
    let lens: Vec<f64> = wavs.iter().map(|e| e.n_samples as f64).collect();
    let dirichlet = Dirichlet::new(&lens).unwrap();
    let mut source = source::default();
    let probs = dirichlet.sample(&mut rand::thread_rng());
    let cat = probability::distribution::Categorical::new(&probs[..]);
    let decider = Independent(&cat, &mut source);
    // let chosen = decider.take(20).collect::<Vec<_>>();
    let chosen = decider.take(1).last().unwrap();
    println!("selected for play: {:?}", chosen);
}

fn consume(n_producers: u32, wdescs_rx: chan::Receiver<Option<WavDesc>>) {
    let mut n = n_producers;
    let mut wavs: Vec<WavDesc> = Vec::new();
    while n > 0 {
        if let Some(wdesc_opt) = wdescs_rx.recv() {
            if let Some(wdesc) = wdesc_opt {
                println!(
                    "consumer received {:?}:{:?}:{:?} with {} producers remaining",
                    wdesc.path, wdesc.spec, wdesc.n_samples, n
                );
                wavs.push(wdesc);
            } else {
                println!("consumer received None");
                n -= 1;
            }
        } else {
            panic!("producers don't close the work channel");
        }
    }
    println!("collected {} wav descriptions", wavs.len());
    select_one(wavs);
}

fn main() {
    let dirs: Vec<String> = env::args().skip(1).collect();

    let (done_tx, done_rx) = chan::sync(0); // worker completion channel
    let (wdescs_tx, wdescs_rx) = chan::sync(0); // wav description channel
    {
        // Work consumer thread takes ownership of wdescs_rx.
        let wdescs_rx = wdescs_rx;
        let done_tx = done_tx.clone();
        thread::spawn(move || {
            consume(N_PRODUCERS, wdescs_rx);
            done_tx.send(N_PRODUCERS); // consumer ID is one greater than max producer ID
        });
    }

    {
        // At the end of this scope, dirs_tx dropped - we're done sending directories.
        let (dirs_tx, dirs_rx) = chan::sync(0);
        for w in 0..N_PRODUCERS {
            let dirs_rx = dirs_rx.clone();
            let done_tx = done_tx.clone();
            let wdescs_tx = wdescs_tx.clone();
            thread::spawn(move || {
                do_work(w, dirs_rx, wdescs_tx, done_tx);
            });
        }
        for d in dirs {
            for entry in WalkDir::new(d).into_iter().filter_map(|e| e.ok()) {
                let p = path::PathBuf::from(entry.path());
                dirs_tx.send(p);
            }
        }
    }
    let mut n_workers = N_PRODUCERS + 1;
    while n_workers > 0 {
        if let Some(worker_id) = done_rx.recv() {
            println!("worker {} finished", worker_id);
            n_workers -= 1;
        }
    }
}