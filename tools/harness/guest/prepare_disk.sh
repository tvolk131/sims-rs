#!/bin/sh
# Run in the harness image with the guest directory at /g:
#   docker run --rm --network none -v <guest dir>:/g sims-harness-qemu:bookworm sh /g/share/prepare_disk.sh
# Replaces /sims-init.sh on the guest disk (a clone, never the seed) with ours.
set -eu
e2fsck -fy /g/working.ext4 || test $? -le 1
debugfs -w -R "rm /sims-init.sh" /g/working.ext4
debugfs -w -R "write /g/share/sims-init.sh /sims-init.sh" /g/working.ext4
debugfs -w -R "set_inode_field /sims-init.sh mode 0100755" /g/working.ext4
debugfs -R "cat /sims-init.sh" /g/working.ext4 | cmp - /g/share/sims-init.sh
e2fsck -fn /g/working.ext4
echo PREPARED
