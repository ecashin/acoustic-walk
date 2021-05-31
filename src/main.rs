use chan;
use std::{env,thread};
use walkdir::WalkDir;

const N_WORKERS: u32 = 10;

fn do_work(worker_id: u32, rdirs: chan::Receiver<String>, done: chan::Sender<()>) {
    for dir in rdirs {
        println!("worker:{} dir:{}", worker_id, dir);
    }
    done.send(());
}

fn main() {
    let dirs: Vec<String> = env::args().skip(1).collect();

    let (sdone, rdone) = chan::sync(0);
    {
        let (sdirs, rdirs) = chan::sync(0);
        for w in 0..N_WORKERS {
            let sdone = sdone.clone();
            let rdirs = rdirs.clone();
            thread::spawn(move || {
                do_work(w, rdirs, sdone);
            });
        }
        for d in dirs {
            sdirs.send(d);
        }
    }
    for _ in 0..N_WORKERS {
        rdone.recv();
    }
}
