use clap::App;
use probability::prelude::*;
use rand_distr::Dirichlet;
use rand_distr::Distribution;
use samplerate::{convert, ConverterType};
use std::{fs, io, path, thread};
use std::io::prelude::*;
use walkdir::WalkDir;

const DEFAULT_TUKEY_WINDOW_ALPHA: f32 = 0.5;
const N_PRODUCERS: u32 = 10;
const N_GRAINS: u32 = 5;
const GRAIN_MS: u32 = 1000;
const GRAIN_BUF_N_SAMPLES: usize = 1024 * 1024;
const MIN_GRAIN_SIZE_FRACTION: f32 = 0.6;
const WAV_MAX_TTL: u32 = 10;

struct Grain {
    start: u32,
    len: u32,
    max_len: u32,
}

impl Grain {
    fn new(sr: u32) -> Self {
        let sr_ms = sr / 1000;
        let len = GRAIN_MS * sr_ms;
        Grain {
            start: 0,
            max_len: len,
            len: len,
        }
    }
    // Toss this grain in the air and let it randomly land somewhere.
    fn toss(&mut self, n: u32) {
        let mut rng = rand::thread_rng();
        let g_right = 1.0 - MIN_GRAIN_SIZE_FRACTION;
        let g_right_fraction = rand_distr::Uniform::from(0.0..1.0).sample(&mut rng);
        // The random "extra" above-minimum length avoids grain synchronization.
        let g_extra = g_right * g_right_fraction;
        let g_size = self.max_len as f32 * (MIN_GRAIN_SIZE_FRACTION + g_extra);
        self.len = g_size as u32;
        let rounding_error = 1; // one-sample safety margin
        self.start = rand_distr::Uniform::from(0..n - rounding_error - self.len).sample(&mut rng);
    }
    // https://en.wikipedia.org/wiki/Window_function#Tukey_window
    fn amplitude(&self, pos: usize, alpha: Option<f32>) -> f32 {
        let alpha = match alpha {
            Some(a) => a,
            None => DEFAULT_TUKEY_WINDOW_ALPHA,
        };

        let n = pos as f32;
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

fn describe_wav(path: path::PathBuf) -> Option<WavDesc> {
    if let Ok(reader) = hound::WavReader::open(&path) {
        if reader.spec().channels != 2 {
            None
        } else {
            Some(WavDesc {
                path,
                n_samples: reader.duration(),
                spec: reader.spec(),
            })
        }
    } else {
        None
    }
}

fn survey_wavs(
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
    path: path::PathBuf,
    n_samples: u32,
    spec: hound::WavSpec,
}

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

    let (samples_tx, samples_rx) = chan::sync(2);
    let (playdone_tx, playdone_rx) = chan::sync(0);
    generate_samples(samples_tx, client.sample_rate(), wavs);
    play(client, playdone_tx, samples_rx);
    playdone_rx.recv();
    println!("use_wavs received playdone message");
}

fn make_grains(
    grain_maker_id: u32,
    wavs: &Vec<WavDesc>,
    grains_tx: chan::Sender<Vec<f32>>,
    sink_sr: usize,
) {
    let mut g = Grain::new(sink_sr as u32);
    let wavs = wavs.clone();
    thread::spawn(move || {
        println!("grain maker {} starting", grain_maker_id);
        let mut rng = rand::thread_rng();
        let mut send_buf: Vec<f32> = Vec::new();
        loop {
            let which = select_wavs(&wavs, 1)
                .unwrap()
                .iter()
                .take(1)
                .map(|e| *e)
                .last()
                .unwrap();
            let wav = &wavs[which];
            println!("grain maker {} chose WAV {:?}", grain_maker_id, wav.path);
            let mut r = hound::WavReader::open(wav.path.clone()).ok().unwrap();
            let src_sr = r.spec().sample_rate;
            let ttl = rand_distr::Uniform::from(1..WAV_MAX_TTL).sample(&mut rng);
            for _ in 0..ttl {
                g.toss(wav.n_samples);
                r.seek(g.start).ok();
                let src_samples: Vec<f32> = r
                    .samples()
                    .take((g.len * 2) as usize)
                    .map(|e: Result<i16, hound::Error>| e.ok().unwrap() as f32 / i16::MAX as f32)
                    .enumerate()
                    .map(|(i, s)| s * g.amplitude(i / 2, None))
                    .collect();
                let mut sink_samples = convert(
                    src_sr,
                    sink_sr as u32,
                    2,
                    ConverterType::SincBestQuality,
                    &src_samples[..],
                )
                .expect("converting sample rate");
                send_buf.append(&mut sink_samples);
                if send_buf.len() >= GRAIN_BUF_N_SAMPLES {
                    let send_part: Vec<f32> = send_buf
                        .iter()
                        .take(GRAIN_BUF_N_SAMPLES)
                        .map(|e| *e)
                        .collect();
                    let new_len = send_buf.len() - GRAIN_BUF_N_SAMPLES;
                    for (i, j) in (0..new_len).enumerate() {
                        send_buf[i] = send_buf[j];
                    }
                    send_buf.truncate(new_len);
                    grains_tx.send(send_part);
                }
            }
        }
    });
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
        make_grains(i, &wavs, grains_tx, sink_sr);
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
    let matches = App::new("acouwalk")
        .author("Ed.Cashin@acm.org")
        .about("stereo granular streamer for JACK")
        .arg(clap::Arg::from_usage(
            "-e --exclude=[FILE] 'Read excluded WAVs from file'",
        ))
        .arg(
            clap::Arg::with_name("dirs")
                .required(true)
                .min_values(1)
                .help("<WAV-directory>..."),
        )
        .get_matches();

    let mut excluded_wavs: Vec<std::path::PathBuf> = Vec::new();
    if let Some(e) = matches.value_of("exclude") {
        println!("e:{}", e);
        let f = fs::File::open(e).ok().unwrap();
        let reader = io::BufReader::new(f);
        for line in reader.lines() {
            let line = line.ok().unwrap();
            println!("excluding {}", line);
            let path = std::path::Path::new(&line);
            excluded_wavs.push(path.to_path_buf());
        }
    }

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
    if let Some(dirs) = matches.values_of("dirs") {
        let dirs: Vec<&str> = dirs.collect();
        {
            // At the end of this scope, dirs_tx dropped - we're done sending directories.
            let (dirs_tx, dirs_rx) = chan::sync(0);
            for w in 0..N_PRODUCERS {
                let dirs_rx = dirs_rx.clone();
                let done_tx = done_tx.clone();
                let wdescs_tx = wdescs_tx.clone();
                thread::spawn(move || {
                    survey_wavs(w, dirs_rx, wdescs_tx, done_tx);
                });
            }
            for d in dirs {
                for entry in WalkDir::new(d).into_iter().filter_map(|e| e.ok()) {
                    if ! excluded_wavs.iter().any(|i| i == entry.path()) {
                        let p = path::PathBuf::from(entry.path());
                        dirs_tx.send(p);
                    }
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
