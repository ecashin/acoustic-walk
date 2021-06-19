#! /bin/sh

d=`dirname "$0"`
set -xe
cd "$d"

case "$1" in
    "stop")
        test -r acouplay.pid && kill `cat acouplay.pid`
        test -r acoubuf.pid && kill `cat acoubuf.pid`
        exit
    ;;
    "")
    ;;
    *)
        exec 1>&2
        echo Usage: `basename "$0"` "[stop]"
        exit 1
    ;;
esac

cargo build
rm -f acourun.pipe
mkfifo acourun.pipe

# (Edit the script if you don't have this file.)
test -r excluded.txt

target/debug/acoustic-walk \
    play \
    --exclude excluded.txt \
    -c 70000 \
    ~/samples-ecashin-orig/Zoom-H5 > acourun.pipe 2>&1 &
echo $! > acouplay.pid

target/debug/acoustic-walk \
    ringbuf \
    --trigger-file acourun.show \
    --n-entries 1024 \
    < acourun.pipe &
echo $! > acoubuf.pid
