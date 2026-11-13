import importlib.util
from contextlib import contextmanager
import json
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock


SPEC = importlib.util.spec_from_file_location("window_session", Path(__file__).with_name("window_session.py"))
DRIVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DRIVER)


def frame_record(identifier, tick=0, fence=0, input_sequence=0, draw_started_unix_ns=1, **state):
    return {"kind":"frame", "elapsed_us":identifier * 10,
            "data":{"frame_id":identifier, "fence":fence,
                    "input_sequence":input_sequence, "cursor":None,
                    "draw_started_unix_ns":draw_started_unix_ns,
                    "widget_draw_cpu_us":1, "draw_interval_us":1,
                    "state":{"state_id":identifier, "value":{
                        "tick":tick, "error":None, "menu_error":None, **state}}}}


def append_records(session, records):
    with session.observations.path.open("a") as stream:
        stream.write("".join(json.dumps(record) + "\n" for record in records))


@contextmanager
def synthetic_session(directory, schedule=(), timeout=5):
    clock = SimpleNamespace(now=0.0)
    pending = list(schedule)
    root = Path(directory)
    (root / "telemetry").mkdir()
    def unix_ns():
        return 1_000_000_000 + round(clock.now * 1_000_000_000)
    def sleep(seconds):
        clock.now += seconds
        while pending and pending[0][0] <= clock.now:
            instant, record = pending.pop(0)
            record["data"]["draw_started_unix_ns"] = 1_000_000_000 + round(instant * 1_000_000_000)
            append_records(session, [record])
    with mock.patch.multiple(DRIVER.time, monotonic=lambda: clock.now, time_ns=unix_ns, sleep=sleep):
        session = DRIVER.Session(SimpleNamespace(output=root, timeout=timeout), None)
        sleep(0)
        yield session, clock


