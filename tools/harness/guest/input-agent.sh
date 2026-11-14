# Guest job (started by input.py through /research/cmd): runs the input
# commands the host drops in /research/input as <n>.cmd, in order, each once,
# writing <n>.out and then <n>.done with the exit code. It polls often, so a
# command runs soon after the host writes it; the generic job runner
# (/sims-init.sh) polls only once a second.
#
# Only one agent runs in a guest: it holds an flock on a file on the guest's
# own disk for its whole life (the kernel drops the lock when it dies or the
# guest restarts), and a second agent exits. Each command is also claimed by
# renaming it to <n>.run, which only one process can do. Programs the agent
# starts get fd 9 closed, so none outlives a killed agent holding its lock.
export DISPLAY=:98
D=${INPUT_DIR:-/research/input}            # INPUT_DIR, INPUT_LOCK: for the host tests
LOCK=${INPUT_LOCK:-/tmp/input-agent.lock}
mkdir -p "$D"
if command -v flock >/dev/null 2>&1; then
    exec 9>"$LOCK"
    flock -n 9 || exit 0
fi
echo $$ > "$D/agent.pid"
while :; do
    for c in "$D"/*.cmd; do
        [ -e "$c" ] || continue
        base=${c%.cmd}
        [ -e "$base.done" ] && continue  # ended (left as .cmd by an older agent)
        mv "$c" "$base.run" 2>/dev/null 9>&- || continue
        /bin/sh "$base.run" > "$base.out" 2>&1 9>&-
        echo "$?" > "$base.done"
    done
    sleep 0.05 9>&-
done
