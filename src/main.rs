use chan;
use std::{env,path,thread};
use walkdir::WalkDir;

const N_WORKERS: u32 = 10;

fn do_work(worker_id: u32,
    paths_rx: chan::Receiver<path::PathBuf>,
    wdescs_tx: chan::Sender<Option<WavDesc>>,
    done_tx: chan::Sender<u32>) {
    for path in paths_rx {
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if let Some(ext) = ext.to_str() {
                    if ext.eq_ignore_ascii_case("wav") {
                        wdescs_tx.send(Some(WavDesc {
                            path
                        }));
                    }
                }
            }
        }
    }
    wdescs_tx.send(None);
    done_tx.send(worker_id);
}

struct WavDesc {
    path: path::PathBuf,
    // hdr: wav::Header,
    // data: wav::BitDepth
}

fn main() {
    let dirs: Vec<String> = env::args().skip(1).collect();

    let (done_tx, done_rx) = chan::sync(0);  // termination channel
    let (wdescs_tx, wdescs_rx) = chan::sync(0);  // wav description channel
    {
        // At the end of this scope, we're done sending directories.
        let (dirs_tx, dirs_rx) = chan::sync(0);
        for w in 0..N_WORKERS {
            let done_tx = done_tx.clone();
            let dirs_rx = dirs_rx.clone();
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
    // Work consumer thread takes ownership of wdescs_rx.
    {
        let mut n_producers = N_WORKERS;   
        let done_tx = done_tx.clone();
        thread::spawn(move || {
            while n_producers > 0 {
                if let Some(wdesc_opt) = wdescs_rx.recv() {
                    if let Some(wdesc) = wdesc_opt {
                        println!("received {:?} with {} producers remaining", wdesc.path, n_producers);
                    } else {
                        println!("received None");
                        n_producers -= 1;
                    }
                } else {
                    panic!("producers don't close the work channel");
                }
            }
            done_tx.send(N_WORKERS);  // work consumer's ID is one greater than any producer's
        });
    }
    for _ in 0..(N_WORKERS + 1) {
        let wid = done_rx.recv();
        println!("worker {:?} finished", wid);
    }
}
