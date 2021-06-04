use probability::prelude::*;
// use rand::prelude::*;
use rand_distr::Dirichlet;
use rand_distr::Distribution;
use samplerate::{convert, ConverterType, Samplerate};
use std::fs::File;
use std::io::BufReader;
use std::{env, path, thread};
use walkdir::WalkDir;

const DEFAULT_TUKEY_WINDOW_ALPHA: f32 = 0.5;
const N_PRODUCERS: u32 = 10;
const N_GRAINS: u32 = 5;
const GRAIN_MS: usize = 1000;

struct Grain {
    start: u32,
    pos: u32,
    end: u32,
}

impl Grain {
    // https://en.wikipedia.org/wiki/Window_function#Tukey_window
    fn amplitude(&self, alpha: Option<f32>) -> f32 {
        let alpha = match alpha {
            Some(a) => a,
            None => DEFAULT_TUKEY_WINDOW_ALPHA,
        };

        let n = self.pos as f32;
        let len = (self.end - self.start) as f32;
        let n = if n >= (len / 2.0) { len - n } else { n };

        if n < alpha * len / 2.0 {
            let x = (2.0 * std::f32::consts::PI * n) / (alpha * len);
            0.5 * (1.0 - x.cos())
        } else {
            1.0
        }
    }
}

fn max_amplitude(mut rd: hound::WavReader<BufReader<File>>) -> u32 {
    if true {
        return 0; // fast and wrong, for now
    }
    let samples: hound::WavSamples<'_, std::io::BufReader<std::fs::File>, i16> = rd.samples();
    let mut max: i64 = 0;
    for s in samples {
        let s = s.expect("expected i16 sample");
        let s = s as i64;
        if s.abs() > max {
            max = s.abs();
        }
    }
    max as u32
}

fn describe_wav(path: path::PathBuf) -> Option<WavDesc> {
    if let Ok(reader) = hound::WavReader::open(&path) {
        Some(WavDesc {
            path,
            n_samples: reader.duration(),
            spec: reader.spec(),
            max_amplitude: max_amplitude(reader),
        })
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

#[derive(Clone)]
struct WavDesc {
    // reader: hound::WavReader<BufReader<File>>,
    path: path::PathBuf,
    n_samples: u32,
    spec: hound::WavSpec,
    max_amplitude: u32,
}

fn play(client: jack::Client, done_tx: chan::Sender<()>, samples_rx: chan::Receiver<Vec<f32>>) {
    println!("play starting");
    let mut out_left = client
        .register_port("acouwalk_out_L", jack::AudioOut::default())
        .unwrap();
    let mut out_right = client
        .register_port("acouwalk_out_R", jack::AudioOut::default())
        .unwrap();
    let n_channels = 2;
    let mut samples: Vec<f32> = Vec::new();
    let process = jack::ClosureProcessHandler::new(
        move |_: &jack::Client, ps: &jack::ProcessScope| -> jack::Control {
            let outl = out_left.as_mut_slice(ps);
            let outr = out_right.as_mut_slice(ps);
            let n = outl.len();
            if samples.len() < n * 2 {
                match samples_rx.recv() {
                    Some(mut new_samples) => samples.append(&mut new_samples),
                    None => return jack::Control::Quit,
                }
            }
            let n = std::cmp::min(n, samples.len() / 2);
            for i in 0..n {
                let src_off = i * n_channels;
                outl[i] = samples[src_off];
                outr[i] = samples[src_off + 1];
            }

            // Continue as normal
            jack::Control::Continue
        },
    );

    // 4. activate the client
    let active_client = client.activate_async((), process).unwrap();
    // processing starts here

    // 6. Optional deactivate. Not required since active_client will deactivate on
    // drop, though explicit deactivate may help you identify errors in
    // deactivate.
    active_client.deactivate().unwrap();

    println!("play is done");
    done_tx.send(());
}

fn select_wavs(wavs: &Vec<WavDesc>, n: usize) -> Option<Vec<usize>> {
    if wavs.len() == 0 {
        return None;
    }
    let lens: Vec<f64> = wavs.iter().map(|e| e.n_samples as f64).collect();
    let dirichlet = Dirichlet::new(&lens).unwrap();
    let mut source = source::default();
    let probs = dirichlet.sample(&mut rand::thread_rng());
    let cat = probability::distribution::Categorical::new(&probs[..]);
    let decider = Independent(&cat, &mut source);
    Some(decider.take(n).collect::<Vec<_>>())
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

    let (samples_tx, samples_rx) = chan::sync(100);
    let (done_tx, done_rx) = chan::sync(0);
    let mut n_wait = 1 + generate_samples(samples_tx, done_tx.clone(), client.sample_rate(), wavs);
    play(client, done_tx, samples_rx);
    while n_wait > 0 {
        done_rx.recv();
        n_wait -= 1;
    }
}

fn grain_samples(grain_ms: usize, sr: u32) -> usize {
    let smpls_per_ms = (sr as f64) / 1000.0;
    let smpls_per_grain = (grain_ms as f64) * smpls_per_ms;
    smpls_per_grain.round() as usize
}

fn generate_samples(
    samples_tx: chan::Sender<Vec<f32>>,
    done_tx: chan::Sender<()>,
    sink_sr: usize,
    wavs: Vec<WavDesc>,
) -> u32 {
    println!("generate_samples using wav {:?} of length {}", &wavs[0].path, &wavs[0].n_samples);
    thread::spawn(move || {
        let mut r = hound::WavReader::open(&wavs[0].path).expect("opening WAV file");
        let src_sr = r.spec().sample_rate;
        let n_src_grain = grain_samples(GRAIN_MS, src_sr);
        let converter =
            Samplerate::new(ConverterType::SincBestQuality, src_sr, sink_sr as u32, 2).unwrap();
        let mut eof = false;
        while !eof {
            let mut grain_samples: Vec<f32> = Vec::new();
            for _ in 0..n_src_grain {
                let samples: hound::WavSamples<'_, std::io::BufReader<std::fs::File>, i16> =
                    r.samples();
                match samples.take(1).last() {
                    Some(res) => {
                        grain_samples.push((res.expect("sample") as f32) * (i16::MAX as f32))
                    }
                    None => {
                        println!("EOF on input WAV");
                        eof = true;
                        break;
                    }
                }
            }
            println!("grain_samples length before sr conversion: {}", grain_samples.len());
            let grain_samples = converter
                .process_last(&grain_samples[..])
                .expect("convert sr");
            println!("sending grain_samples of len {}", grain_samples.len());
            samples_tx.send(grain_samples);
        }
        println!("generate_samples is done");
        done_tx.send(());
    });

    N_GRAINS
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