class WindowSessionTests(unittest.TestCase):
    def test_fresh_window_commands_use_real_startup_without_a_checkpoint(self):
        for household, source in DRIVER.FRESH_HOUSEHOLDS.items():
            args = SimpleNamespace(binary=Path('/synthetic/sims-room'), root=Path('/synthetic/install'),
                                   output=Path('/synthetic/cache/new-run'), load=None,
                                   household=household, present_library=None)
            command = DRIVER.application_command(args)
            self.assertEqual(command[command.index('--original') + 1], source)
            self.assertNotIn('--load', command)
            self.assertIn('--buy-preview', command)
            self.assertIn('--inferred-routing', command)
            self.assertEqual(command[command.index('--save') + 1], '/synthetic/cache/new-run/unused-save.json')

    def test_window_resume_command_preserves_saved_policies(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'synthetic-save.json'
            path.write_text(json.dumps({'session': {'source': 'synthetic-house.iff'}}))
            args = SimpleNamespace(binary=Path('/synthetic/sims-room'), root=Path('/synthetic/install'),
                                   output=Path('/synthetic/cache/new-run'), load=path,
                                   household=None, present_library=Path('/synthetic/observer.dylib'))
            command = DRIVER.application_command(args)
            self.assertEqual(command[command.index('--original') + 1], 'synthetic-house.iff')
            self.assertEqual(command[command.index('--load') + 1], str(path))
            self.assertNotIn('--buy-preview', command)
            self.assertNotIn('--neighborhood-policy', command)
            self.assertEqual(command[command.index('--window-session-present-library') + 1], str(args.present_library))

    def test_diagnostic_wait_does_not_require_or_modify_visible_state(self):
        record = frame_record(1, tick=7)
        frame = record['data']
        frame['state']['diagnostics'] = {'worker_busy': True}
        before = json.dumps(frame, sort_keys=True)
        conditions = [{'path': 'tick', 'op': 'eq', 'value': 7},
                      {'path': 'diagnostics.worker_busy', 'op': 'eq', 'value': True}]
        self.assertTrue(DRIVER.matches_frame(frame, conditions, {}))
        frame['state']['diagnostics']['worker_busy'] = False
        self.assertFalse(DRIVER.matches_frame(frame, conditions, {}))
        frame['state']['diagnostics']['worker_busy'] = True
        self.assertEqual(json.dumps(frame, sort_keys=True), before)
        self.assertNotIn('diagnostics', frame['state']['value'])

    def test_ingestion_retains_shared_raw_bytes_and_decodes_independent_snapshots(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            expected = frame_record(1, tick=7, nested={"items":[1,2]})
            append_records(session, [expected])
            session.observations.read()
            records, frames = session.observations.records, session.observations.frames
            self.assertIsInstance(records.lines[0], bytes)
            self.assertIs(records.lines[0], frames.lines[0])
            self.assertEqual(list(records), [expected])
            self.assertEqual(list(records), [expected])
            snapshot = frames[0]
            snapshot["state"]["value"]["nested"]["items"].append(3)
            snapshot["state"]["value"]["tick"] = 99
            self.assertEqual(frames[0], expected["data"])
            self.assertEqual(records[0], expected)

    def test_raw_record_storage_preserves_absolute_receipt_indices(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            pressed = {"kind":"input", "data":{"fence":1, "received_unix_ns":3,
                "input":{"kind":"key_pressed", "physical_key":"Code(ArrowRight)"}}}
            released = {"kind":"input", "data":{"fence":1, "received_unix_ns":4,
                "input":{"kind":"key_released", "physical_key":"Code(ArrowRight)"}}}
            records = [frame_record(1), pressed, frame_record(2), released]
            append_records(session, records)
            session.observations.read()
            self.assertEqual(session.receipts({"kind":"key", "code":124}, 1), [1,3])
            self.assertEqual(len(session.observations.records), 4)
            self.assertEqual(session.observations.frames[1], records[2]["data"])

    @unittest.skipUnless(sys.platform == "darwin", "native Mach clock requires macOS")
    def test_native_host_clock_has_a_common_epoch_with_quartz_and_python_processes(self):
        before = DRIVER.host_time_ns()
        command = [sys.executable, "-B", "-c",
            "import sys; sys.path.insert(0, sys.argv[1]); "
            "from window_session import host_time_ns; print(host_time_ns())",
            str(Path(__file__).resolve().parent)]
        child = int(DRIVER.subprocess.check_output(command, text=True))
        after = DRIVER.host_time_ns()
        self.assertLessEqual(before, child)
        self.assertLessEqual(child, after)
        quartz = DRIVER.ctypes.CDLL("/System/Library/Frameworks/QuartzCore.framework/QuartzCore")
        current_media_time = quartz.CACurrentMediaTime
        current_media_time.argtypes = []
        current_media_time.restype = DRIVER.ctypes.c_double
        before = DRIVER.host_time_ns()
        quartz_ns = current_media_time() * 1_000_000_000
        after = DRIVER.host_time_ns()
        self.assertLessEqual(before, quartz_ns)
        self.assertLessEqual(quartz_ns, after)

    def test_explicit_scalar_allowlists_preserve_other_fingerprint_predicates(self):
        conditions = [{"path": "origin.instruction", "op": "one_of", "value": [5, 14]},
                      {"path": "origin.source", "op": "eq", "value": "synthetic-source"}]
        DRIVER.validate_predicates(conditions, set())
        for instruction in (5, 14):
            self.assertTrue(DRIVER.matches({"origin": {"instruction": instruction,
                                           "source": "synthetic-source"}}, conditions, {}))
        for origin in ({"instruction": 10, "source": "synthetic-source"},
                       {"instruction": 5, "source": "unrelated"},
                       {"instruction": "5", "source": "synthetic-source"},
                       {"source": "synthetic-source"}):
            self.assertFalse(DRIVER.matches({"origin": origin}, conditions, {}))
        for choices in ([], list(range(65)), "5,14", [{"instruction": 5}], [5.0]):
            with self.subTest(choices=choices), self.assertRaisesRegex(ValueError, "explicit scalar"):
                DRIVER.validate_predicates([{"path": "instruction", "op": "one_of", "value": choices}], set())
        with self.assertRaises(ValueError):
            DRIVER.validate_predicates([{"path": "instruction", "op": "one_of", "ref": "earlier.choices"}], {"earlier"})

    def test_long_linear_plans_remain_bounded(self):
        plan = {"schema": 1, "steps": [{"name": f"checkpoint-{index}",
                "wait": [{"path": "tick", "op": "ge", "value": 0}]}
                for index in range(256)]}
        DRIVER.validate_plan(plan)
        plan["steps"].append({"name": "overflow", "remember": "overflow"})
        with self.assertRaisesRegex(ValueError, "256 steps"):
            DRIVER.validate_plan(plan)

    def test_collection_predicates_require_one_fully_qualified_terminal_record(self):
        remembered = {"queued":{"serial":7}, "route":{"invocation":2}}
        required = [{"path":"serial", "op":"eq", "ref":"queued.serial"},
                    {"path":"invocation", "op":"eq", "ref":"route.invocation"},
                    {"path":"reason", "op":"eq", "value":"normal_return"},
                    {"path":"cancelled", "op":"eq", "value":True}]
        conditions = [{"path":"events", "op":"any_match", "value":required}]
        terminal = {"serial":7, "invocation":2, "reason":"normal_return", "cancelled":True}
        later = {"serial":8, "invocation":3, "reason":"normal_return", "cancelled":False}
        self.assertTrue(DRIVER.matches({"events":[terminal, later]}, conditions, remembered))
        for change in [{"serial":8}, {"invocation":3}, {"reason":"reset"},
                       {"reason":"replacement"}, {"cancelled":False}]:
            self.assertFalse(DRIVER.matches({"events":[{**terminal, **change}, later]}, conditions, remembered))
        self.assertFalse(DRIVER.matches({"events":[{**terminal,"serial":8},
                                                   {**terminal,"reason":"reset"}]}, conditions, remembered))
        for records in [[], None, {}, [None], [7]]:
            self.assertFalse(DRIVER.matches({"events":records}, conditions, remembered))
        self.assertFalse(DRIVER.matches({"events":[terminal]}, conditions, {}))

    def test_collection_predicate_schema_limits_nesting_and_validates_references(self):
        simple = {"path":"serial", "op":"eq", "ref":"queued.serial"}
        predicate = {"path":"events", "op":"any_match", "value":[simple]}
        def plan(condition):
            return {"schema":1, "steps":[{"name":"remember", "remember":"queued"},
                {"name":"terminal", "wait":[condition]}]}
        DRIVER.validate_plan(plan(predicate))
        for value in [[], None, {}, [simple] * 65, [predicate],
                      [{"path":"serial", "op":"eq", "ref":"missing.serial"}],
                      [{"path":"serial", "op":"unknown", "value":7}]]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                DRIVER.validate_plan(plan({**predicate,"value":value}))
        with self.assertRaises(ValueError):
            DRIVER.validate_plan(plan({"path":"events", "op":"any_match", "ref":"queued.serial"}))

    def test_terminal_evidence_loss_is_latched_even_if_a_later_frame_is_clean(self):
        with tempfile.TemporaryDirectory() as directory:
            with synthetic_session(directory) as (session, clock):
                append_records(session, [frame_record(1, terminals={"enabled":True,"dropped":1}),
                                         frame_record(2, terminals={"enabled":True,"dropped":0})])
                with self.assertRaisesRegex(RuntimeError, "Terminal observer lost events"):
                    session.observations.read()
                with self.assertRaisesRegex(RuntimeError, "Terminal observer lost events"):
                    session.observations.read()

    def test_disabled_terminal_observer_cannot_silently_erase_producer_evidence(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            append_records(session, [frame_record(1, terminals={"enabled":True,"dropped":0}),
                                     frame_record(2, terminals={"enabled":False,"dropped":0})])
            with self.assertRaisesRegex(RuntimeError, "Terminal observer is disabled"):
                session.observations.read()
            with self.assertRaisesRegex(RuntimeError, "Terminal observer is disabled"):
                session.observations.read()

    def test_not_contains_projected_action_ids_and_missing_references(self):
        conditions = [{"path":"action_ids", "op":"not_contains", "value":42}]
        self.assertTrue(DRIVER.matches({"action_ids":[3,7]}, conditions, {}))
        self.assertFalse(DRIVER.matches({"action_ids":[3,42]}, conditions, {}))
        self.assertFalse(DRIVER.matches({}, conditions, {}))
        self.assertFalse(DRIVER.matches({"action_ids":None}, conditions, {}))
        self.assertFalse(DRIVER.matches({}, [{"path":"tick", "op":"eq", "ref":"saved.missing"}], {}))
        DRIVER.validate_plan({"schema":1, "steps":[{"name":"absent", "wait":conditions}]})

    def test_observe_schema_is_explicit_and_tightly_bounded(self):
        observation = {"duration_seconds":1, "conditions":[{"path":"modal", "op":"eq", "value":None}]}
        for seconds in [1, 1.5, 300]:
            DRIVER.validate_plan({"schema":1, "steps":[{"name":"soak", "observe":{
                **observation, "duration_seconds":seconds, "min_tick_progress":1, "min_frames":2}}]})
        invalid = [{"duration_seconds":value} for value in [0, 301, True, "1", float("inf"), float("nan")]]
        invalid += [{"conditions":value} for value in [[], None, {}]]
        invalid += [{"min_tick_progress":value} for value in [-1, True, 1.5, 2**63]]
        invalid += [{"min_frames":value} for value in [0, 1, 30001, True, 2.5]]
        invalid += [{"unknown":1}, {"conditions":[{"path":"tick", "op":"eq", "ref":"missing.tick"}]}]
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                DRIVER.validate_plan({"schema":1, "steps":[{"name":"bad", "observe":{**observation, **changes}}]})
        DRIVER.validate_plan({"schema":1, "steps":[{"name":"saved", "remember":"saved"},
            {"name":"soak", "observe":{**observation, "conditions":[{"path":"tick", "op":"ge", "ref":"saved.tick"}]}}]})

    def test_batched_waits_preserve_transient_active_and_terminal_frames(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            append_records(session, [frame_record(1, phase="idle"), frame_record(2, phase="active"),
                                     frame_record(3, phase="terminal"), frame_record(4, phase="idle")])
            active = session.step({"name":"active", "wait":[{"path":"phase", "op":"eq", "value":"active"}]})
            terminal = session.step({"name":"terminal", "wait":[{"path":"phase", "op":"eq", "value":"terminal"}]})
            self.assertEqual((active["frame_id"], terminal["frame_id"]), (2,3))
            with self.assertRaises(TimeoutError):
                session.step({"name":"stale active", "wait":[{"path":"phase", "op":"eq", "value":"active"}]})
            self.assertLessEqual(clock.now, 5)

    def test_remember_pins_exact_wait_match_even_when_more_draws_arrive(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            append_records(session, [frame_record(1, tick=10, phase="active"), frame_record(2, tick=11, phase="idle")])
            matched = session.step({"name":"active", "wait":[{"path":"phase", "op":"eq", "value":"active"}]})
            append_records(session, [frame_record(3, tick=12, phase="idle")])
            session.step({"name":"remember active", "remember":"active"})
            self.assertIs(session.remembered["active"], matched["state"]["value"])
            self.assertEqual(session.journal[-1]["frame_id"], 1)
            self.assertEqual(session.journal[-1]["frame_recorder_elapsed_us"], 10)
            self.assertIn("frame_ingested_elapsed_us", session.journal[-1])
            self.assertIn("started_elapsed_us", session.journal[-1])
            self.assertEqual(session.step({"name":"next", "wait":[{"path":"tick", "op":"gt", "ref":"active.tick"}]})["frame_id"], 2)

    def native_race(self, session, clock, closing_sequence=2, stale_receipts=False):
        append_records(session, [frame_record(1, modal="old", phase="idle", display_size=[100,100])])
        def native(*arguments):
            if arguments[0] == "window":
                clock.now += .01
                identifier = 2 if session.fence_id == 0 else 6
                append_records(session, [frame_record(identifier, fence=session.fence_id + 1,
                    input_sequence=0 if identifier == 2 else closing_sequence,
                    draw_started_unix_ns=DRIVER.time.time_ns(), modal="old" if identifier == 2 else None,
                    phase="idle" if identifier == 2 else "terminal", display_size=[100,100])])
            elif arguments[0] == "key":
                clock.now += .01
                received = DRIVER.time.time_ns() - (20_000_001 if stale_receipts else 0)
                append_records(session, [
                    frame_record(3, fence=1, input_sequence=0, draw_started_unix_ns=DRIVER.time.time_ns(), modal="old", phase="idle"),
                    {"kind":"input", "data":{"fence":1, "input_sequence":1, "received_unix_ns":received,
                        "input":{"kind":"key_pressed", "physical_key":"Code(Space)"}}},
                    frame_record(4, fence=1, input_sequence=1, draw_started_unix_ns=DRIVER.time.time_ns(), modal=None, phase="active"),
                    {"kind":"input", "data":{"fence":1, "input_sequence":2, "received_unix_ns":received,
                        "input":{"kind":"key_released", "physical_key":"Code(Space)"}}},
                    frame_record(5, fence=1, input_sequence=2, draw_started_unix_ns=DRIVER.time.time_ns(), modal=None, phase="terminal"),
                ])
                clock.now += .01
            return {}
        session.native = mock.Mock(side_effect=native)
        closing = session.step({"name":"native press", "input":{"kind":"key", "code":49, "press_ms":80}})
        self.assertEqual(closing["frame_id"], 6)
        self.assertIn(mock.call("key", "--code", 49, "--press-ms", 80), session.native.call_args_list)

    def test_input_fences_preserve_early_transients_without_replaying_stale_modals(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            self.native_race(session, clock)
            active = session.step({"name":"active", "wait":[{"path":"phase", "op":"eq", "value":"active"}]})
            self.assertEqual(active["frame_id"], 4)
            session.step({"name":"exact active", "remember":"active"})
            self.assertEqual(session.remembered["active"]["phase"], "active")
            terminal = session.step({"name":"terminal", "wait":[{"path":"phase", "op":"eq", "value":"terminal"}]})
            self.assertEqual(terminal["frame_id"], 5)
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            self.native_race(session, clock)
            with self.assertRaises(TimeoutError):
                session.step({"name":"stale modal ack", "wait":[{"path":"modal", "op":"eq", "value":"old"}]})

    def test_remember_after_input_uses_closing_draw_not_pre_input_wait(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            self.native_race(session, clock)
            remembered = session.step({"name":"native result", "remember":"result"})
            self.assertEqual(remembered["frame_id"], 6)
            self.assertIsNone(session.remembered["result"]["modal"])
            with self.assertRaises(TimeoutError):
                session.step({"name":"cannot rewind", "wait":[{"path":"phase", "op":"eq", "value":"active"}]})

    def test_pre_command_receipts_cannot_acknowledge_a_later_native_command(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            with self.assertRaises(TimeoutError):
                self.native_race(session, clock, stale_receipts=True)
            self.assertFalse(any("input_receipts" in entry for entry in session.journal))

    def test_continuous_observation_discards_pre_interval_replay_but_not_global_errors(self):
        schedule = [(.1, frame_record(7, phase="terminal", fence=2, input_sequence=2)),
                    (1.06, frame_record(8, phase="terminal", fence=2, input_sequence=2))]
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, schedule) as (session, clock):
            self.native_race(session, clock)
            frame = session.step({"name":"terminal interval", "observe":{"duration_seconds":1,
                "conditions":[{"path":"phase", "op":"eq", "value":"terminal"}]}})
            self.assertEqual(frame["frame_id"], 8)

    def test_continuous_observation_rejects_an_invalid_known_baseline(self):
        schedule = [(0, frame_record(1, healthy=False)), (.99, frame_record(2, healthy=True)),
                    (1.02, frame_record(3, healthy=True))]
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, schedule) as (session, clock):
            session.wait(lambda frame: True)
            with self.assertRaisesRegex(RuntimeError, "invalid starting frame 1"):
                session.step({"name":"known unhealthy", "observe":{"duration_seconds":1,
                    "conditions":[{"path":"healthy", "op":"eq", "value":True}]}})
            self.assertEqual(clock.now, 0)

    def test_continuous_observation_without_a_baseline_starts_at_the_first_fresh_draw(self):
        schedule = [(.99, frame_record(1, healthy=True)), (1.02, frame_record(2, healthy=True)),
                    (1.5, frame_record(3, healthy=True)), (2.02, frame_record(4, healthy=True))]
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, schedule) as (session, clock):
            frame = session.step({"name":"delayed baseline", "observe":{"duration_seconds":1,
                "conditions":[{"path":"healthy", "op":"eq", "value":True}]}})
            self.assertEqual(frame["frame_id"], 4)
            self.assertGreaterEqual(clock.now, 1.99)

    def test_input_closing_draw_must_acknowledge_the_complete_receipt_sequence(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            with self.assertRaisesRegex(RuntimeError, "acknowledge native input receipts"):
                self.native_race(session, clock, closing_sequence=1)
            self.assertIsNone(session.last_step_frame)

    def test_wait_freshness_requires_draw_time_fence_and_input_sequence(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            session.frame_floor = {"frame_id":0, "fence":2, "input_sequence":2, "draw_started_unix_ns":10}
            append_records(session, [
                frame_record(1, fence=2, input_sequence=2, draw_started_unix_ns=10),
                frame_record(2, fence=1, input_sequence=2, draw_started_unix_ns=11),
                frame_record(3, fence=2, input_sequence=1, draw_started_unix_ns=11),
                frame_record(4, fence=2, input_sequence=2, draw_started_unix_ns=11),
            ])
            self.assertEqual(session.wait(lambda frame: True)["frame_id"], 4)

    def test_continuous_observation_reports_fresh_draws_and_real_tick_progress(self):
        schedule = [(0, frame_record(1, tick=3, modal=None)), (.1, frame_record(2, tick=4, modal=None)),
                    (.5, frame_record(3, tick=5, modal=None)), (1.02, frame_record(4, tick=6, modal=None))]
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, schedule) as (session, clock):
            frame = session.step({"name":"bounded soak", "observe":{"duration_seconds":1,
                "conditions":[{"path":"modal", "op":"eq", "value":None}], "min_tick_progress":2, "min_frames":3}})
            evidence = session.journal[-2]
            self.assertEqual(frame["frame_id"], 4)
            self.assertEqual(evidence["fresh_frames"], 3)
            self.assertEqual(evidence["tick_progress"], 2)
            self.assertEqual(evidence["frame_id_range"], [2,4])
            self.assertGreaterEqual(clock.now, 1)
            self.assertLess(clock.now, 1.1)

    def test_continuous_observation_finishes_with_an_advancing_buffered_tail(self):
        for violation in (False, True):
            with self.subTest(violation=violation), tempfile.TemporaryDirectory() as directory, \
                    synthetic_session(directory, [(0, frame_record(1, healthy=True))]) as (session, clock):
                original_read = session.observations.read
                reads = 0

                def growing_read():
                    nonlocal reads
                    reads += 1
                    count = 0 if reads == 1 else (5 if reads == 2 else 1)
                    for _ in range(count):
                        clock.now += .01
                        identifier = len(session.observations.frames) + 1
                        append_records(session, [frame_record(identifier, healthy=not (
                            violation and identifier == 50),
                            draw_started_unix_ns=DRIVER.time.time_ns())])
                        original_read()
                    original_read()

                with mock.patch.object(session.observations, "read", side_effect=growing_read):
                    step = {"name":"growing stream", "observe":{"duration_seconds":1,
                        "conditions":[{"path":"healthy", "op":"eq", "value":True}]}}
                    if violation:
                        with self.assertRaisesRegex(RuntimeError, "violated at frame 50"):
                            session.step(step)
                    else:
                        frame = session.step(step)
                        self.assertGreaterEqual(frame["draw_started_unix_ns"], 2_000_000_000)
                        self.assertLess(clock.now, 1.1)
                        self.assertLess(session.frame_cursor, len(session.observations.frames))
                        self.assertGreaterEqual(session.journal[-2]["fresh_frames"], 100)

    def test_continuous_observation_latches_mid_batch_modal_and_invariant_failure(self):
        for field, value in [("modal", "unexpected"), ("healthy", False)]:
            schedule = [(0, frame_record(1, modal=None, healthy=True)),
                        (.101, frame_record(2, **{field:value})),
                        (.102, frame_record(3, modal=None, healthy=True))]
            conditions = [{"path":"modal", "op":"eq", "value":None}, {"path":"healthy", "op":"eq", "value":True}]
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory, \
                    synthetic_session(directory, schedule) as (session, clock):
                with self.assertRaisesRegex(RuntimeError, "violated at frame 2"):
                    session.step({"name":"soak", "observe":{"duration_seconds":1, "conditions":conditions}})
                self.assertEqual(len(session.observations.frames), 3)
                report = session.report(True, None)
                self.assertFalse(report["passed"])
                self.assertIn("violated at frame 2", report["error"])
                with self.assertRaisesRegex(RuntimeError, "Continuous observation failed"):
                    session.wait(lambda frame: True)

    def test_continuous_observation_latches_transient_application_errors(self):
        schedule = [(.101, frame_record(1, error="transient failure")), (.102, frame_record(2))]
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, schedule) as (session, clock):
            with self.assertRaisesRegex(RuntimeError, "transient failure"):
                session.step({"name":"soak", "observe":{"duration_seconds":1,
                    "conditions":[{"path":"tick", "op":"ge", "value":0}]}})
            self.assertFalse(session.report(True, None)["passed"])

    def test_observation_checks_the_entire_ingested_endpoint_batch(self):
        schedule = [(0, frame_record(1, healthy=True)),
                    (1.001, frame_record(2, healthy=True)),
                    (1.002, frame_record(3, healthy=False)),
                    (1.003, frame_record(4, healthy=True))]
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, schedule) as (session, clock):
            with self.assertRaisesRegex(RuntimeError, "violated at frame 3"):
                session.step({"name":"endpoint failure", "observe":{"duration_seconds":1,
                    "conditions":[{"path":"healthy", "op":"eq", "value":True}]}})
            self.assertEqual(len(session.observations.frames), 4)

    def test_wall_time_and_new_draws_do_not_earn_simulation_progress(self):
        for tick in [None, 7]:
            schedule = [(0, frame_record(1, tick=tick)), (.1, frame_record(2, tick=tick)),
                        (1.02, frame_record(3, tick=tick))]
            with self.subTest(tick=tick), tempfile.TemporaryDirectory() as directory, \
                    synthetic_session(directory, schedule) as (session, clock):
                with self.assertRaisesRegex(RuntimeError, "tick progress"):
                    session.step({"name":"no progress", "observe":{"duration_seconds":1,
                        "conditions":[{"path":"error", "op":"eq", "value":None}], "min_tick_progress":1}})
                self.assertFalse(session.report(True, None)["passed"])

    def test_observation_requires_fresh_draws_and_a_fresh_duration_endpoint(self):
        for schedule, message in [([], "deadline"),
                ([(0, frame_record(1)), (1.02, frame_record(2))], "fresh draws"),
                ([(.1, frame_record(1)), (.2, frame_record(2))], "deadline")]:
            with self.subTest(schedule=schedule), tempfile.TemporaryDirectory() as directory, \
                    synthetic_session(directory, schedule) as (session, clock):
                with self.assertRaisesRegex((RuntimeError, TimeoutError), message):
                    session.step({"name":"no evidence", "observe":{"duration_seconds":1,
                        "conditions":[{"path":"tick", "op":"ge", "value":0}]}})
                self.assertLessEqual(clock.now, 5)
                self.assertEqual(session.deadline, 5)

    def test_observation_never_extends_the_session_deadline(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, timeout=.3) as (session, clock):
            with self.assertRaises(TimeoutError):
                session.step({"name":"bounded", "observe":{"duration_seconds":300,
                    "conditions":[{"path":"tick", "op":"ge", "value":0}]}})
            self.assertLessEqual(clock.now, .3)

    def test_failed_observation_retains_cursor_and_endpoint_diagnostics(self):
        with tempfile.TemporaryDirectory() as directory, \
                synthetic_session(directory, [(0, frame_record(1))]) as (session, clock):
            with self.assertRaises(TimeoutError):
                session.step({"name":"missing draws", "observe":{"duration_seconds":1,
                    "conditions":[{"path":"tick", "op":"ge", "value":0}]}})
            failure = session.journal[-1]
            self.assertEqual(failure["baseline_frame_id"], 1)
            self.assertEqual(failure["frame_cursor"], 1)
            self.assertEqual(failure["ingested_frames"], 1)
            self.assertEqual(failure["fresh_frames"], 0)
            self.assertIsNone(failure["closing_frame_id"])
            self.assertEqual(failure["deadline_elapsed_us"], 2_000_000)
            self.assertGreaterEqual(failure["finished_elapsed_us"], failure["deadline_elapsed_us"])

    def test_native_press_duration_is_explicit_and_bounded(self):
        for kind, coordinates in [("key", {"code":49}), ("click", {"point":[1,2]})]:
            for duration in [1,80,250,1000]:
                DRIVER.validate_plan({"schema":1,"steps":[{"name":"press","input":{
                    "kind":kind,**coordinates,"press_ms":duration}}]})
            for duration in [0,1001,True,1.5,"80"]:
                with self.subTest(kind=kind,duration=duration), self.assertRaises(ValueError):
                    DRIVER.validate_plan({"schema":1,"steps":[{"name":"press","input":{
                        "kind":kind,**coordinates,"press_ms":duration}}]})
        with self.assertRaises(ValueError):
            DRIVER.validate_plan({"schema":1,"steps":[{"name":"move","input":{
                "kind":"move","point":[1,2],"press_ms":80}}]})

    def test_wait_predicates_reference_observed_state(self):
        self.assertTrue(DRIVER.matches({"tick":12,"camera":[3,4]}, [
            {"path":"tick","op":"gt","ref":"initial.tick"},
            {"path":"camera.0","op":"eq","value":3},
        ], {"initial":{"tick":10}}))
        self.assertFalse(DRIVER.matches({"tick":10}, [{"path":"tick","op":"gt","value":10}], {}))
        self.assertFalse(DRIVER.matches({}, [{"path":"tick","op":"gt","value":10}], {}))

    def test_distribution_uses_nearest_rank_and_handles_no_samples(self):
        self.assertEqual(DRIVER.distribution([]), {"samples":0})
        self.assertEqual(DRIVER.distribution([None,3,1,2])["p50"], 2)
        self.assertEqual(DRIVER.distribution(list(range(1,101)))["p95"], 95)

    def span_records(self, identifier=1, name="worker_apply", wall_us=10):
        return [
            {"kind":"span_begin", "elapsed_us":20, "data":{
                "span_id":identifier, "name":name, "details":None, "input_sequence":0}},
            {"kind":"span_end", "elapsed_us":40, "data":{
                "span_id":identifier, "name":name, "wall_us":wall_us}},
        ]

    def test_phase_spans_report_nearest_rank_wall_distributions_by_name(self):
        records = []
        for identifier in range(1,101):
            records += self.span_records(identifier, wall_us=identifier)
        for identifier, name in enumerate(["publication_capture", "prepare_publication", "publication_refresh",
                "engine_step", "native_frame_work", "engine_routes"], 101):
            records += self.span_records(identifier, name=name)
        report = DRIVER.summarize_spans(records)
        self.assertTrue(report["available"])
        self.assertTrue(report["valid"])
        phase = report["phases"]["worker_apply"]
        self.assertEqual((phase["started"], phase["completed"]), (100,100))
        self.assertEqual(phase["wall_us"], {"samples":100, "p50":50, "p95":95, "p99":99, "worst":100})
        self.assertIn("not CPU or GPU time", report["scope"])
        self.assertEqual(report["phases"]["engine_routes"]["wall_us"]["samples"], 1)

    def test_overlapping_phase_spans_pair_by_id_not_stack_order(self):
        first = self.span_records(1)
        second = self.span_records(2, name="publication_capture")
        report = DRIVER.summarize_spans([first[0], second[0], first[1], second[1]])
        self.assertTrue(report["valid"])
        self.assertEqual(report["errors"], [])

    def test_bad_span_pairs_never_produce_valid_phase_measurements(self):
        begin, end = self.span_records()
        invalid = [
            [begin], [end], [end,begin], [begin,begin,end], [begin,end,end],
            [begin, {"kind":"session_end", "data":{}}, end],
            [{"kind":"session_end", "data":{}}, begin, end],
            [begin, {**end, "data":{**end["data"], "name":"engine_step"}}],
            [begin, {**end, "elapsed_us":19}],
            [begin, {**end, "data":{**end["data"], "wall_us":-1}}],
            [begin, {**end, "data":{**end["data"], "wall_us":True}}],
            [begin, {**end, "data":{**end["data"], "wall_us":1.5}}],
            [{**begin, "data":{**begin["data"], "input_sequence":True}}, end],
            [{**begin, "data":{**begin["data"], "span_id":True}}, end],
            [{**begin, "data":{**begin["data"], "name":""}}, end],
            [{**begin, "data":{"span_id":1, "name":"worker_apply", "input_sequence":0}}, end],
            [{**begin, "elapsed_us":-1}, end],
            [begin, end, *self.span_records(1, name="engine_step")],
        ]
        for records in invalid:
            with self.subTest(records=records):
                report = DRIVER.summarize_spans(records + self.span_records(20, "native_frame_work"))
                self.assertFalse(report["valid"])
                self.assertTrue(report["errors"])
                for phase in report["phases"].values():
                    self.assertFalse(phase["valid"])
                    self.assertEqual(phase["wall_us"], {"samples":0})

    def test_disabled_and_legacy_profiling_do_not_claim_phase_evidence(self):
        for records in [[], [{"kind":"session", "data":{"profiling_enabled":False}}],
                [{"kind":"scene_prepare", "data":{"frame_id":1, "cpu_us":4}}]]:
            with self.subTest(records=records):
                report = DRIVER.summarize_spans(records)
                self.assertFalse(report["available"])
                self.assertIsNone(report["valid"])
                self.assertEqual(report["phases"], {})

    def test_report_rejects_incomplete_spans_and_traces_without_graceful_end(self):
        for spans, ended, passed in [(self.span_records(), True, True),
                (self.span_records()[:1], True, False), (self.span_records(), False, False)]:
            with self.subTest(spans=spans, ended=ended), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "telemetry").mkdir()
                session = DRIVER.Session(SimpleNamespace(output=root, timeout=1), None)
                records = [frame_record(1),
                    {"kind":"scene_prepare", "data":{"frame_id":1, "cpu_us":1}},
                    {"kind":"scene_encode", "data":{"frame_id":1, "cpu_us":1}}, *spans]
                if ended:
                    records.append({"kind":"session_end", "data":{}})
                append_records(session, records)
                report = session.report(True, None)
                self.assertEqual(report["passed"], passed)
                self.assertEqual(report["phase_spans"]["valid"], passed)
                self.assertEqual(report["phase_spans"]["phases"]["worker_apply"]["wall_us"]["samples"], int(passed))

    def test_plan_requires_one_named_operation(self):
        for step in [{"wait":[]}, {"name":"bad","wait":[],"capture":"image"},
                     {"name":"bad","capture":"../escape"}]:
            with self.subTest(step=step), self.assertRaises(ValueError):
                DRIVER.validate_plan({"schema":1,"steps":[step]})
        DRIVER.validate_plan({"schema":1,"steps":[
            {"name":"ready","wait":[{"path":"tick","op":"ge","value":0}]},
            {"name":"initial","remember":"initial"},
            {"name":"camera","input":{"kind":"key","code":124}},
            {"name":"moved","wait":[{"path":"camera","op":"ne","ref":"initial.camera"}]},
        ]})

    def test_plan_rejects_ambiguous_or_unbounded_operations(self):
        invalid_steps = [
            [{"name":"bad","wait":[]}],
            [{"name":"bad","wait":[{"path":"tick","op":"eq"}]}],
            [{"name":"bad","wait":[{"path":"tick","op":"eq","ref":"unknown.tick"}]}],
            [{"name":"bad","remember":"with.dot"}],
            [{"name":"bad","input":{"kind":"close"}}],
            [{"name":"bad","input":{"kind":"key","code":True}}],
            [{"name":"bad","input":{"kind":"key","code":128}}],
            [{"name":"bad","input":{"kind":"click","point":[0,float("inf")]}}],
            [{"name":"bad","input":{"kind":"click","text":"Invisible"}}],
            [{"name":"same","capture":"one"},{"name":"same","capture":"two"}],
            [{"name":"one","capture":"same"},{"name":"two","capture":"same"}],
            [{"name":"one","remember":"same"},{"name":"two","remember":"same"}],
        ]
        for steps in invalid_steps:
            with self.subTest(steps=steps), self.assertRaises(ValueError):
                DRIVER.validate_plan({"schema":1,"steps":steps})

    def test_reader_retries_partial_json_records(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events.jsonl"
            record = json.dumps({"kind":"frame","data":{"frame_id":1,
                "state":{"state_id":1,"value":{"tick":0}},
                "fence":0,"draw_started_unix_ns":1,"input_sequence":0,"cursor":None,
                "widget_draw_cpu_us":3,"draw_interval_us":None}})
            path.write_text(record[:10])
            reader = DRIVER.Observations(path)
            self.assertIsNone(reader.frame)
            self.assertEqual(reader.offset, 0)
            self.assertTrue(reader.partial_record)
            path.write_text(record + "\n")
            self.assertEqual(reader.frame["frame_id"], 1)
            self.assertEqual(len(reader.frames), 1)
            self.assertFalse(reader.partial_record)
            self.assertEqual(reader.frame["frame_id"], 1)
            self.assertEqual(len(reader.frames), 1)

    def test_reader_bounds_each_ingestion_to_the_available_file_snapshot(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events.jsonl"
            path.write_text(json.dumps(frame_record(1)) + "\n")
            reader = DRIVER.Observations(path)
            original_loads = DRIVER.json.loads
            def loads(line):
                record = original_loads(line)
                with path.open("a") as stream:
                    stream.write(json.dumps(frame_record(record["data"]["frame_id"] + 1)) + "\n")
                return record
            with mock.patch.object(DRIVER.json, "loads", side_effect=loads):
                reader.read()
            self.assertEqual(len(reader.frames), 1)
            reader.read()
            self.assertEqual(len(reader.frames), 2)

    def test_reader_rejects_incomplete_telemetry(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "events.jsonl"
            path.write_text(json.dumps({"kind":"frame","dropped_records":1,"data":{}})+"\n")
            with self.assertRaisesRegex(RuntimeError, "lost events"):
                DRIVER.Observations(path).read()

    def test_output_rejects_public_paths_and_symlink_escape(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / "cache"
            cache.mkdir()
            with mock.patch.object(DRIVER, "REPOSITORY", root):
                self.assertEqual(DRIVER.private_output(cache / "run"), cache / "run")
                with self.assertRaises((ValueError, FileNotFoundError)):
                    DRIVER.private_output(root / "public-recording")
                with self.assertRaises(ValueError):
                    DRIVER.private_output(cache / ".." / "run")
                (cache / "outside").symlink_to(Path(tempfile.gettempdir()), target_is_directory=True)
                with self.assertRaises(ValueError):
                    DRIVER.private_output(cache / "outside" / "run")

    def test_report_preserves_failure_when_telemetry_is_corrupt(self):
        for contents in ['{"kind":', 'not json\n',
                         json.dumps({"kind":"frame","data":{}})+"\n",
                         json.dumps({"kind":"host_error","data":{"error":"worker failed"}})+"\n"]:
            with self.subTest(contents=contents), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / "telemetry").mkdir()
                (root / "telemetry/events.jsonl").write_text(contents)
                session = DRIVER.Session(SimpleNamespace(output=root,timeout=1), None)
                report = session.report(True, None)
                self.assertFalse(report["passed"])
                self.assertIn("elemetry", report["error"])

    def test_report_excludes_loading_and_does_not_claim_gpu_or_fps(self):
        with tempfile.TemporaryDirectory() as directory:
            session = DRIVER.Session(SimpleNamespace(output=Path(directory),timeout=1), None)
            session.observations.frames = [
                {"frame_id":1,"state":{"value":{"tick":None}},"widget_draw_cpu_us":999,"draw_interval_us":999},
                {"frame_id":2,"state":{"value":{"tick":5}},"widget_draw_cpu_us":10,"draw_interval_us":888},
                {"frame_id":3,"state":{"value":{"tick":5}},"widget_draw_cpu_us":20,"draw_interval_us":15},
            ]
            session.observations.records = [
                {"kind":"scene_prepare","data":{"frame_id":1,"cpu_us":999}},
                {"kind":"scene_prepare","data":{"frame_id":2,"cpu_us":12}},
                {"kind":"scene_encode","data":{"frame_id":2,"cpu_us":9}},
                {"kind":"session_end","data":{}},
            ]
            report = session.report(True, None)
            self.assertTrue(report["passed"])
            self.assertEqual(report["ready_frames"], 2)
            self.assertEqual(report["loading_frames_excluded"], 1)
            self.assertEqual(report["metrics"]["widget_draw_cpu_us"]["worst"], 20)
            self.assertEqual(report["metrics"]["ui_draw_interval_us"]["worst"], 15)
            self.assertEqual(report["metrics"]["scene_prepare_cpu_us"]["worst"], 12)
            self.assertIsNone(report["gpu_duration_us"])
            self.assertIsNone(report["presented_fps"])
            session.observations.records = []
            self.assertFalse(session.report(True, None)["passed"])

    def test_capture_rejects_running_state_without_calling_native_driver(self):
        with tempfile.TemporaryDirectory() as directory:
            session = DRIVER.Session(SimpleNamespace(output=Path(directory),timeout=1), None)
            session.fence = mock.Mock(return_value={"state":{"value":{"playing":True,"pending":False}}})
            session.native = mock.Mock()
            with self.assertRaisesRegex(RuntimeError, "paused"):
                session.step({"name":"bad capture","capture":"image"})
            session.native.assert_not_called()

    def test_capture_rejects_changed_state_and_records_only_stable_ranges(self):
        with tempfile.TemporaryDirectory() as directory:
            session = DRIVER.Session(SimpleNamespace(output=Path(directory),timeout=1), None)
            before = {"frame_id":1,"fence":1,"input_sequence":0,"cursor":None,
                      "state":{"state_id":7,"value":{"playing":False,"pending":False}}}
            after = {"frame_id":3,"fence":2,"draw_started_unix_ns":2**63,"input_sequence":0,"cursor":None,
                     "state":{"state_id":8,"value":{"playing":False,"pending":False}}}
            session.fence = mock.Mock(side_effect=[before,after])
            session.native = mock.Mock()
            with self.assertRaisesRegex(RuntimeError, "ambiguous"):
                session.step({"name":"unstable","capture":"image"})
            self.assertFalse(session.journal)
            after["state"]["state_id"] = 7
            session.fence = mock.Mock(side_effect=[before,after])
            session.step({"name":"stable","capture":"image"})
            self.assertEqual(session.journal[0]["frame_id_range"], [1,3])
            self.assertEqual(session.journal[0]["observer_fence_range"], [1,2])
            session.journal = []
            after["input_sequence"] = 1
            session.fence = mock.Mock(side_effect=[before,after])
            with self.assertRaisesRegex(RuntimeError,"ambiguous"):
                session.step({"name":"local widget state changed","capture":"image"})

    def test_capture_rejects_a_queued_pre_capture_draw(self):
        with tempfile.TemporaryDirectory() as directory:
            session = DRIVER.Session(SimpleNamespace(output=Path(directory),timeout=1), None)
            before = {"frame_id":1,"fence":1,"state":{"state_id":7,"value":{"playing":False,"pending":False}}}
            queued = {"frame_id":3,"fence":2,"draw_started_unix_ns":40,"state":before["state"]}
            session.fence = mock.Mock(side_effect=[before,queued])
            session.native = mock.Mock()
            with mock.patch.object(DRIVER.time, "time_ns", return_value=42), \
                    self.assertRaisesRegex(RuntimeError, "after capture"):
                session.step({"name":"stale closing draw","capture":"image"})

    def test_remember_and_wait_after_capture_use_only_capture_fenced_draws(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            before = frame_record(1, phase="before", playing=False, pending=False)["data"]
            after = frame_record(3, fence=2, draw_started_unix_ns=2_000_000_000,
                                 phase="captured", playing=False, pending=False)["data"]
            after["state"]["state_id"] = before["state"]["state_id"]
            session.fence = mock.Mock(side_effect=[before,after])
            session.native = mock.Mock()
            session.step({"name":"capture", "capture":"image"})
            remembered = session.step({"name":"capture state", "remember":"capture"})
            self.assertEqual(remembered["frame_id"], 3)
            append_records(session, [frame_record(1, phase="before"),
                                     frame_record(2, phase="before"), {"kind":"frame", "data":after}])
            self.assertEqual(session.step({"name":"fresh capture", "wait":[{"path":"phase", "op":"eq", "value":"captured"}]})["frame_id"], 3)
            with self.assertRaises(TimeoutError):
                session.step({"name":"stale capture", "wait":[{"path":"phase", "op":"eq", "value":"before"}]})

    def test_fence_requires_a_new_draw_to_observe_its_token(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "telemetry").mkdir()
            session = DRIVER.Session(SimpleNamespace(output=root,timeout=1), None)
            previous = {"fence":0,"draw_started_unix_ns":1,"state":{"value":{"tick":0}}}
            current = {"fence":1,"draw_started_unix_ns":2,"state":{"value":{"tick":0}}}
            session.native = mock.Mock(return_value={})
            def wait(predicate):
                if session.fence_id:
                    self.assertFalse(predicate(previous))
                self.assertTrue(predicate(current))
                return current
            session.wait = wait
            self.assertEqual(session.fence(), current)
            self.assertEqual((root / "telemetry/observer-fence").read_bytes(), (1).to_bytes(8,"little"))

    def test_input_receipts_require_matching_native_events_in_order(self):
        with tempfile.TemporaryDirectory() as directory:
            session = DRIVER.Session(SimpleNamespace(output=Path(directory),timeout=1), None)
            action = {"kind":"key","code":124}
            session.observations.records = [
                {"kind":"input","data":{"fence":1,"received_unix_ns":1,"input":{"kind":"key_pressed","physical_key":"Code(ArrowRight)"}}},
                {"kind":"input","data":{"fence":1,"received_unix_ns":1,"input":{"kind":"key_released","physical_key":"Code(ArrowRight)"}}},
            ]
            self.assertEqual(session.receipts(action,1), [0,1])
            self.assertIsNone(session.receipts(action,2))
            self.assertIsNone(session.receipts({"kind":"key","code":123},1))
            session.observations.records.pop()
            self.assertIsNone(session.receipts(action,1))
            session.observations.records = [
                {"kind":"input","data":{"fence":2,"received_unix_ns":1,"input":{"kind":"pointer_moved","point":[10,20]}}},
                {"kind":"input","data":{"fence":2,"received_unix_ns":1,"input":{"kind":"button_pressed","button":"Left"}}},
                {"kind":"input","data":{"fence":2,"received_unix_ns":1,"input":{"kind":"button_released","button":"Left"}}},
            ]
            self.assertEqual(session.receipts({"kind":"click","point":[10,20]},2), [0,1,2])
            self.assertIsNone(session.receipts({"kind":"click","point":[12,20]},2))

    def test_receipt_lookup_skips_history_and_preserves_absolute_evidence_indices(self):
        class HistoricalRecord(dict):
            def __getitem__(self, key):
                raise AssertionError("Receipt lookup revisited historical telemetry")
        with tempfile.TemporaryDirectory() as directory:
            session = DRIVER.Session(SimpleNamespace(output=Path(directory), timeout=1), None)
            session.observations.records = [HistoricalRecord(), HistoricalRecord(),
                {"kind":"input", "data":{"fence":1, "received_unix_ns":2,
                    "input":{"kind":"key_pressed", "physical_key":"Code(ArrowRight)"}}},
                {"kind":"input", "data":{"fence":1, "received_unix_ns":3,
                    "input":{"kind":"key_released", "physical_key":"Code(ArrowRight)"}}}]
            action = {"kind":"key", "code":124}
            self.assertEqual(session.receipts(action, 1, after_unix_ns=1, start_record=2), [2, 3])
            self.assertIsNone(session.receipts(action, 1, after_unix_ns=2, start_record=2))
            self.assertIsNone(session.receipts(action, 1, start_record=3))

    def test_input_receipt_lookup_starts_after_its_fenced_observation(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory) as (session, clock):
            session.receipts = mock.Mock(wraps=session.receipts)
            self.native_race(session, clock)
            self.assertTrue(session.receipts.call_args_list)
            self.assertTrue(all(call.args[3] == 2 for call in session.receipts.call_args_list))

    def test_unrelated_redraw_cannot_acknowledge_native_input(self):
        with tempfile.TemporaryDirectory() as directory:
            session = DRIVER.Session(SimpleNamespace(output=Path(directory),timeout=1), None)
            before = {"frame_id":1,"fence":1,"state":{"value":{"display_size":[100,100]}}}
            session.fence = mock.Mock(return_value=before)
            session.native = mock.Mock()
            def wait(predicate):
                self.assertFalse(predicate({"frame_id":2}))
                raise TimeoutError("no matching input receipt")
            session.wait = wait
            with self.assertRaisesRegex(TimeoutError, "receipt"):
                session.step({"name":"undelivered","input":{"kind":"key","code":124}})
            self.assertFalse(session.journal)

    def test_error_in_an_earlier_frame_is_latched_through_reporting(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "telemetry").mkdir()
            records = []
            for identifier, error in [(1,"transient failure"),(2,None)]:
                records.append({"kind":"frame","data":{"frame_id":identifier,"fence":0,
                    "input_sequence":0,"cursor":None,
                    "draw_started_unix_ns":identifier,"widget_draw_cpu_us":1,"draw_interval_us":1,
                    "state":{"state_id":identifier,"value":{"tick":0,"error":error,"menu_error":None}}}})
            (root / "telemetry/events.jsonl").write_text("".join(json.dumps(record)+"\n" for record in records))
            session = DRIVER.Session(SimpleNamespace(output=root,timeout=1), None)
            with self.assertRaisesRegex(RuntimeError,"transient failure"):
                session.observations.read()
            report = session.report(True,None)
            self.assertFalse(report["passed"])
            self.assertIn("transient failure", report["error"])

    def test_output_rejects_a_cache_root_symlink_into_public_tree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            public = root / "tracked"
            public.mkdir()
            (root / "cache").symlink_to(public, target_is_directory=True)
            with mock.patch.object(DRIVER,"REPOSITORY",root), self.assertRaisesRegex(ValueError,"cache root"):
                DRIVER.private_output(root / "cache/run")

    def check_permission_denial(self, permissions):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "cache").mkdir()
            for name in ("binary", "helper", "save"):
                (root / name).write_bytes(b"synthetic test input")
            (root / "plan").write_text(json.dumps({"schema":1,"steps":[
                {"name":"ready","wait":[{"path":"tick","op":"ge","value":0}]}
            ]}))
            output = root / "cache/run"
            arguments = ["window_session.py", "--binary", str(root / "binary"),
                "--native-driver", str(root / "helper"), "--load", str(root / "save"),
                "--root", str(root), "--plan", str(root / "plan"), "--output", str(output)]
            denied = json.dumps(permissions)
            with mock.patch.object(DRIVER, "REPOSITORY", root), mock.patch.object(sys, "argv", arguments), \
                    mock.patch.object(DRIVER.subprocess, "check_output", return_value=denied), \
                    mock.patch.object(DRIVER.subprocess, "Popen") as launch, \
                    mock.patch.object(sys, "stderr"):
                with self.assertRaises(SystemExit) as failure:
                    DRIVER.main()
                self.assertEqual(failure.exception.code, 2)
                launch.assert_not_called()
            report = json.loads((output / "result.json").read_text())
            self.assertFalse(report["passed"])
            self.assertIn("no application was launched", report["error"])

    def test_permission_denial_never_launches_an_application(self):
        permissions = {"event_post_allowed":True,"screen_capture_allowed":True,"accessibility_trusted":True}
        for name in permissions:
            with self.subTest(permission=name):
                self.check_permission_denial({**permissions, name:False})
        self.check_permission_denial({"event_post_allowed":True,"screen_capture_allowed":True})

    def test_process_lifecycle_brackets_launch_through_exit_before_report_analysis(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "cache").mkdir()
            for name in ("binary", "helper"):
                (root / name).write_bytes(b"synthetic unexecuted input")
            (root / "save").write_text(json.dumps({"source":"synthetic-house.iff"}))
            (root / "plan").write_text(json.dumps({"schema":1,"steps":[
                {"name":"ready","wait":[{"path":"tick","op":"ge","value":0}]}
            ]}))
            output = root / "cache/run"
            arguments = ["window_session.py", "--binary", str(root / "binary"),
                "--native-driver", str(root / "helper"), "--load", str(root / "save"),
                "--root", str(root), "--plan", str(root / "plan"), "--output", str(output)]
            clock = {"unix":10, "host":100}
            process = mock.Mock(pid=123, returncode=0)
            process.poll.return_value = 0
            def launch(*arguments, **options):
                clock.update(unix=20, host=200)
                return process
            def exited(*arguments, **options):
                clock.update(unix=30, host=300)
                return 0
            def analyzed(*arguments):
                clock.update(unix=1000, host=10000)
                return {"passed":True, "error":None}
            process.wait.side_effect = exited
            session = mock.Mock()
            session.report.side_effect = analyzed
            permissions = json.dumps({"event_post_allowed":True,"screen_capture_allowed":True,
                                      "accessibility_trusted":True})
            with mock.patch.object(DRIVER, "REPOSITORY", root), mock.patch.object(sys, "argv", arguments), \
                    mock.patch.object(DRIVER.subprocess, "check_output", return_value=permissions), \
                    mock.patch.object(DRIVER.subprocess, "Popen", side_effect=launch), \
                    mock.patch.object(DRIVER, "Session", return_value=session), \
                    mock.patch.object(DRIVER.time, "time_ns", side_effect=lambda:clock["unix"]), \
                    mock.patch.object(DRIVER, "host_time_ns", side_effect=lambda:clock["host"]), \
                    mock.patch("builtins.print"):
                self.assertEqual(DRIVER.main(), 0)
            report = json.loads((output / "result.json").read_text())
            self.assertEqual(report["process_lifecycle"], {"started_unix_ns":10, "finished_unix_ns":30,
                "started_host_ns":100, "finished_host_ns":300, "host_clock":DRIVER.HOST_CLOCK})
            session.native.assert_called_once_with("close")
            self.assertEqual(process.wait.call_args_list, [mock.call(timeout=10), mock.call(timeout=5)])

    def test_native_journal_serializes_capture_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            session = DRIVER.Session(SimpleNamespace(output=root,timeout=1,native_driver=root / "helper"),
                                     SimpleNamespace(pid=123))
            result = SimpleNamespace(returncode=0,stdout='{}',stderr='')
            with mock.patch.object(DRIVER,"native_process",return_value=result):
                session.native("capture","--path",root / "image.png")
            journal = json.loads(json.dumps(session.journal))
            self.assertEqual(journal[0]["native_command"][-1], str(root / "image.png"))

    def test_native_error_keeps_stdout_and_uses_remaining_deadline(self):
        with tempfile.TemporaryDirectory() as directory, synthetic_session(directory, timeout=.5) as (session, clock):
            session.args.native_driver = Path(directory) / "helper"
            session.process = SimpleNamespace(pid=123)
            clock.now = .2
            result = SimpleNamespace(returncode=1, stdout='{"native_error":true}', stderr="denied")
            with mock.patch.object(DRIVER, "native_process", return_value=result) as native:
                with self.assertRaisesRegex(RuntimeError, "denied"):
                    session.native("window")
            self.assertAlmostEqual(native.call_args.kwargs["timeout"], .3)
            self.assertEqual(session.journal[0]["stdout"], '{"native_error":true}')
            self.assertEqual(session.journal[0]["stderr"], "denied")

    def test_native_timeout_retains_partial_stdout_and_stderr(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            session = DRIVER.Session(SimpleNamespace(output=root, timeout=1, native_driver=root / "helper"),
                                     SimpleNamespace(pid=123))
            failure = DRIVER.subprocess.TimeoutExpired(["helper"], 1, output=b"partial native output", stderr=b"stalled")
            with mock.patch.object(DRIVER, "native_process", side_effect=failure):
                with self.assertRaises(DRIVER.subprocess.TimeoutExpired):
                    session.native("window")
            self.assertTrue(session.journal[0]["timed_out"])
            self.assertIsNone(session.journal[0]["exit_code"])
            self.assertEqual(session.journal[0]["stdout"], "partial native output")
            self.assertEqual(session.journal[0]["stderr"], "stalled")

    def test_native_timeout_reserves_termination_grace_before_force_kill(self):
        for cooperative in [True, False]:
            with self.subTest(cooperative=cooperative):
                failure = DRIVER.subprocess.TimeoutExpired(["helper"], .1, output=b"partial")
                process = mock.Mock(returncode=-15)
                process.communicate.side_effect = ([failure, ("cleanup output", "")]
                    if cooperative else [failure, failure, ("cleanup output", "")])
                with mock.patch.object(DRIVER.subprocess, "Popen", return_value=process):
                    with self.assertRaises(DRIVER.subprocess.TimeoutExpired) as raised:
                        DRIVER.native_process(["helper"], .1)
                process.terminate.assert_called_once()
                self.assertEqual(process.kill.call_count, int(not cooperative))
                self.assertEqual(process.communicate.call_args_list[1], mock.call(timeout=1))
                self.assertEqual(raised.exception.stdout, "cleanup output")

    def test_native_focus_uses_authorized_accessibility_and_still_verifies_foreground(self):
        source = Path(__file__).with_name("window_session_macos.swift").read_text()
        focus = source.split("func focus(", 1)[1].split("\n}\n", 1)[0]
        self.assertIn("application.activate(options: [])", focus)
        self.assertIn("processIdentifier != pid && AXIsProcessTrusted()", focus)
        self.assertIn("AXUIElementCreateApplication(pid)", focus)
        self.assertIn("AXUIElementIsAttributeSettable", focus)
        self.assertIn("settable.boolValue", focus)
        self.assertIn("AXUIElementSetAttributeValue(accessible, kAXFrontmostAttribute as CFString, kCFBooleanTrue)", focus)
        self.assertIn("Date().addingTimeInterval(2)", focus)
        self.assertIn("guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid", focus)
        self.assertIn("Refusing input: target application is not frontmost", focus)
        self.assertNotIn("CGEvent", focus)

    def test_native_cleanup_matches_original_input_posting_routes(self):
        source = Path(__file__).with_name("window_session_macos.swift").read_text()
        release = source.split("func postInputRelease(", 1)[1].split("\n}\n", 1)[0]
        self.assertIn("if release.type == .leftMouseUp", release)
        self.assertIn("release.post(tap: .cgSessionEventTap)", release)
        self.assertIn("else if release.type == .keyUp", release)
        self.assertIn("release.postToPid(pid)", release)
        self.assertIn("postInputRelease(release, pid: pid)", source.split('if operation == "release"', 1)[1])
        self.assertIn("if let heldRelease { postInputRelease(heldRelease, pid: pid) }", source)
        self.assertNotIn("heldRelease?.postToPid", source)
        pointer = source.split("func postPointer(", 1)[1].split("\n}\n", 1)[0]
        self.assertIn("event.post(tap: .cgSessionEventTap)", pointer)
        self.assertIn("target application lost focus", pointer)
        self.assertIn("Target window", source)
        self.assertIn("AXUIElementCopyElementAtPosition", pointer)

    def test_failed_or_timed_out_input_attempts_targeted_release_and_stays_failed(self):
        for kind in ["key", "click", "close"]:
            for timed_out in [False, True]:
                with self.subTest(kind=kind, timed_out=timed_out), tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    session = DRIVER.Session(SimpleNamespace(output=root, timeout=1, native_driver=root / "helper"),
                                             SimpleNamespace(pid=123))
                    failure = (DRIVER.subprocess.TimeoutExpired(["helper"], .1) if timed_out else
                               SimpleNamespace(returncode=1, stdout="", stderr="failure after down"))
                    released = SimpleNamespace(returncode=0, stdout='{"release_attempted":true}', stderr="")
                    arguments = (kind, "--code", "49") if kind == "key" else (kind,)
                    with mock.patch.object(DRIVER, "native_process", side_effect=[failure, released]) as native:
                        with self.assertRaises((RuntimeError, DRIVER.subprocess.TimeoutExpired)):
                            session.native(*arguments)
                    cleanup = native.call_args_list[1]
                    self.assertEqual(cleanup.args[0][:4], [str(root / "helper"), "release", "--pid", "123"])
                    self.assertEqual(cleanup.kwargs["timeout"], 2)
                    self.assertEqual(cleanup.args[0][4:], ["--code", "49"] if kind == "key" else [])
                    self.assertTrue(session.journal[-1]["native_cleanup"])


if __name__ == "__main__":
    unittest.main()
