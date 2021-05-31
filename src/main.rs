use chan;
use std::{env,thread};

fn main() {
    let args: Vec<String> = env::args().collect();

    let rx = {
        let (tx, rx) = chan::sync(0);
        for a in args.iter().skip(1) {
            let tx = tx.clone();
            let arg = a.clone();
            thread::spawn(move || {
                tx.send(arg);
            });
        }
        rx
    };
    let wg = chan::WaitGroup::new();
    for i in 0..args.len() {
        println!("receiver {}", i);
        wg.add(1);
        let wg = wg.clone();
        let rx = rx.clone();
        thread::spawn(move || {
            println!("{}:{:?}", i, rx.recv());
            wg.done();
        });
    }
    wg.wait();
}
