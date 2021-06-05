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
const GRAIN_MS: u32 = 1000;
const GRAIN_BUF_N_SAMPLES: usize = 2048;
const MIN_GRAIN_SIZE_FRACTION: f32 = 0.6;
const WAV_MAX_TTL: u32 = 10;

struct Grain {
    start: u32,
    pos: u32, // current position in grain
    len: u32,
    max_len: u32,
}

impl Grain {
    fn new(sr: u32) -> Self {
        let sr_ms = sr / 1000;
        let len = GRAIN_MS * sr_ms;
        Grain {
            start: 0,
            pos: 0,
            max_len: len,
            len: len,
        }
    }
    fn land(&mut self, n: u32) {
        let mut rng = rand::thread_rng();
        let g_right = 1.0 - MIN_GRAIN_SIZE_FRACTION;
        let g_right_fraction = rand_distr::Uniform::from(0.0..1.0).sample(&mut rng);
        let g_extra = g_right * g_right_fraction;
        let g_size = self.len as f32 * (MIN_GRAIN_SIZE_FRACTION + g_extra);
        self.len = g_size as u32;
        self.start = rand_distr::Uniform::from(0..n - self.len).sample(&mut rng);
        self.pos = 0;
    }
    // https://en.wikipedia.org/wiki/Window_function#Tukey_window
    fn amplitude(&self, alpha: Option<f32>) -> f32 {
        let alpha = match alpha {
            Some(a) => a,
            None => DEFAULT_TUKEY_WINDOW_ALPHA,
        };

        let n = self.pos as f32;
        let len = self.len as f32;
        let n = if n >= (len / 2.0) { len - n } else { n };

        if n < alpha * len / 2.0 {
            let x = (2.0 * std::f32::consts::PI * n) / (alpha * len);
            0.5 * (1.0 - x.cos())
        } else {
            1.0
        }
    }
}

// return max i16 sample as f32 fraction of 1.0
fn max_amplitude(mut rd: hound::WavReader<BufReader<File>>) -> f32 {
    let samples: hound::WavSamples<'_, std::io::BufReader<std::fs::File>, i16> = rd.samples();
    let mut max: i16 = 0;
    for s in samples {
        let s = s.expect("expected i16 sample");
        let s = s as i16;
        if s.abs() > max {
            max = s.abs();
        }
    }
    max as f32 / (i16::MAX as f32)
}

