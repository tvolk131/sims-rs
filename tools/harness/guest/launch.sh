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
# The build the harness was made for (Sims.exe 1.91, Complete Collection, and
# its version.dll). Another build stops the launch here (set -e). The verified
# digests go to the share, and capture.py records them.
rm -f /research/game-identity.txt
cat > /tmp/game-identity.txt <<'END'
32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58  Sims.exe
2e9ca4bb2c650dafe2556176aeeef2fb5c4d733e398b45e467fae10c3195035b  version.dll
END
sha256sum -c /tmp/game-identity.txt
cp /tmp/game-identity.txt /research/game-identity.txt
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
