// https://docs.rs/cpal/0.13.3/cpal/
use rand_distr::Distribution;
use std::thread;
use std::time::Duration;

use cpal::{Sample, SampleFormat, SampleRate};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub fn cpal_demo() {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    let supported_configs_range = device.supported_output_configs()
        .expect("error while querying configs");
    let supported_config = supported_configs_range
        .filter(|c| c.channels() == 2 && c.sample_format() == SampleFormat::F32)
        .next()
        .expect("no supported config?!")
        .with_sample_rate(SampleRate(44100));
    println!("selected config: {:#?}", supported_config);
    let err_fn = |err| eprintln!("an error occurred on the output audio stream: {}", err);
    let sample_format = supported_config.sample_format();
    let config = supported_config.into();
    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(&config, write_noise, err_fn),
        SampleFormat::I16 => device.build_output_stream(&config, write_silence::<i16>, err_fn),
        SampleFormat::U16 => device.build_output_stream(&config, write_silence::<u16>, err_fn),
    }.unwrap();

    stream.play().unwrap();

    thread::sleep(Duration::from_secs(5));
}

fn write_noise(data: &mut [f32], info: &cpal::OutputCallbackInfo) {
    println!("write_noise for {} samples with info:{:?}", data.len(), info);
    let mut rng = rand::thread_rng();
    let unif = rand_distr::Uniform::from(0.0..1.0);
    let mut s = 0.0;

    for (i, sample) in data.iter_mut().enumerate() {
        if i % 2 == 0 {
            s = unif.sample(&mut rng);
        }
        *sample = Sample::from(&s);
    }
    // println!("write_noise done.");
}

fn write_silence<T: Sample>(data: &mut [T], info: &cpal::OutputCallbackInfo) {
    println!("write_silence for {} samples with info:{:?}", data.len(), info);
    for sample in data.iter_mut() {
        *sample = Sample::from(&0.0);
    }
    println!("write_silence done.");
}
