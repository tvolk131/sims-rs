#!/bin/sh
# usage: answer.sh <guest dir> <capture dir> <label> X Y
# Presses the left button at (X,Y) in the game window and logs it to
# <capture dir>/input-log.txt. The game takes a press only after the pointer
# has rested and moved over the button, and handles it late (about 20 s under
# TCG), so wait for the dialog_read event before pressing again.
set -eu
G=$1 C=$2 LABEL=$3 X=$4 Y=$5
W=${SIMS_WINDOW:-0xe00013}
CMD="xdotool windowfocus --sync $W sleep 3 mousemove --window $W $X $Y sleep 4 mousemove --window $W $((X + 2)) $Y sleep 2 mousedown 1 sleep 3 mouseup 1"
{ date -u +%FT%TZ; echo "$LABEL: $CMD"; } >> "$C/input-log.txt"
exec python3 "$(dirname "$0")/guest.py" "$G/share" run "$CMD"
