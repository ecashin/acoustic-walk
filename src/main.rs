use chan;
use std::{env,path,thread};
use walkdir::WalkDir;

const N_WORKERS: u32 = 10;

fn do_work(worker_id: u32, rdirs: chan::Receiver<path::PathBuf>, done: chan::Sender<()>) {
    for dir in rdirs {
        println!("worker:{} dir:{:?}", worker_id, dir);
    }
    done.send(());
}

fn main() {
    let dirs: Vec<String> = env::args().skip(1).collect();

    let (sdone, rdone) = chan::sync(0);
    {
        // At the end of this scope, we're done sending directories.
        let (sdirs, rdirs) = chan::sync(0);
        for w in 0..N_WORKERS {
            let sdone = sdone.clone();
            let rdirs = rdirs.clone();
            thread::spawn(move || {
                do_work(w, rdirs, sdone);
            });
        }
        for d in dirs {
            for entry in WalkDir::new(d).into_iter().filter_map(|e| e.ok()) {
                let p = path::PathBuf::from(entry.path());
                sdirs.send(p);
            }
        }
    }
    for _ in 0..N_WORKERS {
        rdone.recv();
    }
}
