#!/bin/sh
# usage: start_vm.sh <guest dir> <iso> <vmlinuz> <initramfs> [container name]
# Starts the x86 guest (QEMU TCG, one CPU, 1 GiB) in a container with no network.
# QEMU's GDB stub (1234) and QMP (4444) listen on the container's loopback only;
# capture.py reaches them through `docker exec`.
set -eu
G=$1 ISO=$2 KERNEL=$3 INITRD=$4 NAME=${5:-sims-harness-vm}
exec docker run --detach --name "$NAME" --platform linux/arm64 --network none --memory 4g --cpus 2 \
 --mount type=bind,src="$G",dst=/g \
 --mount type=bind,src="$ISO",dst=/inputs/game.iso,readonly \
 --mount type=bind,src="$KERNEL",dst=/inputs/vmlinuz,readonly \
 --mount type=bind,src="$INITRD",dst=/inputs/initramfs.cpio.gz,readonly \
 sims-harness-qemu:bookworm qemu-system-i386 -accel tcg,thread=single -cpu qemu32 -m 1024 -smp 1 \
 -kernel /inputs/vmlinuz -initrd /inputs/initramfs.cpio.gz -append 'console=ttyS0 root=/dev/vda rw rdinit=/init quiet' \
 -drive file=/g/working.ext4,format=raw,if=virtio -drive file=/inputs/game.iso,format=raw,if=virtio,readonly=on \
 -virtfs local,path=/g/share,mount_tag=research,security_model=none,id=research \
 -display none -monitor none -serial file:/g/serial.log -no-reboot -nic none \
 -qmp tcp:127.0.0.1:4444,server=on,wait=off -gdb tcp:127.0.0.1:1234
