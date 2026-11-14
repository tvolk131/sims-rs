# Guest job (started once by input.py through /research/cmd): runs the
# input commands the host drops in /research/input as <n>.cmd, in order,
# each once, writing <n>.out and then <n>.done with the exit code. It polls
# often, so a command runs soon after the host writes it; the generic job
# runner (/sims-init.sh) polls only once a second.
export DISPLAY=:98
mkdir -p /research/input
echo $$ > /research/input/agent.pid
while :; do
    for c in /research/input/*.cmd; do
        [ -e "$c" ] || continue
        base=${c%.cmd}
        [ -e "$base.done" ] && continue
        /bin/sh "$c" > "$base.out" 2>&1
        echo "$?" > "$base.done"
    done
    sleep 0.05
done
