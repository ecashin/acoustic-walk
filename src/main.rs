use crossbeam_channel::{bounded, Receiver, RecvError, Sender};
use std::{path, thread};
use walkdir::WalkDir;

use config::Config;
use grain::N_GRAINS;
use wav::WavDesc;

mod config;
mod cpalplay;
mod grain;
mod ringbuf;
mod wav;

const N_PRODUCERS: u32 = 10;

fn play_to_jack(client: jack::Client, done_tx: Sender<()>, samples_rx: Receiver<Vec<f32>>) {
    println!("play starting");
    let mut out_left = client
        .register_port("acouwalk_out_L", jack::AudioOut::default())
        .unwrap();
    let mut out_right = client
        .register_port("acouwalk_out_R", jack::AudioOut::default())
        .unwrap();
    let mut samples: Vec<f32> = Vec::new();
    let (jackdone_tx, jackdone_rx) = bounded(0);
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
                    Ok(mut new_samples) => {
                        println!("play received {} stereo samples", new_samples.len() / 2);
                        samples.append(&mut new_samples)
                    }
                    Err(RecvError) => {
                        println!("play received EOF from samples channel");
                        jackdone_tx.send(()).unwrap();
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
    done_tx.send(()).unwrap();
}

fn use_wavs(use_jack: bool, n_producers: u32, wdescs_rx: Receiver<Option<WavDesc>>) {
    let wavpick_rx = wav::start_wav_picker(n_producers, wdescs_rx);

    let (samples_tx, samples_rx) = bounded(2);
    let (playdone_tx, playdone_rx) = bounded(0);
    if use_jack {
        let (client, status) =
            jack::Client::new("acouwalk", jack::ClientOptions::NO_START_SERVER).unwrap();
        println!("new client:{:?} status:{:?}", client, status);
        generate_samples(samples_tx, client.sample_rate(), wavpick_rx);
        play_to_jack(client, playdone_tx, samples_rx);
    } else {
        generate_samples(samples_tx, cpalplay::SAMPLE_RATE, wavpick_rx);
        cpalplay::play_to_cpal(playdone_tx, samples_rx);
    }
    playdone_rx.recv().unwrap();
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

fn generate_samples(
    samples_tx: Sender<Vec<f32>>,
    sink_sr: usize,
    wavpick_rx: Receiver<WavDesc>,
) -> u32 {
    let mut grains_rxs: Vec<Receiver<Vec<f32>>> = Vec::new();
    for i in 0..N_GRAINS {
        let (grains_tx, grains_rx) = bounded(0);
        let wavpick_rx = wavpick_rx.clone();
        grain::make_grains(i, wavpick_rx, grains_tx, sink_sr);
        grains_rxs.push(grains_rx);
    }
    let mut n_grain_makers = N_GRAINS;
    // now each grain maker will send JACK-ready samples in chunks mixed below

    thread::Builder::new()
        .name("mix sender".to_string())
        .spawn(move || {
            while n_grain_makers > 0 {
                let mut bufs: Vec<Vec<f32>> = Vec::new();
                for i in 0..N_GRAINS {
                    match grains_rxs[i as usize].recv() {
                        Ok(buf) => bufs.push(buf),
                        Err(RecvError) => {
                            n_grain_makers -= 1;
                            println!(
                                "Grain buffer receiver got RecvError -> {} grain makers remaining",
                                n_grain_makers
                            );
                        }
                    }
                }
                if !bufs.is_empty() {
                    let mixed = mix(bufs);
                    println!(
                        "generate_samples sending {} mixed stereo samples",
                        mixed.len() / 2
                    );
                    samples_tx.send(mixed).unwrap();
                } else {
                    println!("generate_samples without anything to send");
                }
            }
        })
        .expect("spawning mix sender");

    N_GRAINS
}

fn main() {
    let cfg = config::make_config();

    match cfg {
        Config::Buf(cfg) => {
            ringbuf::start(cfg.trigfile, cfg.n_entries);
        }
        Config::Cpal => cpalplay::cpal_demo(),
        Config::Play(cfg) => {
            acoustic_walk(cfg);
        }
    }
}

fn acoustic_walk(cfg: config::PlayConfig) {
    let (done_tx, done_rx) = bounded(0); // worker completion channel
    let (wdescs_tx, wdescs_rx) = bounded(0); // wav description channel
    {
        // Work consumer thread takes ownership of wdescs_rx.
        let wdescs_rx = wdescs_rx;
        let done_tx = done_tx.clone();
        let use_jack = cfg.use_jack;
        thread::Builder::new()
            .name("wav user".to_string())
            .spawn(move || {
                use_wavs(use_jack, N_PRODUCERS, wdescs_rx);
                done_tx.send(N_PRODUCERS).unwrap(); // consumer ID is one greater than max producer ID
            })
            .expect("wav user");
    }

    {
        // At the end of this scope, dirs_tx dropped - we're done sending directories.
        let (dirs_tx, dirs_rx) = bounded(0);
        for w in 0..N_PRODUCERS {
            let dirs_rx = dirs_rx.clone();
            let done_tx = done_tx.clone();
            let wdescs_tx = wdescs_tx.clone();
            let cfg = cfg.clone();
            thread::Builder::new()
                .name("wav surveyor".to_string())
                .spawn(move || {
                    wav::survey_wavs(w, cfg, dirs_rx, wdescs_tx, done_tx);
                })
                .expect("spawning wav surveyor");
        }
        for d in cfg.dirs {
            for entry in WalkDir::new(d).into_iter().filter_map(|e| e.ok()) {
                if !cfg.excluded_wavs.contains(entry.path()) {
                    let p = path::PathBuf::from(entry.path());
                    dirs_tx.send(p).unwrap();
                }
            }
        }
    }
    let mut n_workers = N_PRODUCERS + 1;
    while n_workers > 0 {
        match done_rx.recv() {
            Ok(worker_id) => {
                println!("worker {} finished", worker_id);
                n_workers -= 1;
            }
            Err(RecvError) => {
                println!("work generator done channel was closed");
            }
        }
    }
}
