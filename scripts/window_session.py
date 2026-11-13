"""Run a bounded native-window input session; all game observations stay in cache."""
import argparse
import ctypes
import hashlib
import json
import math
from pathlib import Path
import subprocess
import time
from window_presentation import summarize_presentations
from window_display import summarize_displays


REPOSITORY = Path(__file__).resolve().parent.parent
HOST_CLOCK = "mach_absolute_time ticks converted with mach_timebase_info to nanoseconds"
MAC_KEYS = {36:"Enter",49:"Space",53:"Escape",123:"ArrowLeft",124:"ArrowRight",
            125:"ArrowDown",126:"ArrowUp"}
OPERATORS = {
    "eq": lambda actual, expected: actual == expected,
    "ne": lambda actual, expected: actual != expected,
    "gt": lambda actual, expected: actual > expected,
    "ge": lambda actual, expected: actual >= expected,
    "lt": lambda actual, expected: actual < expected,
    "contains": lambda actual, expected: expected in actual,
    "not_contains": lambda actual, expected: expected not in actual,
    "one_of": lambda actual, expected: actual in expected,
}


def lookup(value, path):
    for component in path.split("."):
        value = value[int(component)] if isinstance(value, list) else value[component]
    return value


def matches(state, conditions, remembered):
    for condition in conditions:
        try:
            if condition["op"] == "any_match":
                records = lookup(state, condition["path"])
                if not isinstance(records, list) or not any(
                        isinstance(record, dict) and matches(record, condition["value"], remembered)
                        for record in records):
                    return False
                continue
            expected = (lookup(remembered, condition["ref"])
                        if "ref" in condition else condition.get("value"))
            if not OPERATORS[condition["op"]](lookup(state, condition["path"]), expected):
                return False
        except (KeyError, IndexError, TypeError, ValueError):
            return False
    return True


def distribution(values):
    samples = sorted(value for value in values if value is not None)
    if not samples:
        return {"samples": 0}
    percentile = lambda fraction: samples[max(0, math.ceil(len(samples) * fraction) - 1)]
    return {"samples": len(samples), "p50": percentile(.5), "p95": percentile(.95),
            "p99": percentile(.99), "worst": samples[-1]}


def host_time_ns():
    class Timebase(ctypes.Structure):
        _fields_ = [("numer", ctypes.c_uint32), ("denom", ctypes.c_uint32)]
    system = ctypes.CDLL("/usr/lib/libSystem.B.dylib")
    timebase_info = system.mach_timebase_info
    timebase_info.argtypes = [ctypes.POINTER(Timebase)]
    timebase_info.restype = ctypes.c_int
    absolute_time = system.mach_absolute_time
    absolute_time.argtypes = []
    absolute_time.restype = ctypes.c_uint64
    timebase = Timebase()
    if timebase_info(ctypes.byref(timebase)) != 0 or not timebase.numer or not timebase.denom:
        raise RuntimeError("Cannot establish the native Mach host-clock timebase")
    return absolute_time() * timebase.numer // timebase.denom


def summarize_spans(records):
    active, seen, phases, errors = {}, set(), {}, []
    available, ended = False, False
    for index, record in enumerate(records):
        if record["kind"] == "session_end":
            ended = True
            if active:
                errors.append(f"Session ended with incomplete spans at record {index}")
            continue
        if record["kind"] not in {"span_begin", "span_end"}:
            continue
        available = True
        if ended:
            errors.append(f"Span event after session end at record {index}")
        data = record["data"]
        name, identifier = data.get("name"), data.get("span_id")
        if not isinstance(name, str) or not name or type(identifier) is not int or identifier <= 0:
            errors.append(f"Malformed span identity at record {index}")
            continue
        phase = phases.setdefault(name, {"started":0, "completed":0, "durations":[]})
        elapsed = record.get("elapsed_us")
        if elapsed is not None and (type(elapsed) is not int or elapsed < 0):
            errors.append(f"Invalid span timestamp at record {index}")
            continue
        if record["kind"] == "span_begin":
            phase["started"] += 1
            if (identifier in seen or "details" not in data
                    or type(data.get("input_sequence")) is not int or data["input_sequence"] < 0):
                errors.append(f"Duplicate or malformed span_begin {identifier} ({name})")
                continue
            seen.add(identifier)
            active[identifier] = (name, elapsed)
        else:
            begin = active.pop(identifier, None)
            if (begin is None or begin[0] != name or type(data.get("wall_us")) is not int
                    or data["wall_us"] < 0
                    or (begin[1] is not None and elapsed is not None and elapsed < begin[1])):
                errors.append(f"Unpaired or malformed span_end {identifier} ({name})")
                continue
            phase["completed"] += 1
            phase["durations"].append(data["wall_us"])
    for identifier, (name, elapsed) in active.items():
        errors.append(f"Incomplete span {identifier} ({name})")
    valid = not errors if available else None
    for phase in phases.values():
        phase["valid"] = valid
        durations = phase.pop("durations")
        phase["wall_us"] = distribution(durations if valid else [])
    return {"available":available, "valid":valid, "errors":errors, "phases":phases,
            "scope":"Elapsed wall duration, not CPU or GPU time; overlapping spans are not additive"}


