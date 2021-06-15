// https://docs.rs/cpal/0.13.3/cpal/
use std::thread;
use std::time::Duration;

use cpal::{Sample, SampleFormat};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub fn cpal_demo() {
    let host = cpal::default_host();
    let device = host.default_output_device().expect("no output device available");
    let supported_configs_range = device.supported_output_configs()
        .expect("error while querying configs");
    let supported_config = supported_configs_range
        .filter(|c| c.channels() == 2)
        .next()
        .expect("no supported config?!")
        .with_max_sample_rate();
    println!("selected config: {:#?}", supported_config);
    let err_fn = |err| eprintln!("an error occurred on the output audio stream: {}", err);
    let sample_format = supported_config.sample_format();
    let config = supported_config.into();
    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(&config, write_silence::<f32>, err_fn),
        SampleFormat::I16 => device.build_output_stream(&config, write_silence::<i16>, err_fn),
        SampleFormat::U16 => device.build_output_stream(&config, write_silence::<u16>, err_fn),
    }.unwrap();

    stream.play().unwrap();

    thread::sleep(Duration::from_secs(5));
}

fn write_silence<T: Sample>(data: &mut [T], _: &cpal::OutputCallbackInfo) {
    for sample in data.iter_mut() {
        *sample = Sample::from(&0.0);
    }
}