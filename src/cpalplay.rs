// https://docs.rs/cpal/0.13.3/cpal/
use rand_distr::Distribution;
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, Sample, SampleFormat, SampleRate, StreamConfig};
use crossbeam_channel::{bounded, Receiver, RecvError, Sender};

pub const SAMPLE_RATE: usize = 44100;

pub fn play_to_cpal(done_tx: Sender<()>, samples_rx: Receiver<Vec<f32>>) {
    println!("play_to_cpal starting");
    let mut samples: Vec<f32> = Vec::new();
    let (cb_done_tx, cb_done_rx) = bounded(0);
    let mut consumed = 0;
    let callback = move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
        let n = data.len();
        if consumed > SAMPLE_RATE {
            samples = samples.split_off(consumed);
            consumed = 0;
        }
        if samples.len() - consumed < n {
            // (2 for stereo)
            match samples_rx.recv() {
                Ok(mut new_samples) => {
                    println!("play received {} stereo samples", new_samples.len() / 2);
                    samples.append(&mut new_samples)
                }
                Err(RecvError) => {
                    println!("play received EOF from samples channel");
                    cb_done_tx.send(()).unwrap();
                    return;
                }
            }
        }
        let n = std::cmp::min(n, samples.len());

        for i in 0..n {
            data[i] = samples[consumed + i];
        }
        consumed += n;
    };
    let (device, config, sample_format) = prep_for_stream();
    let err_fn = |err| eprintln!("an error occurred on the output audio stream: {}", err);
    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(&config, callback, err_fn),
        SampleFormat::I16 => device.build_output_stream(&config, write_silence::<i16>, err_fn),
        SampleFormat::U16 => device.build_output_stream(&config, write_silence::<u16>, err_fn),
    }
    .unwrap();
    stream.play().unwrap();

    let _ = cb_done_rx.recv();
    println!("play received playdone message from cpal callback");
    println!("play_to_cpal is done");
    done_tx.send(()).unwrap();
}

fn prep_for_stream() -> (Device, StreamConfig, SampleFormat) {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .expect("no output device available");
    let supported_configs_range = device
        .supported_output_configs()
        .expect("error while querying configs");
    let supported_config = supported_configs_range
        .filter(|c| c.channels() == 2 && c.sample_format() == SampleFormat::F32)
        .next()
        .expect("no supported config?!")
        .with_sample_rate(SampleRate(SAMPLE_RATE as u32));
    println!("selected config: {:#?}", supported_config);
    let sample_format = supported_config.sample_format();
    let config = supported_config.into();
    (device, config, sample_format)
}

pub fn cpal_demo() {
    let (device, config, sample_format) = prep_for_stream();
    let err_fn = |err| eprintln!("an error occurred on the output audio stream: {}", err);
    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(&config, write_noise, err_fn),
        SampleFormat::I16 => device.build_output_stream(&config, write_silence::<i16>, err_fn),
        SampleFormat::U16 => device.build_output_stream(&config, write_silence::<u16>, err_fn),
    }
    .unwrap();
    stream.play().unwrap();

    thread::sleep(Duration::from_secs(5));
}

fn write_noise(data: &mut [f32], info: &cpal::OutputCallbackInfo) {
    println!(
        "write_noise for {} samples with info:{:?}",
        data.len(),
        info
    );
    let mut rng = rand::thread_rng();
    let unif = rand_distr::Uniform::from(0.0..1.0);
    let mut s = 0.0;

    for (i, sample) in data.iter_mut().enumerate() {
        if i % 2 == 0 {
            s = unif.sample(&mut rng);
        }
        *sample = Sample::from(&s);
    }
}

fn write_silence<T: Sample>(data: &mut [T], info: &cpal::OutputCallbackInfo) {
    println!(
        "write_silence for {} samples with info:{:?}",
        data.len(),
        info
    );
    for sample in data.iter_mut() {
        *sample = Sample::from(&0.0);
    }
    println!("write_silence done.");
}