def file_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def native_process(command, timeout):
    process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired as failure:
        process.terminate()
        try:
            stdout, stderr = process.communicate(timeout=1)
        except subprocess.TimeoutExpired:
            process.kill()
            stdout, stderr = process.communicate(timeout=1)
        failure.stdout = stdout or failure.stdout
        failure.stderr = stderr or failure.stderr
        raise
    return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def private_output(destination):
    destination = destination.absolute()
    checkout = REPOSITORY.resolve(strict=True)
    cache = checkout / "cache"
    if cache.is_symlink() or cache.resolve(strict=True) != cache:
        raise ValueError("The cache root must be a real directory inside the checkout")
    if ".." in destination.parts or destination.is_symlink():
        raise ValueError("Recording destination cannot contain traversal or a symlink")
    destination.parent.resolve(strict=True).relative_to(cache)
    return destination


def validate_predicates(conditions, remembered, allow_collections=True):
    for condition in conditions:
        if (not isinstance(condition, dict)
                or condition.get("op") not in set(OPERATORS) | {"any_match"}
                or not isinstance(condition.get("path"), str) or not condition["path"]
                or len({"ref", "value"}.intersection(condition)) != 1):
            raise ValueError("Invalid wait predicate")
        if condition["op"] == "any_match":
            nested = condition.get("value")
            if not allow_collections or not isinstance(nested, list) or not 1 <= len(nested) <= 64:
                raise ValueError("any_match requires 1-64 simple predicates on one record")
            validate_predicates(nested, remembered, allow_collections=False)
        elif condition["op"] == "one_of":
            choices = condition.get("value")
            if (not isinstance(choices, list) or not 1 <= len(choices) <= 64
                    or any(type(choice) not in (type(None), bool, int, str) for choice in choices)):
                raise ValueError("one_of requires 1-64 explicit scalar choices")
        elif "ref" in condition:
            reference = condition["ref"]
            if not isinstance(reference, str) or reference.split(".")[0] not in remembered:
                raise ValueError("Wait reference must name an earlier observation")


def validate_plan(plan):
    if (not isinstance(plan, dict) or plan.get("schema") != 1
            or not isinstance(plan.get("steps"), list)
            or not 1 <= len(plan["steps"]) <= 256):
        raise ValueError("Expected schema 1 and between one and 256 steps")
    allowed = {"wait", "remember", "input", "capture", "observe"}
    names, remembered, captures = set(), set(), set()
    for step in plan["steps"]:
        if (not isinstance(step, dict) or not isinstance(step.get("name"), str)
                or not step["name"] or len(allowed.intersection(step)) != 1
                or set(step) - allowed - {"name"}):
            raise ValueError("Each named step requires exactly one operation")
        if step["name"] in names:
            raise ValueError("Step names must be unique")
        names.add(step["name"])
        if "capture" in step:
            stem = step["capture"]
            if not isinstance(stem, str) or not stem.isalnum() or stem in captures:
                raise ValueError("Capture names must be unique and alphanumeric")
            captures.add(stem)
        if "remember" in step:
            alias = step["remember"]
            if not isinstance(alias, str) or not alias.isidentifier() or alias in remembered:
                raise ValueError("Observation names must be unique identifiers")
            remembered.add(alias)
        if "wait" in step and (not isinstance(step["wait"], list) or not step["wait"]):
            raise ValueError("Wait steps require nonempty predicate lists")
        conditions = step.get("wait", [])
        if "observe" in step:
            observation = step["observe"]
            if (not isinstance(observation, dict)
                    or set(observation) - {"duration_seconds", "conditions", "min_tick_progress", "min_frames"}
                    or type(observation.get("duration_seconds")) not in (int, float)
                    or not math.isfinite(observation["duration_seconds"])
                    or not 1 <= observation["duration_seconds"] <= 300
                    or not isinstance(observation.get("conditions"), list) or not observation["conditions"]
                    or type(observation.get("min_tick_progress", 0)) is not int
                    or not 0 <= observation.get("min_tick_progress", 0) <= 2**63 - 1
                    or type(observation.get("min_frames", 2)) is not int
                    or not 2 <= observation.get("min_frames", 2) <= 30000):
                raise ValueError("Observe requires 1-300 seconds, continuous conditions, and bounded progress/draw counts")
            conditions = observation["conditions"]
        validate_predicates(conditions, remembered)
        if "input" in step:
            action = step["input"]
            if not isinstance(action, dict) or action.get("kind") not in {"key", "click", "move"}:
                raise ValueError("Input must be a native key, click or move")
            if action["kind"] == "key":
                if (set(action) - {"kind", "code", "press_ms"} or type(action.get("code")) is not int
                        or action["code"] not in MAC_KEYS):
                    raise ValueError("Native key code must have a supported application receipt mapping")
            else:
                point = action.get("point")
                allowed_input = {"kind", "point", "press_ms"} if action["kind"] == "click" else {"kind", "point"}
                if (set(action) - allowed_input or not isinstance(point, list) or len(point) != 2
                        or any(type(value) not in (int, float) or not math.isfinite(value)
                               or value < 0 for value in point)):
                    raise ValueError("Native pointer input requires a finite nonnegative [x,y] point")
            if "press_ms" in action and (type(action["press_ms"]) is not int or not 1 <= action["press_ms"] <= 1000):
                raise ValueError("Native press duration must be an integer between 1 and 1000 milliseconds")
    return plan


