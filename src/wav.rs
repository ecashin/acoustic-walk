use crate::config::PlayConfig;
use crossbeam_channel::{bounded, Receiver, Sender};
use probability::prelude::*;
use rand_distr::Dirichlet;
use rand_distr::Distribution;
use std::{fs, io, path, thread};

#[derive(Clone)]
pub struct WavDesc {
    pub path: path::PathBuf,
    pub n_samples: u32,
    pub spec: hound::WavSpec,
    pub ms_for_choice: f32,
}

pub fn start_wav_picker(
    n_producers: u32,
    wdescs_rx: Receiver<Option<WavDesc>>,
) -> Receiver<WavDesc> {
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
    println!(
        "WAV picker collected {} wav descriptions - spawning thread",
        wavs.len()
    );
    let (wavpick_tx, wavpick_rx) = bounded(0);
    thread::Builder::new()
        .name("wav selector".to_string())
        .spawn(move || {
            while let Some(which) = crate::wav::select_wavs(&wavs, 1) {
                let which = which.iter().take(1).copied().last().unwrap();
                let wav = &wavs[which];
                println!("wav picker: {:?}", wav.path);
                wavpick_tx.send(wav.clone()).unwrap();
            }
        })
        .expect("spawning wav selector");

    wavpick_rx
}

pub fn describe_wav(path: path::PathBuf, cap_ms: Option<u32>) -> Option<WavDesc> {
    if let Ok(reader) = hound::WavReader::open(&path) {
        if reader.spec().channels != 2 {
            None
        } else {
            let path_str = format!("{:?}", path);
            Some(WavDesc {
                path,
                n_samples: reader.duration(),
                spec: reader.spec(),
                ms_for_choice: capped_ms(&path_str, reader, cap_ms),
            })
        }
    } else {
        None
    }
}

fn capped_ms(
    path: &str,
    reader: hound::WavReader<io::BufReader<fs::File>>,
    cap_ms: Option<u32>,
) -> f32 {
    let sr_ms = (reader.spec().sample_rate as f32) / 1000.0;
    let n_samples = reader.duration() as f32;
    let wav_ms = n_samples / sr_ms;
    match cap_ms {
        None => wav_ms,
        Some(cap_ms) => {
            let cap_ms = cap_ms as f32;
            if wav_ms > cap_ms {
                println!("capping {} -> {} for {:?}", wav_ms, cap_ms, path);
                cap_ms
            } else {
                wav_ms
            }
        }
    }
}

pub fn select_wavs(wavs: &[WavDesc], n: usize) -> Option<Vec<usize>> {
    if wavs.is_empty() {
        return None;
    }
    let lens: Vec<f64> = wavs.iter().map(|e| e.ms_for_choice as f64).collect();
    let dirichlet = Dirichlet::new(&lens).unwrap();
    let mut source = source::default();
    let probs = dirichlet.sample(&mut rand::thread_rng());
    let cat = probability::distribution::Categorical::new(&probs[..]);
    let decider = Independent(&cat, &mut source);
    Some(decider.take(n).collect::<Vec<_>>())
}

pub fn survey_wavs(
    worker_id: u32,
    cfg: PlayConfig,
    paths_rx: Receiver<path::PathBuf>,
    wdescs_tx: Sender<Option<WavDesc>>,
    done_tx: Sender<u32>,
) {
    for path in paths_rx {
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if let Some(ext) = ext.to_str() {
                    if ext.eq_ignore_ascii_case("wav") {
                        if let Some(wdesc) = describe_wav(path, cfg.cap_ms) {
                            println!("worker:{} sending for {:?}", worker_id, &wdesc.path);
                            wdescs_tx.send(Some(wdesc)).unwrap();
                        }
                    }
                }
            }
        }
    }
    wdescs_tx.send(None).unwrap();
    done_tx.send(worker_id).unwrap();
}
