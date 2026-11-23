#!/bin/sh
# Run by build_runtime.sh in the harness image, with the installed guest disk
# at /disk.ext4 (read-only), this directory at /h and the output at /out.
# Copies the guest's kernel out of /boot and builds an initramfs from the
# guest's own busybox and modules, so the runtime matches the disk it boots.
# debugfs opens the disk read-only (no -w).
set -eu
DISK=/disk.ext4
MODULES="virtio_pci virtio_blk crc32c_generic ext4"
K=${KERNEL_VERSION:-}
if [ -z "$K" ]; then
    # `ls -p` prints /inode/mode/uid/gid/name/size/ per entry.
    K=$(debugfs -R 'ls -p /lib/modules' "$DISK" 2>/dev/null | awk -F/ '$6 != "." && $6 != ".." && $6 != "" { print $6 }')
    [ -n "$K" ] && [ "$(printf '%s\n' "$K" | wc -l)" -eq 1 ] || { echo "not one kernel in /lib/modules: $K" >&2; exit 1; }
fi
echo "kernel $K"
S=$(mktemp -d /tmp/runtime.XXXXXX)
trap 'rm -rf "$S"' EXIT
get() { # get <path on disk> <local file>
    debugfs -R "dump $1 $2" "$DISK" 2>/dev/null
    test -s "$2" || { echo "cannot read $1 from the disk" >&2; exit 1; }
}

get "/boot/vmlinuz-$K" "$S/vmlinuz"

R=$S/root
M=lib/modules/$K
mkdir -p "$R/bin" "$R/dev" "$R/proc" "$R/sys" "$R/newroot" "$R/$M"
get /usr/bin/busybox "$R/bin/busybox"
chmod 0755 "$R/bin/busybox"
# The initramfs has no libc, so busybox must be static: no PT_INTERP header.
perl -e '
    open(my $f, "<:raw", $ARGV[0]) or die; local $/; my $b = <$f>;
    die "not a 32-bit ELF\n" unless substr($b, 0, 5) eq "\x7fELF\x01";
    my ($phoff) = unpack("V", substr($b, 28, 4));
    my ($phentsize, $phnum) = unpack("vv", substr($b, 42, 4));
    for my $i (0 .. $phnum - 1) {
        die "busybox is dynamically linked\n"
            if unpack("V", substr($b, $phoff + $i * $phentsize, 4)) == 3;
    }' "$R/bin/busybox"
for applet in sh mount mkdir modprobe sleep switch_root uname; do
    ln -s busybox "$R/bin/$applet"
done
cp /h/init "$R/init"
chmod 0755 "$R/init"

# The modules /init loads and everything they depend on, from modules.dep,
# with a modules.dep that names only those (busybox modprobe reads it).
get "/$M/modules.dep" "$S/modules.dep"
awk -v want="$MODULES" '
    { sub(/:$/, "", $1); key = $1; line[key] = $0; n = split(key, p, "/"); base[p[n]] = key }
    END {
        nw = split(want, w, " ")
        for (i = 1; i <= nw; i++) {
            k = base[w[i] ".ko"]
            if (k == "") { print "no module " w[i] > "/dev/stderr"; exit 1 }
            stack[++sp] = k
        }
        while (sp > 0) {
            k = stack[sp--]
            if (k in seen) continue
            seen[k] = 1
            if (!(k in line)) { print "no modules.dep line for " k > "/dev/stderr"; exit 1 }
            print line[k]
            nd = split(line[k], d, " ")
            for (j = 2; j <= nd; j++) stack[++sp] = d[j]
        }
    }' "$S/modules.dep" | sed 's/^\([^ ]*\) /\1: /; s/^\([^ :]*\)$/\1:/' | sort > "$R/$M/modules.dep"
cut -d: -f1 "$R/$M/modules.dep" | while read -r ko; do
    mkdir -p "$R/$M/$(dirname "$ko")"
    get "/$M/$ko" "$R/$M/$ko"
done
echo "modules: $(cut -d: -f1 "$R/$M/modules.dep" | tr '\n' ' ')"

perl /h/newc.pl "$R" dev/console:c:5:1 | gzip -n -9 > "$S/initramfs.cpio.gz"

cp "$S/vmlinuz" /out/vmlinuz
cp "$S/initramfs.cpio.gz" /out/initramfs.cpio.gz
{
    echo "kernel $K"
    echo "modules $(cut -d: -f1 "$R/$M/modules.dep" | tr '\n' ' ')"
    (cd /out && sha256sum vmlinuz initramfs.cpio.gz)
    (cd "$R" && sha256sum init bin/busybox)
} > /out/runtime.txt
cat /out/runtime.txt