class JsonRecords:
    def __init__(self, data_only=False):
        self.lines = []
        self.data_only = data_only

    def __len__(self):
        return len(self.lines)

    def __getitem__(self, index):
        record = json.loads(self.lines[index])
        return record["data"] if self.data_only else record

    def __iter__(self):
        for index in range(len(self.lines)):
            yield self[index]

    def append(self, line):
        self.lines.append(line)


def matches_frame(frame, conditions, remembered):
    value = frame["state"]["value"]
    if any(condition["path"].startswith("diagnostics.") for condition in conditions):
        value = {**value, "diagnostics": frame["state"].get("diagnostics", {})}
    return matches(value, conditions, remembered)


class Observations:
    def __init__(self, path, started=None):
        self.path = path
        self.started = time.monotonic() if started is None else started
        self.offset = 0
        self.records = JsonRecords()
        self.frames = JsonRecords(data_only=True)
        self.frame_times = {}
        self.partial_record = False
        self.failure = None

    def read(self):
        if self.failure:
            raise RuntimeError(self.failure)
        if not self.path.exists():
            return
        with self.path.open("rb") as stream:
            limit = stream.seek(0, 2)
            stream.seek(self.offset)
            while stream.tell() < limit:
                line = stream.readline(limit - stream.tell())
                if not line.endswith(b"\n"):
                    self.partial_record = True
                    break
                self.partial_record = False
                try:
                    record = json.loads(line)
                    if (not isinstance(record, dict) or not isinstance(record.get("data"), dict)
                            or not isinstance(record.get("kind"), str)):
                        raise ValueError("Invalid telemetry record")
                    if record.get("dropped_records", 0):
                        raise ValueError("Recorder lost events; evidence is incomplete")
                    if record["kind"] in {"host_error", "application_error"}:
                        raise ValueError(f"Window host failed: {record['data'].get('error')}")
                    if record["kind"] == "presentation_error":
                        raise ValueError(f"Presentation observer failed: {record['data']}")
                    data = record["data"]
                    if record["kind"] == "frame":
                        expected_id = len(self.frames) + 1
                        if type(data.get("frame_id")) is not int or data["frame_id"] != expected_id:
                            raise ValueError("Nonconsecutive draw IDs")
                        state = data["state"]
                        if (not isinstance(state, dict) or type(state.get("state_id")) is not int
                                or not isinstance(state.get("value"), dict)
                                or "tick" not in state["value"]):
                            raise ValueError("Incomplete rendered-state observation")
                        value = state["value"]
                        if value.get("error") or value.get("menu_error") or value.get("shell", {}).get("error"):
                            raise ValueError(f"Application error: {value.get('error') or value.get('menu_error') or value['shell']['error']}")
                        terminals = value.get("terminals")
                        if terminals is not None:
                            if (not isinstance(terminals, dict) or terminals.get("enabled") is not True
                                    or type(terminals.get("dropped")) is not int or terminals["dropped"] < 0):
                                raise ValueError("Terminal observer is disabled or malformed; evidence is incomplete")
                            if terminals["dropped"]:
                                raise ValueError("Terminal observer lost events; evidence is incomplete")
                        if (type(data["fence"]) is not int or data["fence"] < 0
                                or type(data["draw_started_unix_ns"]) is not int or data["draw_started_unix_ns"] <= 0
                                or type(data["input_sequence"]) is not int or data["input_sequence"] < 0
                                or "cursor" not in data):
                            raise ValueError("Missing draw timestamp or observer fence")
                        if type(data["widget_draw_cpu_us"]) is not int or data["widget_draw_cpu_us"] < 0:
                            raise ValueError("Invalid widget draw duration")
                        interval = data["draw_interval_us"]
                        if interval is not None and (type(interval) is not int or interval < 0):
                            raise ValueError("Invalid UI draw interval")
                    elif record["kind"] in {"scene_prepare", "scene_encode"}:
                        if (type(data["frame_id"]) is not int or type(data["cpu_us"]) is not int
                                or data["cpu_us"] < 0):
                            raise ValueError("Invalid scene CPU duration")
                except (ValueError, KeyError, TypeError) as failure:
                    self.failure = f"Invalid telemetry at byte {self.offset}: {failure}"
                    raise RuntimeError(self.failure) from failure
                self.records.append(line)
                if record["kind"] == "frame":
                    self.frames.append(line)
                    self.frame_times[record["data"]["frame_id"]] = {
                        "frame_recorder_elapsed_us": record.get("elapsed_us"),
                        "frame_ingested_elapsed_us": int((time.monotonic() - self.started) * 1_000_000),
                    }
                self.offset = stream.tell()

    @property
    def frame(self):
        self.read()
        return self.frames[-1] if self.frames else None


