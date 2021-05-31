use chan;
use std::{env,path,thread};
use walkdir::WalkDir;

const N_WORKERS: u32 = 10;

fn do_work(worker_id: u32,
    paths_rx: chan::Receiver<path::PathBuf>,
    wdescs_tx: chan::Sender<Option<WavDesc>>) {
    for path in paths_rx {
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if let Some(ext) = ext.to_str() {
                    if ext.eq_ignore_ascii_case("wav") {
                        println!("worker:{} sending for {:?}", worker_id, path);
                        wdescs_tx.send(Some(WavDesc {
                            path
                        }));
                    }
                }
            }
        }
    }
    wdescs_tx.send(None);
}

struct WavDesc {
    path: path::PathBuf,
    // hdr: wav::Header,
    // data: wav::BitDepth
}

fn main() {
    let dirs: Vec<String> = env::args().skip(1).collect();

    let (wdescs_tx, wdescs_rx) = chan::sync(0);  // wav description channel
    {
        // At the end of this scope, we're done sending directories.
        let (dirs_tx, dirs_rx) = chan::sync(0);
        for w in 0..N_WORKERS {
            let dirs_rx = dirs_rx.clone();
            let wdescs_tx = wdescs_tx.clone();
            thread::spawn(move || {
                do_work(w, dirs_rx, wdescs_tx);
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
    let mut n_producers = N_WORKERS;
    while n_producers > 0 {
        if let Some(wdesc_opt) = wdescs_rx.recv() {
            if let Some(wdesc) = wdesc_opt {
                println!("consumer received {:?} with {} producers remaining", wdesc.path, n_producers);
            } else {
                println!("consumer received None");
                n_producers -= 1;
            }
        } else {
            panic!("producers don't close the work channel");
        }
    }
}
