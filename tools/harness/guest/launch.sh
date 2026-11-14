# Guest job: put the install's UserData in place, start X and the game.
set -eu
export WINEPREFIX=/root/installer-prefix WINEARCH=win32 DISPLAY=:98 LANG=C.UTF-8 WINEDEBUG=-all
export WINEDLLOVERRIDES='version=n,b;winemenubuilder.exe,mscoree,mshtml='
GAME="$WINEPREFIX/drive_c/Program Files/Maxis/The Sims"
/bin/busybox modprobe isofs
mkdir -p /media/cdrom
mountpoint -q /media/cdrom || mount -t iso9660 -o ro /dev/vdb /media/cdrom
ls -l "$WINEPREFIX/dosdevices/"
cd "$GAME"
sha256sum Sims.exe version.dll
for d in UserData UserData2 UserData3 UserData4 UserData5 UserData6 UserData7 UserData8; do
    rm -rf "$d"
    cp -a "/research/userdata/$d" .
done
sha256sum -c --quiet /research/userdata-hashes.txt
echo "userdata files: $(find UserData UserData2 UserData3 UserData4 UserData5 UserData6 UserData7 UserData8 -type f | wc -l)"
sync
rm -f /tmp/.X98-lock /tmp/.X11-unix/X98
Xvfb :98 -screen 0 1024x768x16 -nolisten tcp > /research/xvfb-98.log 2>&1 &
sleep 5
cd /research
wine start /d 'C:\Program Files\Maxis\The Sims' /wait 'C:\Program Files\Maxis\The Sims\Sims.exe' -w -nosound > /research/wine-launch.log 2>&1 &
echo "wine pid $!"
