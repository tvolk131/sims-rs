#!/bin/sh
# usage: build_runtime.sh <installed guest ext4> <out dir>
# Writes the kernel and initramfs that start_vm.sh boots (<out dir>/vmlinuz,
# <out dir>/initramfs.cpio.gz) and runtime.txt (kernel version, modules,
# digests), all taken from the installed guest disk: the kernel is the disk's
# /boot/vmlinuz-<version>, and the initramfs holds the disk's static busybox,
# runtime/init and the modules it loads. The disk is mounted read-only and
# never written. The same disk gives the same files, byte for byte.
set -eu
DISK=$1 OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
test -f "$DISK"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)
DISK=$(cd "$(dirname "$DISK")" && pwd)/$(basename "$DISK")
for f in vmlinuz initramfs.cpio.gz runtime.txt; do
    test ! -e "$OUT/$f" || { echo "$OUT/$f exists" >&2; exit 1; }
done
docker run --rm --network none --platform linux/arm64 ${KERNEL_VERSION:+-e KERNEL_VERSION="$KERNEL_VERSION"} \
 --mount type=bind,src="$DISK",dst=/disk.ext4,readonly \
 --mount type=bind,src="$HERE/runtime",dst=/h,readonly \
 --mount type=bind,src="$OUT",dst=/out \
 sims-harness-qemu:bookworm sh /h/build_in_image.sh
