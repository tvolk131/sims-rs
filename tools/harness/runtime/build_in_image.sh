#!/bin/sh
# Run by build_runtime.sh in the harness image, with a throwaway clone of the
# installed guest disk at /out/.build/disk.ext4, this directory at /h and the
# output directory at /out. Replays the clone's journal (as prepare_disk.sh
# does for a guest), then copies the guest's kernel out of /boot and builds an
# initramfs from the guest's own busybox and modules, so the runtime matches
# the disk a prepared guest boots. Every step's failure stops the build, and
# the outputs appear only once all of them are written.
set -eu
DISK=/out/.build/disk.ext4
MODULES="virtio_pci virtio_blk crc32c_generic ext4"
e2fsck -fy "$DISK" > /dev/null || test $? -le 1
K=${KERNEL_VERSION:-}
if [ -z "$K" ]; then
    # `ls -p` prints /inode/mode/uid/gid/name/size/ per entry.
    debugfs -R 'ls -p /lib/modules' "$DISK" 2>/dev/null > /out/.build/kernels
    K=$(awk -F/ '$6 != "." && $6 != ".." && $6 != "" { print $6 }' /out/.build/kernels)
    [ -n "$K" ] && [ "$(printf '%s\n' "$K" | wc -l)" -eq 1 ] || { echo "not one kernel in /lib/modules: $K" >&2; exit 1; }
fi
echo "kernel $K"
S=$(mktemp -d /tmp/runtime.XXXXXX)
trap 'rm -rf "$S"' EXIT

# resolve <absolute path on disk>: the path of the regular file it names,
# following symbolic links (debugfs itself does not follow a last one).
resolve() {
    p=$1 n=0
    while :; do
        debugfs -R "stat \"$p\"" "$DISK" 2>/dev/null > "$S/stat" || true
        type=$(sed -n 's/.*Type: \([a-z]*\).*/\1/p' "$S/stat" | head -n 1)
        case $type in
        regular) echo "$p"; return 0 ;;
        symlink)
            n=$((n + 1)); [ "$n" -le 8 ] || { echo "too many links at $1" >&2; return 1; }
            t=$(sed -n 's/^Fast link dest: "\(.*\)"$/\1/p' "$S/stat")
            [ -n "$t" ] || t=$(debugfs -R "cat \"$p\"" "$DISK" 2>/dev/null)
            case $t in /*) p=$t ;; *) p=$(dirname "$p")/$t ;; esac ;;
        *) echo "$1 is not a regular file on the disk (${type:-missing})" >&2; return 1 ;;
        esac
    done
}
# get <absolute path on disk> <local file>: copies it and checks its size,
# which must not be zero.
get() {
    src=$(resolve "$1")
    rm -f "$2"
    debugfs -R "dump \"$src\" \"$2\"" "$DISK" 2>/dev/null
    want=$(sed -n 's/.*Size: \([0-9]*\).*/\1/p' "$S/stat" | head -n 1)
    have=$(wc -c < "$2" 2>/dev/null || echo none)
    [ -n "$want" ] && [ "$want" -gt 0 ] && [ "$have" = "$want" ] || { echo "cannot read $1 from the disk ($have of $want bytes)" >&2; exit 1; }
}

get "/boot/vmlinuz-$K" "$S/vmlinuz"

R=$S/root
M=lib/modules/$K
mkdir -p "$R/bin" "$R/dev" "$R/proc" "$R/sys" "$R/newroot" "$R/$M"
get /usr/bin/busybox "$R/bin/busybox"
chmod 0755 "$R/bin/busybox"
# The initramfs has no libc, so busybox must be a static 32-bit x86 ELF
# executable: no PT_INTERP header.
perl -e '
    open(my $f, "<:raw", $ARGV[0]) or die; local $/; my $b = <$f>;
    die "busybox: not a 32-bit little-endian ELF\n"
        unless length $b >= 52 && substr($b, 0, 6) eq "\x7fELF\x01\x01";
    my ($type, $machine) = unpack("vv", substr($b, 16, 4));
    die "busybox: not an i386 executable\n" unless $machine == 3 && ($type == 2 || $type == 3);
    my ($phoff) = unpack("V", substr($b, 28, 4));
    my ($phentsize, $phnum) = unpack("vv", substr($b, 42, 4));
    die "busybox: bad program headers\n"
        if $phnum == 0 || $phentsize != 32 || $phoff + $phnum * $phentsize > length $b;
    my $loads = 0;
    for my $i (0 .. $phnum - 1) {
        my ($ptype, $off, undef, undef, $filesz) = unpack("V5", substr($b, $phoff + $i * 32, 20));
        die "busybox is dynamically linked\n" if $ptype == 3;
        next unless $ptype == 1;
        $loads++;
        die "busybox: a load segment lies past the end of the file\n" if $off + $filesz > length $b;
    }
    die "busybox: no load segment\n" unless $loads;' "$R/bin/busybox"
