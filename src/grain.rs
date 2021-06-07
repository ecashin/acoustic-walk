use rand_distr::Distribution;
use samplerate::{convert, ConverterType};
use std::thread;

use crate::wav::WavDesc;

const DEFAULT_TUKEY_WINDOW_ALPHA: f32 = 0.5;
pub const N_GRAINS: u32 = 5;
const GRAIN_MS: u32 = 1000;
const GRAIN_BUF_N_SAMPLES: usize = 1024 * 1024;
const MIN_GRAIN_SIZE_FRACTION: f32 = 0.6;
const WAV_MAX_TTL: u32 = 10;

pub struct Grain {
    start: u32,
    len: u32,
    max_len: u32,
}

impl Grain {
    pub fn new(sr: u32) -> Self {
        let sr_ms = sr / 1000;
        let len = GRAIN_MS * sr_ms;
        Grain {
            start: 0,
            max_len: len,
            len: len,
        }
    }
    // Toss this grain in the air and let it randomly land somewhere.
    pub fn toss(&mut self, n: u32) {
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
    pub fn amplitude(&self, pos: usize, alpha: Option<f32>) -> f32 {
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

pub fn make_grains(
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
            let which = crate::wav::select_wavs(&wavs, 1)
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
                let mut too_loud = false;
                g.toss(wav.n_samples);
                r.seek(g.start).ok();
                let mut src_samples: Vec<f32> = r
                    .samples()
                    .take((g.len * 2) as usize)
                    .map(|e: Result<i16, hound::Error>| {
                        let s = e.ok().unwrap();
                        if s == i16::MAX || s == i16::MIN {
                            too_loud = true;
                        }
                        s as f32 / i16::MAX as f32
                    })
                    .enumerate()
                    .map(|(i, s)| s * g.amplitude(i / 2, None))
                    .collect();
                if too_loud {
                    println!("muting {:?} at too-loud sample index {}", wav.path, g.start);
                    src_samples = src_samples.iter().map(|_| 0.0).collect();
                }
                if src_sr == sink_sr as u32 {
                    send_buf.append(&mut src_samples);
                } else {
                    let mut sink_samples = convert(
                        src_sr,
                        sink_sr as u32,
                        2,
                        ConverterType::SincBestQuality,
                        &src_samples[..],
                    )
                    .expect("converting sample rate");
                    send_buf.append(&mut sink_samples);
                }
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
