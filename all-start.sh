#! /bin/sh

d=`dirname "$0"`
set -xe
cd "$d"
sh jack-start.sh
sleep 2

cargo run -- -c 70000 ~/samples-ecashin-orig/Zoom-H5 > acourun.log 2>&1 &
echo $! > acourun.pid
sleep 2

# Use `jack_lsp -c` to find out names.
jack_connect acouwalk:acouwalk_out_L system:playback_1
jack_connect acouwalk:acouwalk_out_R system:playback_2
