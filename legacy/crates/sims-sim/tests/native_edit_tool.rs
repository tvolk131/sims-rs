use sims_sim::{
    edit_history::{
        self, List, Store,
        command::{Method, Request},
        manager::Service as HistoryService,
    },
    object_catalog::{
        definition::Catalog,
        move_tool::{
            Groups, cleanup,
            dispatch::{Dispatch, Service},
            hover,
        },
    },
};

struct Host;
impl cleanup::View for Host {
    fn disposable(&self, _: u32) -> Result<bool, String> {
        Err("no held object".into())
    }
    fn script_id(&self, _: u32) -> Result<i16, String> {
        Err("no held object".into())
    }
    fn clear_payload_cost(&mut self, _: u32) -> Result<(), String> {
        Err("no held object".into())
    }
}
impl hover::View for Host {
    fn lookup(&self, _: i16) -> Result<Option<u32>, String> {
        Ok(None)
    }
    fn flags(&self, _: u32) -> Result<u32, String> {
        Err("no hovered object".into())
    }
}
fn advance(parent: &mut Dispatch, state: &mut cleanup::State) -> Option<Service> {
    parent
        .advance(
            state,
            &mut Host,
            cleanup::Inputs {
                registry: 1,
                window: 0,
                key_function: 0,
                control_state: 0,
            },
            &Groups::default(),
            &Catalog::default(),
            0,
        )
        .unwrap()
}

#[test]
fn tool_stop_and_release_run_concrete_children_before_parent_acknowledgement() {
    let mut store = Store::default();
    for id in [1, 2] {
        store.construct_command(id).unwrap();
        let c = store.command_mut(id).unwrap();
        c.state = 1;
        c.references = 1;
    }
    store.append(List::After(1), 2).unwrap();
    let mut state = cleanup::State::constructor_fields(10);
    state.payload = Some(1);
    let mut parent = Dispatch::cleanup(cleanup::Kind::Deactivate);
    for (request, methods) in [
        (
            cleanup::Request::StopPayload(1),
            vec![
                Request {
                    command: 2,
                    method: Method::Undo,
                },
                Request {
                    command: 2,
                    method: Method::Destroy,
                },
            ],
        ),
        (
            cleanup::Request::ReleasePayload(1),
            vec![Request {
                command: 1,
                method: Method::Destroy,
            }],
        ),
    ] {
        assert_eq!(
            advance(&mut parent, &mut state),
            Some(Service::External(request))
        );
        assert_eq!(state.payload, Some(1));
        let mut child = edit_history::tool::Call::new(request).unwrap().unwrap();
        let unchanged = parent.clone();
        assert!(child.finish(&mut parent).is_err());
        assert_eq!(parent, unchanged);
        for virtual_call in methods {
            let external = HistoryService::Virtual(virtual_call);
            assert_eq!(child.advance(&mut store).unwrap(), Some(external));
            (store, state, parent, child) = serde_json::from_slice(
                &serde_json::to_vec(&(&store, &state, &parent, &child)).unwrap(),
            )
            .unwrap();
            let unchanged = store.clone();
            assert_eq!(child.advance(&mut store).unwrap(), Some(external));
            assert_eq!(store, unchanged);
            assert!(child.finish(&mut parent).is_err());
            let saved = child.clone();
            assert!(child.acknowledge(HistoryService::Notify, 0).is_err());
            assert_eq!(child, saved);
            if virtual_call.method == Method::Destroy {
                // Concrete destruction is a supplied boundary here. Its caller
                // may not access this command after it returns.
                store.deallocate_command(virtual_call.command).unwrap();
            }
            child.acknowledge(external, 0).unwrap();
        }
        assert_eq!(child.advance(&mut store).unwrap(), None);
        let mut wrong_parent = Dispatch::cleanup(cleanup::Kind::Reset);
        let unchanged = wrong_parent.clone();
        assert!(child.finish(&mut wrong_parent).is_err());
        assert_eq!(wrong_parent, unchanged);
        child.finish(&mut parent).unwrap();
        assert!(child.finish(&mut parent).is_err());
    }
    assert_eq!(advance(&mut parent, &mut state), None);
    assert!(parent.complete() && state.payload.is_none());
    assert!(store.commands.is_empty());
    store.validate().unwrap();
    assert!(
        edit_history::tool::Call::new(cleanup::Request::DetachObject(3))
            .unwrap()
            .is_none()
    );
}
