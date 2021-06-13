#! /bin/sh

d=`dirname "$0"`
set -xe
cd "$d"
sh jack-start.sh
sleep 2

cargo build
rm -f acourun.pipe
mkfifo acourun.pipe

target/debug/acoustic-walk \
    play \
    --exclude excluded.txt \
    -c 70000 \
    ~/samples-ecashin-orig/Zoom-H5 > acourun.pipe 2>&1 &
echo $! > acouplay.pid

sleep 1

target/debug/acoustic-walk \
    ringbuf \
    --trigger-file acourun.show \
    --n-entries 1024 \
    < acourun.pipe &
echo $! > acoubuf.pid

sleep 2

# Use `jack_lsp -c` to find out names.
jack_connect acouwalk:acouwalk_out_L system:playback_1
jack_connect acouwalk:acouwalk_out_R system:playback_2