class Session:
    def __init__(self, args, process):
        self.args = args
        self.process = process
        self.started = time.monotonic()
        self.observations = Observations(args.output / "telemetry/events.jsonl", self.started)
        self.remembered = {}
        self.journal = []
        self.deadline = self.started + args.timeout
        self.fence_id = 0
        self.frame_cursor = 0
        self.current_frame = None
        self.pending_frame = None
        self.last_step_frame = None
        self.failure = None
        self.frame_floor = None

    def native(self, *arguments):
        arguments = list(map(str, arguments))
        command = [str(self.args.native_driver), *arguments, "--pid", str(self.process.pid)]
        started = time.time_ns()
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("Bounded window-session deadline exceeded before native command")
        try:
            result = native_process(command, timeout=min(10, remaining))
        except subprocess.TimeoutExpired as failure:
            output = lambda value: value.decode(errors="replace") if isinstance(value, bytes) else value or ""
            self.journal.append({"native_command":list(arguments), "started_unix_ns":started,
                                 "finished_unix_ns":time.time_ns(), "exit_code":None, "timed_out":True,
                                 "stdout":output(failure.stdout).strip(), "stderr":output(failure.stderr).strip()})
            self.release_input(arguments)
            raise
        self.journal.append({"native_command": list(arguments), "started_unix_ns": started,
                             "finished_unix_ns": time.time_ns(), "exit_code": result.returncode,
                             "stdout":result.stdout.strip(), "stderr":result.stderr.strip()})
        if result.returncode:
            self.release_input(arguments)
            raise RuntimeError(result.stderr.strip() or "Native input helper failed")
        return json.loads(result.stdout)

    def release_input(self, arguments):
        if arguments[0] not in {"key", "click", "close"}:
            return
        command = [str(self.args.native_driver), "release", "--pid", str(self.process.pid)]
        if arguments[0] == "key":
            command += ["--code", arguments[arguments.index("--code") + 1]]
        try:
            result = native_process(command, timeout=2)
            self.journal.append({"native_cleanup":True, "command":command,
                                 "exit_code":result.returncode, "stdout":result.stdout.strip(),
                                 "stderr":result.stderr.strip()})
        except (OSError, subprocess.TimeoutExpired) as failure:
            self.journal.append({"native_cleanup":True, "command":command, "error":str(failure)})

    def wait(self, predicate):
        if self.failure:
            raise RuntimeError(self.failure)
        while time.monotonic() < self.deadline:
            if self.process is not None and self.process.poll() is not None:
                raise RuntimeError(f"Application exited with code {self.process.returncode}")
            self.observations.read()
            while self.pending_frame is not None or self.frame_cursor < len(self.observations.frames):
                if time.monotonic() >= self.deadline:
                    break
                if self.pending_frame is not None:
                    frame, self.pending_frame = self.pending_frame, None
                else:
                    frame = self.observations.frames[self.frame_cursor]
                    self.frame_cursor += 1
                if self.frame_floor is not None and (
                        frame["frame_id"] <= self.frame_floor["frame_id"]
                        or frame["fence"] < self.frame_floor["fence"]
                        or frame["input_sequence"] < self.frame_floor["input_sequence"]
                        or frame["draw_started_unix_ns"] <= self.frame_floor["draw_started_unix_ns"]):
                    continue
                self.current_frame = frame
                if predicate(frame):
                    return frame
            time.sleep(min(.02, max(0, self.deadline - time.monotonic())))
        raise TimeoutError("Bounded window-session deadline exceeded while waiting for an explicit state")

    def fence(self, after_unix_ns=0):
        if self.current_frame is None or self.current_frame["state"]["value"]["tick"] is None:
            self.wait(lambda frame: frame["state"]["value"]["tick"] is not None)
        self.native("window")
        self.fence_id += 1
        temporary = self.args.output / "telemetry/observer-fence.next"
        temporary.write_bytes(self.fence_id.to_bytes(8, "little"))
        temporary.replace(self.args.output / "telemetry/observer-fence")
        return self.wait(lambda frame: frame["fence"] == self.fence_id
                         and frame["draw_started_unix_ns"] > after_unix_ns)

    def receipts(self, action, fence, after_unix_ns=0, start_record=0):
        if action["kind"] == "key":
            physical = f"Code({MAC_KEYS[action['code']]})"
            expected = [{"kind":"key_pressed","physical_key":physical},
                        {"kind":"key_released","physical_key":physical}]
        else:
            expected = [{"kind":"pointer_moved","point":action["point"]}]
            if action["kind"] == "click":
                expected += [{"kind":"button_pressed","button":"Left"},
                             {"kind":"button_released","button":"Left"}]
        accepted = []
        for index in range(start_record, len(self.observations.records)):
            record = self.observations.records[index]
            if record["kind"] != "input" or record["data"].get("fence") != fence:
                continue
            received = record["data"].get("received_unix_ns")
            if type(received) is not int or received <= after_unix_ns:
                continue
            event = record["data"].get("input")
            target = expected[len(accepted)]
            if not isinstance(event, dict) or event.get("kind") != target["kind"]:
                continue
            if "point" in target:
                point = event.get("point", [])
                matched = len(point) == 2 and all(abs(actual - wanted) <= 1
                    for actual, wanted in zip(point, target["point"]))
            else:
                matched = all(event.get(key) == value for key, value in target.items())
            if matched:
                accepted.append(index)
                if len(accepted) == len(expected):
                    return accepted
        return None

    def step(self, step):
        started = time.monotonic()
        if "wait" in step:
            frame = self.wait(lambda frame: matches_frame(frame, step["wait"], self.remembered))
        elif "remember" in step:
            frame = (self.last_step_frame if self.last_step_frame is not None
                     else self.wait(lambda frame: frame["state"]["value"]["tick"] is not None))
            self.remembered[step["remember"]] = frame["state"]["value"]
            self.advance_to(frame)
        elif "observe" in step:
            frame = self.observe(step["observe"])
        elif "input" in step:
            self.last_step_frame = None
            before = self.fence()
            replay_cursor = self.frame_cursor
            action = step["input"]
            press = ["--press-ms", action["press_ms"]] if "press_ms" in action else []
            receipt_start = len(self.observations.records)
            input_started_unix_ns = time.time_ns()
            if action["kind"] == "key":
                self.native("key", "--code", action["code"], *press)
            else:
                point = action["point"]
                self.native(action["kind"], "--x", point[0], "--y", point[1],
                            "--content-width", before["state"]["value"]["display_size"][0],
                            "--content-height", before["state"]["value"]["display_size"][1], *press)
            input_finished_unix_ns = time.time_ns()
            self.wait(lambda frame: self.receipts(action, before["fence"], input_started_unix_ns,
                                                  receipt_start) is not None)
            receipts = self.receipts(action, before["fence"], input_started_unix_ns, receipt_start)
            sequences = [self.observations.records[index]["data"].get("input_sequence") for index in receipts]
            if (any(type(sequence) is not int for sequence in sequences)
                    or sequences[0] <= before["input_sequence"]
                    or any(later <= earlier for earlier, later in zip(sequences, sequences[1:]))):
                raise RuntimeError("Native input receipt lacks a fresh input sequence")
            input_sequence = sequences[1] if action["kind"] == "click" else sequences[0]
            self.journal.append({"input_receipts":receipts,
                                 "input_sequence":input_sequence,
                                 "receipt_input_sequences":sequences,
                                 "input_started_unix_ns":input_started_unix_ns,
                                 "input_finished_unix_ns":input_finished_unix_ns,
                                 "fence":before["fence"],"step":step["name"]})
            frame = self.fence(after_unix_ns=input_finished_unix_ns)
            if (frame["input_sequence"] < sequences[-1]
                    or frame["draw_started_unix_ns"] <= input_finished_unix_ns):
                raise RuntimeError("Closing draw did not acknowledge native input receipts after completion")
            self.frame_floor = {"frame_id":before["frame_id"], "fence":before["fence"],
                                "input_sequence":input_sequence,
                                "draw_started_unix_ns":input_started_unix_ns}
            self.frame_cursor = replay_cursor
            self.pending_frame = None
        elif "capture" in step:
            self.last_step_frame = None
            before = self.fence()
            state = before["state"]["value"]
            if state["playing"] or state["pending"]:
                raise RuntimeError("Window captures require an explicitly paused, settled state")
            path = self.args.output / (step["capture"] + ".png")
            self.native("capture", "--path", path)
            captured_unix_ns = time.time_ns()
            frame = self.fence(after_unix_ns=captured_unix_ns)
            if frame["draw_started_unix_ns"] <= captured_unix_ns:
                raise RuntimeError("Closing draw did not occur after capture completion")
            if (frame["state"]["state_id"] != before["state"]["state_id"]
                    or frame["input_sequence"] != before["input_sequence"]
                    or frame["cursor"] != before["cursor"]):
                raise RuntimeError("Rendered state changed during capture; screenshot attribution is ambiguous")
            self.journal.append({"capture": path.name, "state_id":before["state"]["state_id"],
                                 "frame_id_range":[before["frame_id"],frame["frame_id"]],
                                 "observer_fence_range":[before["fence"],frame["fence"]],
                                 "capture_finished_unix_ns":captured_unix_ns,
                                 "closing_draw_started_unix_ns":frame["draw_started_unix_ns"],
                                 "input_sequence":frame["input_sequence"],"cursor":frame["cursor"],
                                 "attribution":"Stable observed state over a frame range; exact compositor frame is not measured"})
            self.pending_frame = frame
            self.frame_floor = {"frame_id":frame["frame_id"] - 1, "fence":frame["fence"],
                                "input_sequence":frame["input_sequence"],
                                "draw_started_unix_ns":captured_unix_ns}
        self.last_step_frame = frame
        self.journal.append({"step":step["name"], "passed":True,
                             "frame_id":frame["frame_id"],"state":frame["state"],
                             "started_elapsed_us":int((started - self.started) * 1_000_000),
                             "finished_elapsed_us":int((time.monotonic() - self.started) * 1_000_000),
                             **self.observations.frame_times.get(frame["frame_id"], {})})
        return frame

    def advance_to(self, frame):
        self.frame_cursor = max(self.frame_cursor, frame["frame_id"])
        self.pending_frame = frame
        self.current_frame = frame

    def observe(self, observation):
        self.observations.read()
        baseline = (self.observations.frames[-1] if self.observations.frames
                    else self.wait(lambda frame: True))
        if not matches_frame(baseline, observation["conditions"], self.remembered):
            self.failure = f"Continuous observation failed: invalid starting frame {baseline['frame_id']}"
            raise RuntimeError(self.failure)
        self.advance_to(baseline)
        self.pending_frame = None
        started = time.monotonic()
        started_unix_ns = time.time_ns()
        duration = observation["duration_seconds"]
        end = started + duration
        end_unix_ns = started_unix_ns + math.ceil(duration * 1_000_000_000)
        deadline = self.deadline
        self.deadline = min(deadline, end + 1)
        first, last, count = None, None, 0
        closing_frame_id = None
        try:
            while True:
                frame = self.wait(lambda candidate: True)
                if frame["draw_started_unix_ns"] <= started_unix_ns:
                    continue
                if not matches_frame(frame, observation["conditions"], self.remembered):
                    raise RuntimeError(f"Continuous observation condition violated at frame {frame['frame_id']}")
                if frame["draw_started_unix_ns"] > started_unix_ns:
                    if first is None:
                        first = frame
                    last = frame
                    count += 1
                if (closing_frame_id is None and time.monotonic() >= end
                        and last is not None and last["draw_started_unix_ns"] >= end_unix_ns):
                    closing_frame_id = self.observations.frames[-1]["frame_id"]
                if (closing_frame_id is not None and last["frame_id"] >= closing_frame_id
                        and last["draw_started_unix_ns"] >= end_unix_ns):
                    break
            minimum = observation.get("min_frames", max(2, math.ceil(duration)))
            if count < minimum:
                raise RuntimeError(f"Continuous observation requires {minimum} fresh draws; observed {count}")
            first_tick, last_tick = first["state"]["value"]["tick"], last["state"]["value"]["tick"]
            progress = (last_tick - first_tick if type(first_tick) is int and type(last_tick) is int else None)
            if observation.get("min_tick_progress", 0) and (progress is None or progress < observation["min_tick_progress"]):
                raise RuntimeError(f"Insufficient simulation tick progress: {progress}; required {observation['min_tick_progress']}")
            self.journal.append({"observe":observation, "fresh_frames":count,
                                 "frame_id_range":[first["frame_id"],last["frame_id"]],
                                 "baseline_frame_id":baseline["frame_id"],
                                 "tick_progress":progress, "duration_seconds":duration,
                                 "started_elapsed_us":int((started - self.started) * 1_000_000),
                                 "finished_elapsed_us":int((time.monotonic() - self.started) * 1_000_000)})
            return last
        except (RuntimeError, TimeoutError) as failure:
            self.failure = f"Continuous observation failed: {failure}"
            self.journal.append({"observe_failed":observation, "fresh_frames":count,
                                 "baseline_frame_id":baseline["frame_id"],
                                 "first_frame_id":first["frame_id"] if first else None,
                                 "last_frame_id":last["frame_id"] if last else None,
                                 "closing_frame_id":closing_frame_id,
                                 "frame_cursor":self.frame_cursor,
                                 "ingested_frames":len(self.observations.frames),
                                 "started_elapsed_us":int((started - self.started) * 1_000_000),
                                 "finished_elapsed_us":int((time.monotonic() - self.started) * 1_000_000),
                                 "deadline_elapsed_us":int((self.deadline - self.started) * 1_000_000)})
            raise
        finally:
            self.deadline = deadline

    def report(self, passed, error):
        if self.failure:
            passed, error = False, f"{error + '; ' if error else ''}{self.failure}"
        try:
            self.observations.read()
            if self.observations.partial_record:
                raise RuntimeError("Telemetry ends with an incomplete record")
        except RuntimeError as failure:
            passed, error = False, f"{error + '; ' if error else ''}{failure}"
        records = list(self.observations.records)
        spans = summarize_spans(records)
        presentations = summarize_presentations(records,
            required=bool(getattr(self.args, "present_library", None)))
        displays = summarize_displays(self.journal,
            required=bool(getattr(self.args, "present_library", None)))
        if displays["valid"] is False:
            passed = False
            error = f"{error + '; ' if error else ''}Invalid display evidence: {'; '.join(displays['errors'])}"
        if presentations["valid"] is False:
            passed = False
            error = f"{error + '; ' if error else ''}Invalid presentation evidence: {'; '.join(presentations['errors'])}"
        if spans["valid"] is False:
            passed = False
            error = f"{error + '; ' if error else ''}Invalid phase span evidence: {'; '.join(spans['errors'])}"
        ended = any(record["kind"] == "session_end" for record in records)
        if passed and not ended:
            passed, error = False, "Recorder did not confirm a graceful session end"
        ready_frames = [frame for frame in self.observations.frames if frame["state"]["value"]["tick"] is not None]
        ready_ids = {frame["frame_id"] for frame in ready_frames}
        metrics = {"widget_draw_cpu_us":distribution([frame["widget_draw_cpu_us"] for frame in ready_frames]),
                   "ui_draw_interval_us":distribution([frame["draw_interval_us"] for frame in ready_frames
                                                        if frame["frame_id"] - 1 in ready_ids])}
        for kind in ("scene_prepare", "scene_encode"):
            metrics[kind + "_cpu_us"] = distribution([record["data"]["cpu_us"]
                for record in records
                if record["kind"] == kind and record["data"]["frame_id"] in ready_ids])
        missing_stages = [kind for kind in ("scene_prepare", "scene_encode")
                          if not metrics[kind + "_cpu_us"]["samples"]]
        if missing_stages:
            passed = False
            error = f"{error + '; ' if error else ''}Missing ready-scene telemetry: {', '.join(missing_stages)}"
        if spans["available"] and not passed:
            spans["valid"] = False
            for phase in spans["phases"].values():
                phase["valid"], phase["wall_us"] = False, distribution([])
        return {"schema":1,"passed":passed,"error":error,"journal":self.journal,
                "metrics":metrics,"phase_spans":spans,"metal_presentations":presentations,
                "display_configuration":displays,
                "gpu_duration_us":None,"presented_fps":None,
                "graceful_end":ended,"metrics_include_capture_overhead":True,
                "missing_scene_stages":missing_stages,
                "ready_frames":len(ready_frames),"loading_frames_excluded":len(self.observations.frames)-len(ready_frames),
                "scope":"Native window events; observed UI draw intervals are not displayed FPS; screenshot capture steps are not throughput benchmarks"}


