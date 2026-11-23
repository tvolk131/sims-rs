#!/bin/sh
# usage: build_runtime.sh <installed guest ext4> <new out dir>
# Makes <new out dir> holding the kernel and initramfs that start_vm.sh boots
# (vmlinuz, initramfs.cpio.gz) and runtime.txt (kernel version, modules,
# digests), all taken from the installed guest disk: the kernel is the disk's
# /boot/vmlinuz-<version>, and the initramfs holds the disk's static busybox,
# runtime/init and the modules it loads. The same disk gives the same files,
# byte for byte.
#
# The work is done in a staging directory next to <new out dir>, on an APFS
# clone of the disk with its journal replayed (as prepare_guest.sh does), so
# the disk itself is never mounted or written. Only the staging directory is
# mounted in the container. When the build has succeeded, the finished
# directory is renamed to <new out dir> in one step that refuses to replace
# anything there, so <new out dir> either appears whole or not at all.
set -eu
DISK=$1 OUT=$2
HERE=$(cd "$(dirname "$0")" && pwd)
test -f "$DISK"
if [ -e "$OUT" ] || [ -L "$OUT" ]; then echo "$OUT exists" >&2; exit 1; fi
PARENT=$(cd "$(dirname "$OUT")" && pwd -P)
OUT=$PARENT/$(basename "$OUT")
ST=$(mktemp -d "$PARENT/.runtime-build.XXXXXX")
trap 'chflags -R nouchg "$ST" 2>/dev/null; rm -rf "$ST"' EXIT
case $ST$HERE in *,*) echo "paths with commas are not supported (docker --mount)" >&2; exit 1 ;; esac
cp -c "$DISK" "$ST/disk.ext4"  # a free clone on the disk's volume, else a full copy
chflags nouchg "$ST/disk.ext4"  # the clone keeps a locked seed's flag
chmod u+w "$ST/disk.ext4"
docker run --rm --network none --platform linux/arm64 ${KERNEL_VERSION:+-e KERNEL_VERSION="$KERNEL_VERSION"} \
 --mount type=bind,src="$HERE/runtime",dst=/h,readonly \
 --mount type=bind,src="$ST",dst=/s \
 sims-harness-qemu:bookworm sh /h/build_in_image.sh
for f in vmlinuz initramfs.cpio.gz runtime.txt; do test -s "$ST/runtime/$f"; done
# rename(2) would replace an empty directory, so ask for no replacement:
# renamex_np(RENAME_EXCL) on macOS, renameat2(RENAME_NOREPLACE) on Linux.
python3 -I - "$ST/runtime" "$OUT" <<'EOF'
import ctypes, os, sys
src, dst = (os.fsencode(a) for a in sys.argv[1:])
libc = ctypes.CDLL(None, use_errno=True)
if sys.platform == 'darwin':
    rc = libc.renamex_np(src, dst, ctypes.c_uint(0x4))
else:
    rc = libc.renameat2(-100, src, -100, dst, ctypes.c_uint(0x1))
if rc != 0:
    e = ctypes.get_errno()
    sys.exit(f'cannot publish {sys.argv[2]}: {os.strerror(e)}')
EOF
echo "published $OUT"
