use probability::prelude::*;
// use rand::prelude::*;
use rand_distr::Dirichlet;
use rand_distr::Distribution;
use samplerate::{convert, ConverterType};
use std::fs::File;
use std::io::BufReader;
use std::{env, path, thread};
use walkdir::WalkDir;

const DEFAULT_TUKEY_WINDOW_ALPHA: f32 = 0.5;
const N_PRODUCERS: u32 = 10;

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
        let n = if n >= (len / 2.0) {
            len - n
        } else {
            n
        };

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
        return 0  // fast and wrong, for now
    }
    let samples: hound::WavSamples<'_, std::io::BufReader<std::fs::File>, i16> =
                rd.samples();
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

fn play_one(wav: WavDesc) {
    println!(
        "path:{:?} spec:{:?} n_samples:{}",
        wav.path,
        wav.spec,
        wav.n_samples
    );
    let mut wav_reader = hound::WavReader::open(wav.path).unwrap();

    // https://github.com/RustAudio/rust-jack/blob/main/examples/sine.rs example

    // 1. open a client
    let (client, status) =
        jack::Client::new("acouwalk", jack::ClientOptions::NO_START_SERVER).unwrap();
    println!("new client:{:?} status:{:?}", client, status);

    // 2. register ports
    let mut out_left = client
        .register_port("acouwalk_out_L", jack::AudioOut::default())
        .unwrap();
    let mut out_right = client
        .register_port("acouwalk_out_R", jack::AudioOut::default())
        .unwrap();
    let n_channels = wav.spec.channels as usize;

    // read interleaved samples
    if wav.spec.bits_per_sample != 16 {
        panic!("need other than 16-bit sample support");
    }

    // https://docs.rs/samplerate/0.2.4/samplerate/fn.convert.html
    let source_sr = wav.spec.sample_rate as usize;
    let sink_sr = client.sample_rate();

    let process = jack::ClosureProcessHandler::new(
        move |_: &jack::Client, ps: &jack::ProcessScope| -> jack::Control {
            // TODO: Try to re-use a converter inside thread if possible.
            let outl = out_left.as_mut_slice(ps);
            let outr = out_right.as_mut_slice(ps);
            let n = outl.len();
            let n_source = ((n * source_sr) / sink_sr) + n_channels;

            let samples: hound::WavSamples<'_, std::io::BufReader<std::fs::File>, i16> =
                wav_reader.samples();
            let samples: Vec<_> = samples
                .take(n_source * n_channels)
                .map(|e| (e.ok().unwrap() as f32) / (i16::MAX as f32))
                .collect();
            let samples = convert(
                source_sr as u32,
                sink_sr as u32,
                n_channels,
                ConverterType::SincBestQuality,
                &samples,
            )
            .unwrap();
            for i in 0..outl.len() {
                let src_off = i * n_channels;
                outl[i] = samples[src_off];
                if n_channels > 1 {
                    outr[i] = samples[src_off + 1];
                } else {
                    outr[i] = samples[src_off];
                }
            }

            // Continue as normal
            jack::Control::Continue
        },
    );

    // 4. activate the client
    let active_client = client.activate_async((), process).unwrap();
    // processing starts here

    let _: String = text_io::read!();

    // 6. Optional deactivate. Not required since active_client will deactivate on
    // drop, though explicit deactivate may help you identify errors in
    // deactivate.
    active_client.deactivate().unwrap();
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

fn consume(n_producers: u32, wdescs_rx: chan::Receiver<Option<WavDesc>>) {
    let mut n = n_producers;
    let mut wavs: Vec<WavDesc> = Vec::new();
    while n > 0 {
        let wdesc_opt = wdescs_rx.recv().expect("producers don't close work chan");
        if let Some(wdesc) = wdesc_opt {
            println!(
                "consumer received {:?}:{:?}:{:?} with {} producers remaining",
                wdesc.path,
                wdesc.spec,
                wdesc.n_samples,
                n
            );
            wavs.push(wdesc);
        } else {
            println!("consumer received None");
            n -= 1;
        }
    }
    println!("collected {} wav descriptions", wavs.len());
    let which = select_wavs(&wavs, 1).expect("couldn't pick one track");
    let i: usize = which[0];
    play_one(wavs[i].clone());
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
