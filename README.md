# Acoustic Walk

To play shuffled audio, specify a directory
where a recursive file-tree walk will find stereo WAV files,
as in the example below.

    cargo run ~/samples-ecashin-orig/Zoom-H5

The application is designed to run until stopped
with control-c.

## JACK

The applications sends stereo audio
to the JACK audio system.
There's a lot of file opening and closing,
and data copying going on right now.
To avoid audible buzzes from audio underruns,
I am configuring JACK
to use a relatively low sample rate of 44100,
and a relatively large buffer size of 2048.
