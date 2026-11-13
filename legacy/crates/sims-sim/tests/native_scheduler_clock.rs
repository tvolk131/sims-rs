use sims_sim::clock::{ClockEvent, LotClock};
use sims_vm::scheduler::{Inputs, Pass, Registry, Reply, Request};

struct RegistryPair([u32; 2]);
impl Registry for RegistryPair {
    fn len(&self) -> usize {
        2
    }
    fn object(&self, slot: i32) -> Result<Option<u32>, String> {
        Ok(Some(slot as u32))
    }
    fn word(&self, slot: i32) -> Result<u32, String> {
        Ok(self.0[(slot - 1) as usize])
    }
    fn set_word(&mut self, slot: i32, word: u32) -> Result<(), String> {
        self.0[(slot - 1) as usize] = word;
        Ok(())
    }
    fn error(&self, _: u32) -> Result<i16, String> {
        Ok(0)
    }
}

/// Exercises the scheduler-to-clock boundary with explicit Tick callbacks.
/// It does not execute SimAntics or claim the frontend has a Buy mode host.
#[test]
fn paused_controller_passes_do_not_advance_calendar_and_resume_live_once() {
    let mut clock = LotClock {
        hour: 10,
        minute: 59,
        second: 58,
        day: 1,
        month: 1,
        year: 2000,
        period: 0,
        elapsed_days: 0,
        month_days: 30,
        ticks: 194316,
        statistics: [[0; 8]; 6],
    };
    let before = clock.clone();
    let mut registry = RegistryPair([0x1ffff, 0x3ffff]);
    let mut cursor = 0;
    for paused in [true, true, true, false] {
        let input = Inputs {
            paused: 0,
            mode: if paused { 2 } else { 0 },
            lot_number: 1,
            tick: clock.ticks,
        };
        let mut pass = Pass::begin(input, &registry, &mut cursor).unwrap();
        let mut visited = vec![];
        while let Some(request) = pass.advance(&mut registry, &mut cursor, input).unwrap() {
            let Request::Tick { object, tick } = request else {
                panic!("unexpected object error")
            };
            assert_eq!(tick, before.ticks);
            visited.push(object);
            let saved = serde_json::to_string(&(&clock, &pass, cursor)).unwrap();
            (clock, pass, cursor) = serde_json::from_str(&saved).unwrap();
            pass.acknowledge(&request, Reply::Tick(1)).unwrap();
        }
        assert_eq!(visited, if paused { vec![2] } else { vec![1, 2] });
        let events = clock.finish_tick(pass.result().unwrap());
        if paused {
            assert_eq!(clock, before);
            assert_eq!(events, vec![ClockEvent::WorldUpdate { partial: true }]);
        } else {
            assert_eq!(clock.ticks, before.ticks + 1);
            assert_eq!((clock.hour, clock.minute, clock.second), (11, 0, 0));
            assert_eq!(
                events,
                vec![
                    ClockEvent::WorldUpdate { partial: false },
                    ClockEvent::Signal {
                        kind: 0xdb,
                        argument: 0
                    }
                ]
            );
        }
    }
}
