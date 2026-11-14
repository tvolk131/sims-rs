# Guest job (started by input.py through /research/cmd): runs the input
# commands the host drops in /research/input as <n>.cmd, in order, each once,
# writing <n>.out and then <n>.done with the exit code. It polls often, so a
# command runs soon after the host writes it; the generic job runner
# (/sims-init.sh) polls only once a second.
#
# Only one agent serves the folder: a new one exits while the agent named in
# agent.pid still runs (the pid file outlives a guest restart, so the pid is
# checked against the process's command line). And each command is claimed
# by renaming it to <n>.run, which only one process can do, so even two
# agents never run a command twice.
export DISPLAY=:98
D=${INPUT_DIR:-/research/input}  # INPUT_DIR: for the host tests
mkdir -p "$D"
old=$(cat "$D/agent.pid" 2>/dev/null)
if [ -n "$old" ] && [ "$old" != "$$" ] && tr '\0' ' ' 2>/dev/null < "/proc/$old/cmdline" | grep -q input-agent; then
    exit 0
fi
echo $$ > "$D/agent.pid"
while :; do
    for c in "$D"/*.cmd; do
        [ -e "$c" ] || continue
        base=${c%.cmd}
        mv "$c" "$base.run" 2>/dev/null || continue
        /bin/sh "$base.run" > "$base.out" 2>&1
        echo "$?" > "$base.done"
    done
    sleep 0.05
done
