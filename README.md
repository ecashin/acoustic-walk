# Acoustic Walk

## Context

This is hastily made software
targeted toward an art opening in Athens, Georgia,
and to be used under carefully controlled circumstances.

## Usage

To play shuffled audio, specify a directory
where a recursive file-tree walk will find stereo 16-bit WAV files,
as in the example below.

    cargo run play ~/samples-ecashin-orig/Zoom-H5

The application is designed to run until stopped
with control-c.

Unsupported WAV files will result in undefined behavior.
Only uncompressed 16-bit stereo WAV files are supported.

## WAV Exclusion

Multiple WAVs may be listed by absolute path
in an exclude file specified via an option.
Use `--` to separate acoustic-walk arguments
from `cargo` arguments.

    cargo run -- play --exclude excluded.txt \
        ~/samples-ecashin-orig/Zoom-H5

## Music Non-stop

This application is designed to run indefinitely
and lacks fully fledged shut-down mechanics by design.

It can be manually stopped by control-c
or by `kill`.

## JACK Support

The `play` subcommand offers a `--use-jack` option
that causes acoustic-walk to use JACK for audio.

By default it uses `cpal`, the cross-platform audio library.

The applications sends stereo audio
to the JACK audio system.
There's a lot of file opening and closing,
and data copying going on right now.
To avoid audible buzzes from audio underruns,
I am configuring JACK
to use a relatively low sample rate of 44100,
and a relatively large buffer size of 2048.

The sample rate of 44100 allows acoustic-walk
to skip the use of `samplerate::convert`
when the WAV files also have a 44100 sample rate.
The opportunity was exposed by using `flamegraph`
and opening the resulting SVG file in a web browser.

    cargo install flamegraph
    flamegraph target/debug/acoustic-walk play ~/samples-ecashin-orig/Zoom-H5

## Example Scripts

Scripts that work for me could serve as useful examples
for you to build upon.

    sh all-start.sh

The above command runs acoustic-walk in both "play" and "ring buffer" modes.
It creates some files in the current working directory.
To see output from the player, I trigger the ringbuf's output
by creating a file named `acourun.show`.

I can kill the acoustic-walk processes as shown below
or via `sh acouwalk.sh stop`.

    for i in *.pid; do kill `cat "$i"`; done
