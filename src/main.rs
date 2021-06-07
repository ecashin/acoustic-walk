use std::{path, thread};
use walkdir::WalkDir;

use grain::N_GRAINS;
use wav::WavDesc;

mod config;

mod grain;
mod wav;

const N_PRODUCERS: u32 = 10;

fn play(client: jack::Client, done_tx: chan::Sender<()>, samples_rx: chan::Receiver<Vec<f32>>) {
    println!("play starting");
    let mut out_left = client
        .register_port("acouwalk_out_L", jack::AudioOut::default())
        .unwrap();
    let mut out_right = client
        .register_port("acouwalk_out_R", jack::AudioOut::default())
        .unwrap();
    let mut samples: Vec<f32> = Vec::new();
    let (jackdone_tx, jackdone_rx) = chan::sync(0);
    let jack_sr = client.sample_rate();
    let mut consumed = 0;
    let process = jack::ClosureProcessHandler::new(
        move |_: &jack::Client, ps: &jack::ProcessScope| -> jack::Control {
            let outl = out_left.as_mut_slice(ps);
            let outr = out_right.as_mut_slice(ps);
            let n = outl.len();
            if consumed > jack_sr {
                samples = samples.split_off(consumed);
                consumed = 0;
            }
            if samples.len() - consumed < n * 2 {
                // (2 for stereo)
                match samples_rx.recv() {
                    Some(mut new_samples) => {
                        println!("play received {} stereo samples", new_samples.len() / 2);
                        samples.append(&mut new_samples)
                    }
                    None => {
                        println!("play received EOF from samples channel");
                        jackdone_tx.send(());
                        return jack::Control::Quit;
                    }
                }
            }
            let n = std::cmp::min(n, samples.len() / 2);

            for i in 0..n {
                let src_off = consumed + i * 2;
                outl[i] = samples[src_off];
                outr[i] = samples[src_off + 1];
            }
            consumed += n * 2;
            jack::Control::Continue
        },
    );

    let active_client = client.activate_async((), process).unwrap();
    let _ = jackdone_rx.recv();
    println!("play received playdone message from JACK handler");
    active_client.deactivate().unwrap();
    println!("play is done");
    done_tx.send(());
}

fn use_wavs(n_producers: u32, wdescs_rx: chan::Receiver<Option<WavDesc>>) {
    let mut n = n_producers;
    let mut wavs: Vec<WavDesc> = Vec::new();
    while n > 0 {
        let wdesc_opt = wdescs_rx.recv().expect("producers don't close work chan");
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
    }
    println!("collected {} wav descriptions", wavs.len());

    let (client, status) =
        jack::Client::new("acouwalk", jack::ClientOptions::NO_START_SERVER).unwrap();
    println!("new client:{:?} status:{:?}", client, status);

    let (samples_tx, samples_rx) = chan::sync(2);
    let (playdone_tx, playdone_rx) = chan::sync(0);
    generate_samples(samples_tx, client.sample_rate(), wavs);
    play(client, playdone_tx, samples_rx);
    playdone_rx.recv();
    println!("use_wavs received playdone message");
}

fn mix(bufs: Vec<Vec<f32>>) -> Vec<f32> {
    let n = bufs.len();
    assert_ne!(n, 0);
    let len = bufs[0].len();
    let mut mixbuf: Vec<f32> = Vec::new();
    for i in 0..len {
        let s: f32 = bufs.iter().map(|buf: &Vec<f32>| buf[i]).sum();
        mixbuf.push(s / n as f32);
    }
    mixbuf
}

fn generate_samples(samples_tx: chan::Sender<Vec<f32>>, sink_sr: usize, wavs: Vec<WavDesc>) -> u32 {
    let mut grains_rxs: Vec<chan::Receiver<Vec<f32>>> = Vec::new();
    for i in 0..N_GRAINS {
        let (grains_tx, grains_rx) = chan::sync(0);
        grain::make_grains(i, &wavs, grains_tx, sink_sr);
        grains_rxs.push(grains_rx);
    }
    let mut n_grain_makers = N_GRAINS;
    // now each grain maker will send JACK-ready samples in chunks mixed below

    thread::spawn(move || {
        while n_grain_makers > 0 {
            let mut bufs: Vec<Vec<f32>> = Vec::new();
            for i in 0..N_GRAINS {
                match grains_rxs[i as usize].recv() {
                    None => n_grain_makers -= 1,
                    Some(buf) => bufs.push(buf),
                }
            }
            if bufs.len() > 0 {
                let mixed = mix(bufs);
                println!(
                    "generate_samples sending {} mixed stereo samples",
                    mixed.len() / 2
                );
                samples_tx.send(mixed);
            } else {
                println!("generate_samples without anything to send");
            }
        }
    });

    N_GRAINS
}

fn main() {
    let cfg = config::make_config();

    let (done_tx, done_rx) = chan::sync(0); // worker completion channel
    let (wdescs_tx, wdescs_rx) = chan::sync(0); // wav description channel
    {
        // Work consumer thread takes ownership of wdescs_rx.
        let wdescs_rx = wdescs_rx;
        let done_tx = done_tx.clone();
        thread::spawn(move || {
            use_wavs(N_PRODUCERS, wdescs_rx);
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
            let cfg = cfg.clone();
            thread::spawn(move || {
                wav::survey_wavs(w, cfg, dirs_rx, wdescs_tx, done_tx);
            });
        }
        for d in cfg.dirs {
            for entry in WalkDir::new(d).into_iter().filter_map(|e| e.ok()) {
                if !cfg.excluded_wavs.iter().any(|i| i == entry.path()) {
                    let p = path::PathBuf::from(entry.path());
                    dirs_tx.send(p);
                }
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
