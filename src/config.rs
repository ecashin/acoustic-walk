use clap::{App, Arg, SubCommand};
use std::collections::HashSet;
use std::io::prelude::*;
use std::str::FromStr;
use std::{fs, io, path};

#[derive(Clone)]
pub struct PlayConfig {
    pub excluded_wavs: HashSet<std::path::PathBuf>,
    pub dirs: Vec<String>,
    pub cap_ms: Option<u32>,
    pub grain_ms: Option<u32>,
    pub use_jack: bool,
}

#[derive(Clone)]
pub struct RingbufConfig {
    pub trigfile: path::PathBuf,
    pub n_entries: usize,
}

#[derive(Clone)]
pub enum Config {
    Play(PlayConfig),
    Buf(RingbufConfig),
    Cpal,
}

pub fn make_config() -> Config {
    let matches = App::new("acouwalk")
        .author("Ed.Cashin@acm.org")
        .about("stereo granular audio streamer")
        .subcommand(SubCommand::with_name("cpal"))
        .subcommand(
            SubCommand::with_name("play")
                .arg(Arg::from_usage(
                    "-c --len-cap=[INT] 'Cap on WAV length in ms as used for selection'",
                ))
                .arg(Arg::from_usage(
                    "-e --exclude=[FILE] 'Read excluded WAVs from file'",
                ))
                .arg(
                    Arg::with_name("jack")
                        .long("--use-jack")
                        .short("-j")
                        .takes_value(false),
                )
                .arg(Arg::from_usage(
                    "-g --grain-ms=[INT] 'Milliseconds for minimum grain length'",
                ))
                .arg(
                    Arg::with_name("dirs")
                        .required(true)
                        .min_values(1)
                        .help("<WAV-directory>..."),
                ),
        )
        .subcommand(
            SubCommand::with_name("ringbuf")
                .arg(Arg::from_usage(
                    "-t --trigger-file=[FILE] 'Trigger file for output'",
                ))
                .arg(Arg::from_usage(
                    "-n --n-entries=[INT] 'Number of lines in buffer'",
                )),
        )
        .get_matches();

    match matches.subcommand() {
        ("cpal", Some(_)) => Config::Cpal,
        ("ringbuf", Some(matches)) => {
            let trigfile = if let Some(trigfile) = matches.value_of("trigger-file") {
                path::PathBuf::from_str(trigfile).expect("path for trigger file")
            } else {
                path::PathBuf::from_str("acouwalk.show").expect("path for default trigfile")
            };
            let n_entries = if let Some(n) = matches.value_of("n-entries") {
                n.parse::<usize>().unwrap()
            } else {
                crate::ringbuf::DEFAULT_N_ENTRIES
            };
            Config::Buf(RingbufConfig {
                trigfile,
                n_entries,
            })
        }
        ("play", Some(matches)) => {
            let dirs = if let Some(dirs) = matches.values_of("dirs") {
                dirs.map(String::from).collect()
            } else {
                Vec::new()
            };

            let mut excluded_wavs: HashSet<std::path::PathBuf> = HashSet::new();
            if let Some(e) = matches.value_of("exclude") {
                println!("e:{}", e);
                let f = fs::File::open(e).ok().unwrap();
                let reader = io::BufReader::new(f);
                for line in reader.lines() {
                    let line = line.ok().unwrap();
                    println!("excluding {}", line);
                    let path = std::path::Path::new(&line);
                    excluded_wavs.insert(path.to_path_buf());
                }
            }

            let cap_ms = if let Some(c) = matches.value_of("len-cap") {
                Some(c.parse::<u32>().unwrap())
            } else {
                None
            };

            let grain_ms = if let Some(ms) = matches.value_of("grain-ms") {
                Some(ms.parse::<u32>().expect("ill formed grain milliseconds"))
            } else {
                None
            };

            let use_jack = matches.value_of("jack").is_some();
            Config::Play(PlayConfig {
                excluded_wavs,
                dirs,
                cap_ms,
                grain_ms,
                use_jack,
            })
        }
        _ => panic!("unrecognized subcommand"),
    }
}
