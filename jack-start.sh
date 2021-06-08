#! /bin/sh

# qjackctl created a `~/.jackdrc` with contents below:
#     /usr/bin/jackd -dalsa -dhw:0 -r44100 -p2048 -n2

# https://wiki.archlinux.org/title/JACK_Audio_Connection_Kit

# See current settings with `jack_control dp`.

jack_control stop
jack_control ds alsa
for role in device capture playback; do
    jack_control dps $role hw:0
done
jack_control dps nperiods 2
jack_control dps period 2048
jack_control dps rate 44100
jack_control start