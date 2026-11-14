#!/bin/sh
# usage: prepare_guest.sh <install dir> <installed guest ext4> <new guest dir>
# Builds a guest directory for start_vm.sh: an APFS clone of the installed
# guest disk (the seed is never written) with /sims-init.sh replaced, and a
# share holding the guest scripts and a copy of the install's UserData folders
# with their hashes, which guest/launch.sh puts in place before the game runs.
set -eu
I=$1 SEED=$2 G=$3
test ! -e "$G"
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$G/share/userdata"
cp -c "$SEED" "$G/working.ext4"
cp "$HERE/guest/sims-init.sh" "$HERE/guest/prepare_disk.sh" "$HERE/guest/launch.sh" "$G/share/"
for d in UserData UserData2 UserData3 UserData4 UserData5 UserData6 UserData7 UserData8; do
    cp -c -R "$I/$d" "$G/share/userdata/"
done
(cd "$G/share/userdata" && find . -type f -print0 | sort -z | xargs -0 shasum -a 256) > "$G/share/userdata-hashes.txt"
docker run --rm --network none -v "$G:/g" sims-harness-qemu:bookworm sh /g/share/prepare_disk.sh
