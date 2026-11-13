use serde::Deserialize;
use sims_vm::ui_effect::widgets::{
    messages,
    timers::{self, dispatch},
};
#[derive(Deserialize, PartialEq, Debug)]
struct Timers {
    registry: timers::Registry,
    flags: dispatch::Flags,
}
#[derive(Deserialize)]
struct Post {
    message: u32,
    argument: u32,
    packed: u32,
    success: bool,
}
#[derive(Deserialize)]
struct Native {
    before: Timers,
    after: Timers,
    point: [u32; 2],
    clock: u32,
    special_owner: u32,
    members: Vec<u32>,
    posted: Post,
    messages_before: messages::Fields,
    messages_after: messages::Fields,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn actual_window_pointer_timer_and_empty_message_pass() -> Result<(), Box<dyn std::error::Error>> {
    let native: Native =
        serde_json::from_str(include_str!("fixtures/native-framework-window.json"))?;
    let mut state = native.before;
    let mut pass = dispatch::Pass::default();
    let mut kinds = Vec::new();
    while let Some(request) = pass.advance(&mut state.registry, &mut state.flags)? {
        let saved = serde_json::to_vec(&(&pass, &state.registry, state.flags))?;
        (pass, state.registry, state.flags) = serde_json::from_slice(&saved)?;
        assert_eq!(
            pass.advance(&mut state.registry, &mut state.flags)?,
            Some(request)
        );
        assert_eq!(
            serde_json::to_vec(&(&pass, &state.registry, state.flags))?,
            saved
        );
        let answer = match request {
            dispatch::Request::PointerPosition => {
                kinds.push("point");
                dispatch::Reply::Point(native.point)
            }
            dispatch::Request::PublishPointer {
                message,
                argument,
                packed,
            } => {
                kinds.push("post");
                assert_eq!(
                    (message, argument, packed),
                    (13, native.posted.argument, native.posted.packed)
                );
                assert_eq!(native.posted.message, 0x200);
                assert!(native.posted.success);
                dispatch::Reply::Done
            }
            dispatch::Request::Clock => {
                kinds.push("clock");
                dispatch::Reply::Clock(native.clock)
            }
            dispatch::Request::Registered(owner) => {
                kinds.push("membership");
                let registered = timers::registered(owner, native.special_owner, &native.members);
                assert!(!registered);
                dispatch::Reply::Registered(registered)
            }
            dispatch::Request::Dispatch { .. } => panic!("captured stale timer must not dispatch"),
        };
        pass.acknowledge(request, answer)?;
    }
    assert_eq!(kinds, ["point", "post", "clock", "membership"]);
    assert_eq!(state, native.after);
    let mut fields = native.messages_before;
    let mut pass = messages::Pass::default();
    assert_eq!(pass.advance(&mut fields)?, None);
    assert!(pass.complete());
    assert_eq!(fields, native.messages_after);
    Ok(())
}
