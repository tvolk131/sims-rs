use super::*;
use crate::native_application::InitializedSection;

fn input() -> Incoming {
    let caller = Owner {
        token: 0x2000_0000,
        data: (0..384).map(|at| (at as u8).wrapping_mul(17)).collect(),
    };
    let b = caller.token;
    Incoming {
        origin: HeapOrigin::new(),
        caller,
        frames: Frames {
            factory: b + 32,
            derived: b + 80,
            base: b + 128,
            refcount: b + 176,
            child: b + 224,
            base72: b + 288,
        },
    }
}
struct Model {
    parents: usize,
    trees: usize,
    sections: usize,
    null: bool,
}
impl Model {
    fn new() -> Self {
        Self {
            parents: 0,
            trees: 0,
            sections: 0,
            null: false,
        }
    }
}
impl Host for Model {
    fn allocate_provider(&mut self, request: &Request) -> Result<Allocated, Stop> {
        request.validate()?;
        self.parents += 1;
        Ok(if self.null {
            Allocated::Null
        } else {
            Allocated::Bytes {
                token: 0x1000_0000,
                data: vec![0xa7; 1240],
            }
        })
    }
    fn allocate_tree(&mut self, request: &TreeRequest) -> Result<Allocated, Stop> {
        request.validate()?;
        let token = 0x3000_0000 + self.trees as u64 * 0x100;
        self.trees += 1;
        Ok(Allocated::Bytes {
            token,
            data: vec![0x6c; 24],
        })
    }
    fn prepare_base72(&mut self, c: &Checkpoint) -> Result<(), Stop> {
        c.validate()
    }
    fn initialize_base72(&mut self, c: &Base72Call) -> Result<InitializedSection, Stop> {
        c.validate()?;
        self.sections += 1;
        InitializedSection::from_debug_info(
            &c.request().section_request(),
            Some(&Owner {
                token: 0x4000_0000,
                data: vec![0x39; 32],
            }),
            42,
        )
    }
    fn acknowledge_base72(&mut self, c: &Base72Call, p: &base72::Prefix) -> Result<(), Stop> {
        c.validate()?;
        p.validate()
    }
    fn synchronize_checkpoint(&mut self, c: &Checkpoint) -> Result<(), Stop> {
        c.validate()
    }
}
fn finish(c: &mut Construction, h: &mut Model) {
    for _ in 0..512 {
        if c.step(h).unwrap() == Some(Cut::FirstDerivedChildReturned) {
            return;
        }
    }
    panic!("system prefix failed to reach source cut");
}
fn word(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
}

#[test]
fn system_factory_source_prefix_preserves_actual_tree_flags_padding_and_frame_aliases() {
    let incoming = input();
    let original = incoming.caller.clone();
    let frames = incoming.frames;
    let mut c = Construction::new(incoming).unwrap();
    let mut h = Model::new();
    finish(&mut c, &mut h);
    assert_eq!((h.parents, h.trees, h.sections), (1, 2, 1));
    let p = c.checkpoint().unwrap();
    p.validate().unwrap();
    assert_eq!(
        word(&p.parent().data, 24),
        p.tree(TreeSite::Refcount).unwrap().token as u32
    );
    assert_eq!(
        word(&p.parent().data, 116),
        p.tree(TreeSite::Derived).unwrap().token as u32
    );
    assert_eq!(
        p.parent().data[32],
        original.data[(frames.refcount - 2 - original.token) as usize]
    );
    assert_eq!(
        p.parent().data[124],
        original.data[(frames.child - 14 - original.token) as usize]
    );
    assert_eq!(word(&p.parent().data, 36), 0x6737bc);
    assert_eq!(word(&p.parent().data, 108), 0x673768);
    assert_eq!(
        word(
            &p.caller().data,
            (frames.factory - 16 - original.token) as usize
        ),
        p.parent().token as u32
    );
    assert_eq!(
        word(
            &p.caller().data,
            (frames.base72 + 8 - original.token) as usize
        ),
        0
    );
    for site in [TreeSite::Refcount, TreeSite::Derived] {
        let tree = p.tree(site).unwrap();
        assert_eq!(tree.data[0], 0);
        assert_eq!(&tree.data[1..4], &[0x6c; 3]);
        assert_eq!(&tree.data[16..], &[0x6c; 8]);
        assert_eq!(word(&tree.data, 8), tree.token as u32);
    }
    let mut caller_mask = vec![false; original.data.len()];
    for write in p
        .writes()
        .iter()
        .filter(|write| write.owner == original.token)
    {
        caller_mask[write.offset..write.offset + write.after.len()].fill(true);
    }
    for (at, touched) in caller_mask.into_iter().enumerate() {
        if !touched {
            assert_eq!(p.caller().data[at], original.data[at]);
        }
    }
    assert!(matches!(c.step(&mut h), Ok(None))); // actual parent EH-byte2
    assert!(matches!(
        c.pending_call(),
        Some(Call::Collection {
            instruction: 0x5d5879
        })
    ));
    assert!(matches!(c.step(&mut h), Err(Stop::Unavailable(_))));
    assert!(matches!(c.step(&mut h), Err(Stop::Unavailable(_))));
    assert_eq!((h.parents, h.trees, h.sections), (1, 2, 1));
}
#[test]
fn system_factory_rejects_overlapping_or_absent_frame_operands_without_losing_incoming() {
    for bad in [0, 1] {
        let mut i = input();
        if bad == 0 {
            i.frames.base72 = i.frames.child;
        } else {
            i.frames.factory = i.caller.token + 8;
        }
        let old = i.clone();
        let error = Construction::new(i).unwrap_err();
        assert_eq!(error.incoming, old);
    }
}
#[test]
fn system_factory_null_allocation_is_retained_unavailable_and_never_reissued() {
    let mut c = Construction::new(input()).unwrap();
    let mut h = Model::new();
    h.null = true;
    assert_eq!(c.step(&mut h).unwrap(), None);
    assert!(c.step(&mut h).is_err());
    assert!(c.step(&mut h).is_err());
    assert_eq!(h.parents, 1);
    assert_eq!((h.trees, h.sections), (0, 0));
    assert!(c.checkpoint().is_err());
}
#[test]
fn system_factory_source_receipts_reject_parent_frame_and_future_resource_tampering() {
    let mut c = Construction::new(input()).unwrap();
    let mut h = Model::new();
    finish(&mut c, &mut h);
    let valid = c.checkpoint().unwrap();
    let mut p = valid.clone();
    p.images.parent.data[900] ^= 1;
    assert!(p.validate().is_err());
    let mut p = valid.clone();
    p.images.caller.data[0] ^= 1;
    assert!(p.validate().is_err());
    let mut p = valid.clone();
    p.images.writes[0].instruction ^= 1;
    assert!(p.validate().is_err());
    let mut p = valid;
    p.initial_trees[1] = None;
    assert!(p.validate().is_err());
    let first = HeapOrigin::new();
    let second = HeapOrigin::new();
    assert_eq!(first, first.clone());
    assert_ne!(first, second);
}
