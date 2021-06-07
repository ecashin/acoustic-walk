use clap::App;
use std::io::prelude::*;
use std::{fs, io};

#[derive(Clone)]
pub struct Config {
    pub excluded_wavs: Vec<std::path::PathBuf>,
    pub dirs: Vec<String>,
    pub cap_ms: Option<u32>,
}

pub fn make_config() -> Config {
    let matches = App::new("acouwalk")
        .author("Ed.Cashin@acm.org")
        .about("stereo granular streamer for JACK")
        .arg(clap::Arg::from_usage(
            "-e --exclude=[FILE] 'Read excluded WAVs from file'",
        ))
        .arg(clap::Arg::from_usage(
            "-c --len-cap=[INT] 'Cap on WAV length in ms as used for selection'",
        ))
        .arg(
            clap::Arg::with_name("dirs")
                .required(true)
                .min_values(1)
                .help("<WAV-directory>..."),
        )
        .get_matches();

    let dirs = if let Some(dirs) = matches.values_of("dirs") {
        dirs.map(|e| String::from(e)).collect()
    } else {
        Vec::new()
    };

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

    let cap_ms = if let Some(c) = matches.value_of("len-cap") {
        Some(c.parse::<u32>().unwrap())
    } else {
        None
    };

    Config {
        excluded_wavs,
        dirs,
        cap_ms,
    }
}
