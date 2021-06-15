use cpal::traits::{DeviceTrait, HostTrait};

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
    println!("{:?}", supported_config);
}