#!/bin/sh
# usage: build_runtime.sh <installed guest ext4> <out dir>
# Writes the kernel and initramfs that start_vm.sh boots (<out dir>/vmlinuz,
# <out dir>/initramfs.cpio.gz) and runtime.txt (kernel version, modules,
# digests), all taken from the installed guest disk: the kernel is the disk's
# /boot/vmlinuz-<version>, and the initramfs holds the disk's static busybox,
# runtime/init and the modules it loads. The work is done on an APFS clone of
# the disk (its journal replayed, as prepare_guest.sh does), so the disk
# itself is never mounted or written. The same disk gives the same files,
# byte for byte.
set -eu
DISK=$1 OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
test -f "$DISK"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd -P)
case $OUT$HERE in *,*) echo "paths with commas are not supported (docker --mount)" >&2; exit 1 ;; esac
for f in vmlinuz initramfs.cpio.gz runtime.txt .build; do
    if [ -e "$OUT/$f" ] || [ -L "$OUT/$f" ]; then echo "$OUT/$f exists" >&2; exit 1; fi
done
mkdir "$OUT/.build"
trap 'chflags -R nouchg "$OUT/.build" 2>/dev/null; rm -rf "$OUT/.build"' EXIT
cp -c "$DISK" "$OUT/.build/disk.ext4"  # a free clone on the disk's volume, else a full copy
chflags nouchg "$OUT/.build/disk.ext4"  # the clone keeps a locked seed's flag
chmod u+w "$OUT/.build/disk.ext4"
docker run --rm --network none --platform linux/arm64 ${KERNEL_VERSION:+-e KERNEL_VERSION="$KERNEL_VERSION"} \
 --mount type=bind,src="$HERE/runtime",dst=/h,readonly \
 --mount type=bind,src="$OUT",dst=/out \
 sims-harness-qemu:bookworm sh /h/build_in_image.sh
