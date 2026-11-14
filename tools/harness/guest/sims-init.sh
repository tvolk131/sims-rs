#!/bin/sh
# PID 1 of the x86 guest (installed by prepare_disk.sh as /sims-init.sh; the
# initramfs switch_roots into it). No network. It mounts the host share at
# /research and runs every /research/cmd/<name>.sh it finds, once, writing
# <name>.out and then <name>.done, so the host drives the guest through files.
mkdir -p /dev/pts /dev/shm /research
mount -t devpts devpts /dev/pts
mount -t tmpfs tmpfs /dev/shm
/bin/busybox modprobe 9pnet_virtio
/bin/busybox modprobe 9p
mount -t 9p -o trans=virtio,version=9p2000.L research /research
echo SIMS_HARNESS_GUEST_READY > /dev/ttyS0
mkdir -p /research/cmd
while :; do
    for job in /research/cmd/*.sh; do
        [ -e "$job" ] || continue
        base=${job%.sh}
        [ -e "$base.done" ] && continue
        [ -e "$base.started" ] && continue
        : > "$base.started"
        ( /bin/sh "$job" > "$base.out" 2>&1; echo "$?" > "$base.done" ) &
    done
    /bin/busybox sleep 1
done
