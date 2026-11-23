#!/usr/bin/perl
# usage: newc.pl <dir> [path:c:major:minor ...] > archive.cpio
# Writes the tree under <dir> as a cpio archive in the kernel's initramfs
# ("newc") format, deterministically: names sorted, owner root, mtime 0,
# inode numbers in order, modes reduced to 0755/0644 (0777 for links). Each
# extra argument adds a character device node (mode 0600) that a build user
# could not create in the tree. Only perl-base is used, since the harness
# image has nothing more.
use strict;
use warnings;

my ($root, @nodes) = @ARGV;
die "usage: newc.pl <dir> [path:c:major:minor ...]\n" unless defined $root;
chdir $root or die "chdir $root: $!\n";

my %entry;
sub walk {
    my ($rel) = @_;
    opendir(my $dh, $rel eq '' ? '.' : $rel) or die "opendir $rel: $!\n";
    for my $name (sort grep { $_ ne '.' && $_ ne '..' } readdir $dh) {
        my $path = $rel eq '' ? $name : "$rel/$name";
        my @st = lstat $path or die "lstat $path: $!\n";
        if (-l _) {
            my $target = readlink $path;
            defined $target or die "readlink $path: $!\n";
            $entry{$path} = [0120777, $target, 0];
        } elsif (-d _) {
            $entry{$path} = [040755, '', 0];
            walk($path);
        } elsif (-f _) {
            open(my $fh, '<:raw', $path) or die "open $path: $!\n";
            local $/;
            my $data = <$fh> // '';
            close $fh;
            $entry{$path} = [($st[2] & 0111) ? 0100755 : 0100644, $data, 0];
        } else {
            die "unsupported file type: $path\n";
        }
    }
    closedir $dh;
}
walk('');
for my $node (@nodes) {
    my ($path, $kind, $major, $minor) = split /:/, $node;
    die "bad node $node\n" unless defined $minor && $kind eq 'c';
    die "node $path exists in the tree\n" if exists $entry{$path};
    $entry{$path} = [020600, '', ($major << 8) | $minor, $major, $minor];
}

my $ino = 0;
sub record {
    my ($name, $mode, $data, $rmaj, $rmin) = @_;
    my $nlink = ($mode & 0170000) == 040000 ? 2 : 1;
    my $hdr = sprintf('070701' . ('%08X' x 13),
        ++$ino, $mode, 0, 0, $nlink, 0, length $data, 0, 0, $rmaj, $rmin,
        length($name) + 1, 0);
    my $out = $hdr . $name . "\0";
    $out .= "\0" x ((4 - length($out) % 4) % 4);
    $out .= $data;
    $out .= "\0" x ((4 - length($data) % 4) % 4);
    print $out;
}
binmode STDOUT;
for my $path (sort keys %entry) {
    my ($mode, $data, undef, $rmaj, $rmin) = @{$entry{$path}};
    record($path, $mode, $data, $rmaj // 0, $rmin // 0);
}
record('TRAILER!!!', 0, '', 0, 0);