fn describe_wav(path: path::PathBuf) -> Option<WavDesc> {
    if let Ok(reader) = hound::WavReader::open(&path) {
        Some(WavDesc {
            path,
            n_samples: reader.duration(),
            spec: reader.spec(),
            max_amplitude: None,
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
    max_amplitude: Option<f32>,
}

fn play(
    client: jack::Client,
    done_tx: chan::Sender<()>,
    samples_rx: chan::Receiver<(usize, Vec<f32>)>,
) {
    println!("play starting");
    let mut out_left = client
        .register_port("acouwalk_out_L", jack::AudioOut::default())
        .unwrap();
    let mut out_right = client
        .register_port("acouwalk_out_R", jack::AudioOut::default())
        .unwrap();
    let mut n_channels = 2;
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
            if samples.len() - consumed < n * n_channels {
                match samples_rx.recv() {
                    Some((n_src_channels, mut new_samples)) => {
                        println!(
                            "play received {} {}-channel samples",
                            new_samples.len(),
                            n_src_channels
                        );
                        n_channels = n_src_channels;
                        if n_channels != 2 {
                            // Or you could skip all non-stereo WAVs during collection!
                            // Not OK to mix interleaved stereo and mono in the same buffer below.
                            panic!("all the audio received here should be stereo");
                        }
                        samples.append(&mut new_samples)
                    }
                    None => {
                        println!("play received EOF from samples channel");
                        jackdone_tx.send(());
                        return jack::Control::Quit;
                    }
                }
            }
            let n = std::cmp::min(n, samples.len() / n_channels);
            // println!("n:{}", n);
            for i in 0..n {
                let src_off = consumed + i * n_channels;
                outl[i] = samples[src_off];
                if n_channels == 1 {
                    outr[i] = samples[src_off];
                } else {
                    outr[i] = samples[src_off + 1];
                }
                // println!("outl[{}], outr[{}] <- {}, {}", i, i, outl[i], outr[i]);
            }
            consumed += n * n_channels;

            // Continue as normal
            jack::Control::Continue
        },
    );

    // 4. activate the client
    let active_client = client.activate_async((), process).unwrap();
    // processing starts here

    let _ = jackdone_rx.recv();
    println!("play received playdone message from JACK handler");

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

    let (samples_tx, samples_rx) = chan::sync(5);
    let (playdone_tx, playdone_rx) = chan::sync(0);
    generate_samples(samples_tx, client.sample_rate(), wavs);
    play(client, playdone_tx, samples_rx);
    playdone_rx.recv();
    println!("use_wavs received playdone message");
}

fn grain_samples(grain_ms: usize, sr: u32) -> usize {
    let smpls_per_ms = (sr as f64) / 1000.0;
    let smpls_per_grain = (grain_ms as f64) * smpls_per_ms;
    smpls_per_grain.round() as usize
}

fn make_grains(
    grain_maker_id: u32,
    wavs: &Vec<WavDesc>,
    grains_tx: chan::Sender<f32>,
    sink_sr: usize,
) {
    let wavs = wavs.clone();
    let g = Grain::new(sink_sr as u32);
    thread::spawn(move || {
        let mut rng = rand::thread_rng();
        loop {
            let which = select_wavs(&wavs, 1)
                .unwrap()
                .iter()
                .take(1)
                .last()
                .unwrap();
            let wav = &wavs[*which];
            let r = hound::WavReader::open(wav.path).ok().unwrap();
            let src_sr = r.spec().sample_rate;
            let src_per_sink = src_sr as f32 * sink_sr as f32;
            let ttl = rand_distr::Uniform::from(1..WAV_MAX_TTL).sample(&mut rng);
            for _ in 0..ttl {
                g.land(wav.n_samples);
                let n_read = (g.len as f32 * src_per_sink).ceil() as usize;
                r.seek(g.start);
                let src_samples: Vec<f32> = r
                    .samples()
                    .take(n_read)
                    .map(|e: Result<i16, hound::Error>| e.ok().unwrap() as f32 / i16::MAX as f32)
                    .collect();
                let sink_samples = convert(
                    src_sr,
                    sink_sr as u32,
                    2,
                    ConverterType::SincBestQuality,
                    &src_samples[..],
                );
                // apply grain envelope
                // push to vec
                // split and send data if enough for buf
            }
        }
    });
}

fn generate_samples(
    samples_tx: chan::Sender<(usize, Vec<f32>)>,
    sink_sr: usize,
    wavs: Vec<WavDesc>,
) -> u32 {
    let mut grains_rxs: Vec<chan::Receiver<f32>> = Vec::new();
    for i in 0..N_GRAINS {
        let (grains_tx, grains_rx) = chan::sync(2);
        make_grains(i, &wavs, grains_tx, sink_sr);
        grains_rxs.push(grains_rx);
    }
    // now each grain maker will send JACK-ready samples in chunks mixed below

    thread::spawn(move || {
        let mut r = hound::WavReader::open(&wavs[0].path).expect("opening WAV file");
        let n_channels = r.spec().channels; // TODO: Use for each WAV
        let src_sr = r.spec().sample_rate;
        let n_src_grain = grain_samples(GRAIN_MS, src_sr);
        let converter1 =
            Samplerate::new(ConverterType::SincBestQuality, src_sr, sink_sr as u32, 1).unwrap();
        let converter2 =
            Samplerate::new(ConverterType::SincBestQuality, src_sr, sink_sr as u32, 2).unwrap();
        let mut eof = false;
        while !eof {
            let mut grain_samples: Vec<f32> = Vec::new();
            for _ in 0..n_src_grain {
                let samples: hound::WavSamples<'_, std::io::BufReader<std::fs::File>, i16> =
                    r.samples();
                match samples.take(1).last() {
                    Some(res) => {
                        let s = res.expect("sample") as f32;
                        let s = s / (i16::MAX as f32);
                        grain_samples.push(s)
                    }
                    None => {
                        println!("EOF on input WAV");
                        eof = true;
                        break;
                    }
                }
            }
            let converter = match n_channels {
                1 => &converter1,
                2 => &converter2,
                _ => panic!("more than two channels unsupported"),
            };
            // println!("grain_samples length before sr conversion: {}", grain_samples.len());
            let grain_samples = converter.process(&grain_samples[..]).expect("convert sr");
            println!("sending grain_samples of len {}", grain_samples.len());
            samples_tx.send((n_channels as usize, grain_samples));
        }
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