for applet in sh mount mkdir modprobe sleep switch_root uname; do
    ln -s busybox "$R/bin/$applet"
done
cp /h/init "$R/init"
chmod 0755 "$R/init"

# The modules /init loads and everything they depend on, from modules.dep,
# with a modules.dep that names only those (busybox modprobe reads it).
# Names match as modprobe matches them: '-' and '_' alike, the first row
# for a name (or a path) wins, and the rows keep their order. The kernel's modules are not compressed; a compressed
# one stops the build.
get "/$M/modules.dep" "$S/modules.dep"
awk -v want="$MODULES" '
    function name(path,   n, p, b) {
        n = split(path, p, "/"); b = p[n]
        if (b !~ /\.ko$/) { print "unsupported module file " path > "/dev/stderr"; bad = 1; exit 1 }
        sub(/\.ko$/, "", b); gsub(/-/, "_", b); return b
    }
    {
        key = $1; sub(/:$/, "", key)
        deps = ""; for (j = 2; j <= NF; j++) deps = deps " " $j
        if (key in dep) next
        dep[key] = deps; order[++rows] = key
        b = name(key); if (!(b in path)) path[b] = key
    }
    END {
        if (bad) exit 1
        nw = split(want, w, " ")
        for (i = 1; i <= nw; i++) {
            b = w[i]; gsub(/-/, "_", b)
            if (!(b in path)) { print "no module " w[i] > "/dev/stderr"; exit 1 }
            stack[++sp] = path[b]
        }
        while (sp > 0) {
            k = stack[sp--]
            if (k in seen) continue
            seen[k] = 1
            if (!(k in dep)) { print "no modules.dep row for " k > "/dev/stderr"; exit 1 }
            name(k)
            nd = split(dep[k], d, " ")
            for (j = 1; j <= nd; j++) stack[++sp] = d[j]
        }
        # In modules.dep order, so modprobe meets the rows as it would there.
        for (i = 1; i <= rows; i++) if (order[i] in seen) print order[i] ":" dep[order[i]]
    }' "$S/modules.dep" > "$R/$M/modules.dep"
cut -d: -f1 "$R/$M/modules.dep" > "$S/kos"
while read -r ko; do
    mkdir -p "$R/$M/$(dirname "$ko")"
    get "/$M/$ko" "$R/$M/$ko"
done < "$S/kos"
echo "modules: $(tr '\n' ' ' < "$S/kos")"

perl /h/newc.pl "$R" dev/console:c:5:1 > "$S/initramfs.cpio"
gzip -n -9 "$S/initramfs.cpio"

{
    echo "kernel $K"
    echo "modules $(tr '\n' ' ' < "$S/kos")"
    (cd "$S" && sha256sum vmlinuz initramfs.cpio.gz)
    (cd "$R" && sha256sum init bin/busybox)
} > "$S/runtime.txt"
for f in vmlinuz initramfs.cpio.gz runtime.txt; do
    cp "$S/$f" "/out/.build/$f"
done
# build_runtime.sh checked that none of the three exists, so a failed
# publication removes the ones already moved.
for f in vmlinuz initramfs.cpio.gz runtime.txt; do
    mv "/out/.build/$f" "/out/$f" || { (cd /out && rm -f vmlinuz initramfs.cpio.gz runtime.txt); exit 1; }
done
cat /out/runtime.txt