FRESH_HOUSEHOLDS = {
    "newbie": "UserData/Houses/House07.iff",
    "goth": "UserData/Houses/House05.iff",
    "mashuga": "UserData2/Houses/House09.iff",
}


def application_command(args):
    if args.load is not None:
        saved = json.loads(args.load.read_bytes())
        source = saved.get("session", saved)["source"]
        inputs = ["--load", str(args.load)]
    else:
        source = FRESH_HOUSEHOLDS[args.household]
        inputs = ["--inferred-routing", "--buy-preview", "--neighborhood-policy",
                  "installed-houses-ascending-v1"]
    if not isinstance(source, str) or not source:
        raise ValueError("The selected input has no original-house identity")
    command = [str(args.binary), "--original", source, "--root", str(args.root),
               *inputs, "--paused", "--save", str(args.output / "unused-save.json"),
               "--window-session-output", str(args.output / "telemetry")]
    if args.present_library is not None:
        command += ["--window-session-present-library", str(args.present_library)]
    return command


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("binary", "root", "plan", "output", "native-driver"):
        parser.add_argument("--" + name, type=Path, required=True)
    starting = parser.add_mutually_exclusive_group(required=True)
    starting.add_argument("--load", type=Path)
    starting.add_argument("--household", choices=FRESH_HOUSEHOLDS,
                          help="Import a shipped starting household through production startup")
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--present-library", type=Path)
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("timeout must be positive and finite")
    for name in ("binary", "root", "plan", "native_driver"):
        setattr(args, name, getattr(args, name).resolve(strict=True))
    plan = validate_plan(json.loads(args.plan.read_bytes()))
    args.runner = Path(__file__).resolve(strict=True)
    identity_names = ["binary", "native_driver", "plan", "runner"]
    if args.load is not None:
        args.load = args.load.resolve(strict=True)
        identity_names.append("load")
    else:
        args.original_house = (args.root / FRESH_HOUSEHOLDS[args.household]).resolve(strict=True)
        identity_names.append("original_house")
    if args.present_library is not None:
        args.present_library = args.present_library.resolve(strict=True)
        identity_names.append("present_library")
    args.presentation_analyzer = Path(__file__).with_name("window_presentation.py").resolve(strict=True)
    identity_names.append("presentation_analyzer")
    args.display_analyzer = Path(__file__).with_name("window_display.py").resolve(strict=True)
    identity_names.append("display_analyzer")
    identities = {name: {"path":str(getattr(args, name)), "sha256":file_digest(getattr(args, name))}
                  for name in identity_names}
    args.output = private_output(args.output)
    args.output.mkdir()
    permissions = json.loads(subprocess.check_output([str(args.native_driver), "permissions"], text=True, timeout=10))
    if not all(permissions.get(name) is True for name in (
            "event_post_allowed", "screen_capture_allowed", "accessibility_trusted")):
        (args.output / "result.json").write_text(json.dumps({"passed":False,"permissions":permissions,
            "error":"Native input and screen capture permissions are required; no application was launched"},indent=2)+"\n")
        parser.error("macOS denied native input or capture; run the native driver with permissions --request")
    command = application_command(args)
    with (args.output / "application.log").open("x") as log:
        process_started_unix_ns = time.time_ns()
        process_started_host_ns = host_time_ns()
        process = subprocess.Popen(command, cwd=REPOSITORY, stdout=log, stderr=subprocess.STDOUT)
        session = Session(args, process)
        passed, error = False, None
        try:
            for step in plan["steps"]:
                session.step(step)
            session.native("close")
            process.wait(timeout=10)
            if process.returncode != 0:
                raise RuntimeError(f"Application close returned {process.returncode}")
            passed = True
        except Exception as failure:
            error = f"{type(failure).__name__}: {failure}"
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
        process_finished_host_ns = host_time_ns()
        process_finished_unix_ns = time.time_ns()
        report = session.report(passed,error)
        report["process_lifecycle"] = {
            "started_unix_ns": process_started_unix_ns, "finished_unix_ns": process_finished_unix_ns,
            "started_host_ns": process_started_host_ns, "finished_host_ns": process_finished_host_ns,
            "host_clock": HOST_CLOCK,
        }
        for name, identity in identities.items():
            try:
                identity["final_sha256"] = file_digest(getattr(args, name))
            except OSError:
                identity["final_sha256"] = None
            if identity["final_sha256"] != identity["sha256"]:
                report["passed"] = False
                report["error"] = f"{report['error'] + '; ' if report['error'] else ''}{name} changed during execution"
        passed = report["passed"]
        report["inputs"] = identities
        report["application_command"] = command
        report["starting_state"] = {"kind": "fresh" if args.load is None else "managed_resume",
                                    "household": args.household}
        (args.output / "result.json").write_text(json.dumps(report,indent=2)+"\n")
    print(f"passed={passed}; result: {args.output / 'result.json'}")
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
