//! Ordered source effects from5d5a20/5d9461/5d919a/5edd4d and4eeb9a.
use super::Cut;

#[derive(Debug, Clone, Copy)]
pub(super) enum Region {
    Application,
    Globals,
    Singleton,
    MutexSingleton,
    MutexWrapper,
    Tree,
}
pub(super) enum Op {
    CallerApplication,
    CallerStore {
        relative: i32,
        bytes: Vec<u8>,
        pc: u32,
    },
    Store {
        region: Region,
        offset: usize,
        bytes: Vec<u8>,
        pc: u32,
    },
    Memset {
        offset: usize,
        bytes: usize,
        pc: u32,
    },
    Section,
    Tree,
    TreePointer {
        region: Region,
        offset: usize,
        pc: u32,
    },
    StackByte {
        offset: usize,
        pc: u32,
    },
    String {
        offset: usize,
        copy_argument: bool,
        pc: u32,
    },
    Commander,
    Bind,
    CoInitialize,
    Publish,
    Version,
    MutexAllocate,
    MutexName,
    MutexPreparation,
    MutexZero,
    MutexVptr,
    MutexCut,
    MutexCreate,
    MutexPublish,
    MutexTemporaryDestroy,
    MutexReturnedCut,
    FpuRead,
    FpuInspect,
    FpuNormalize,
    FpuLoad,
    FpuOriginal,
    FpuChanged,
    Cut(Cut),
    Boundary,
}

pub(super) fn operations() -> Vec<Op> {
    let mut p = vec![
        Op::CallerApplication,
        Op::CallerStore {
            relative: -0x10,
            bytes: 0_u32.to_le_bytes().to_vec(),
            pc: 0x4eebb4,
        },
    ];
    let store = |p: &mut Vec<Op>, offset, value: u32, pc| {
        p.push(Op::Store {
            region: Region::Application,
            offset,
            bytes: value.to_le_bytes().to_vec(),
            pc,
        })
    };
    for (offset, value, pc) in [
        (0, 0x673840, 0x5d5a32),
        (12, 0, 0x5d5a38),
        (8, 0x673720, 0x5d5a3c),
        (4, 0x673714, 0x5d5a43),
        (8, 0x6736fc, 0x5d5a4a),
        (48, 0, 0x5d91a7),
        (52, 0, 0x5d91aa),
        (20, 0x673ab8, 0x5d91ad),
    ] {
        store(&mut p, offset, value, pc);
    }
    p.push(Op::Memset {
        offset: 24,
        bytes: 24,
        pc: 0x5d91b3,
    });
    p.push(Op::Section);
    store(&mut p, 48, 1, 0x5d91c2);
    store(&mut p, 16, 0x673858, 0x5d9485);
    // The actual inherited getter5d91f2 reads the just-written member+0x1c.
    store(&mut p, 56, 1, 0x5d9492);
    p.push(Op::Store {
        region: Region::Application,
        offset: 64,
        bytes: vec![0],
        pc: 0x5d5a62,
    });
    for (offset, value, pc) in [
        (68, 0, 0x5d5a69),
        (60, 0xffe17b80, 0x5d5a6d),
        (16, 0x673838, 0x5d5a70),
        (0, 0x673820, 0x5d5a76),
        (4, 0x673814, 0x5d5a7c),
        (8, 0x6737fc, 0x5d5a83),
        (72, 0x675218, 0x5edd6f),
        (80, 0, 0x55e376),
    ] {
        store(&mut p, offset, value, pc);
    }
    p.push(Op::Tree);
    store(&mut p, 84, 0, 0x55e37e);
    p.push(Op::TreePointer {
        region: Region::Application,
        offset: 80,
        pc: 0x55e387,
    });
    p.push(Op::StackByte {
        offset: 88,
        pc: 0x55e38b,
    });
    p.push(Op::Store {
        region: Region::Tree,
        offset: 0,
        bytes: vec![0],
        pc: 0x55e38e,
    });
    p.push(Op::Store {
        region: Region::Tree,
        offset: 4,
        bytes: 0_u32.to_le_bytes().to_vec(),
        pc: 0x55e393,
    });
    for (offset, pc) in [(8, 0x55e399), (12, 0x55e39e)] {
        p.push(Op::TreePointer {
            region: Region::Tree,
            offset,
            pc,
        });
    }
    store(&mut p, 76, 0x676a6c, 0x602ee2);
    p.push(Op::String {
        offset: 92,
        copy_argument: true,
        pc: 0x5edd85,
    });
    for (offset, value, pc) in [
        (0, 0x6751e8, 0x5edd8d),
        (4, 0x6751dc, 0x5edd93),
        (8, 0x6751c4, 0x5edd9a),
        (16, 0x6751bc, 0x5edda1),
        (72, 0x675174, 0x5edda8),
    ] {
        store(&mut p, offset, value, pc);
    }
    p.push(Op::Cut(Cut::Base104Returned));
    p.push(Op::CallerStore {
        relative: -4,
        bytes: 0_u32.to_le_bytes().to_vec(),
        pc: 0x4eebbf,
    });
    p.push(Op::Commander);
    p.push(Op::Cut(Cut::CommanderReturned));
    for (i, pc) in [
        0x4eebc9, 0x4eebcc, 0x4eebcf, 0x4eebd5, 0x4eebdb, 0x4eebe1, 0x4eebe7, 0x4eebed, 0x4eebf3,
        0x4eebf9,
    ]
    .into_iter()
    .enumerate()
    {
        store(&mut p, 120 + i * 4, 0, pc);
    }
    p.push(Op::CallerStore {
        relative: -4,
        bytes: vec![1],
        pc: 0x4eec05,
    });
    store(&mut p, 160, 0, 0x4eec09);
    for (i, (offset, pc)) in [
        (164, 0x4eec0f),
        (180, 0x4eec1e),
        (192, 0x4eec2d),
        (204, 0x4eec3c),
        (216, 0x4eec4b),
        (228, 0x4eec5a),
    ]
    .into_iter()
    .enumerate()
    {
        if i != 0 {
            p.push(Op::CallerStore {
                relative: -4,
                bytes: vec![(i + 1) as u8],
                pc: pc - 4,
            });
        }
        p.push(Op::String {
            offset,
            copy_argument: false,
            pc,
        });
    }
    p.push(Op::Cut(Cut::DefaultStringsReturned));
    for (offset, value, pc) in [
        (244, 0, 0x4eec5f),
        (245, 1, 0x4eec65),
        (246, 0, 0x4eec6c),
        (247, 0, 0x4eec72),
        (249, 0, 0x4eec78),
        (250, 0, 0x4eec7e),
        (251, 0, 0x4eec84),
        (252, 0, 0x4eec8a),
        (253, 0, 0x4eec90),
    ] {
        p.push(Op::Store {
            region: Region::Application,
            offset,
            bytes: vec![value],
            pc,
        });
    }
    for (offset, pc) in [(276, 0x4eec96), (296, 0x4eec9c), (300, 0x4eeca2)] {
        store(&mut p, offset, 0, pc);
    }
    p.push(Op::Store {
        region: Region::Application,
        offset: 306,
        bytes: vec![255; 2],
        pc: 0x4eeca8,
    });
    p.push(Op::Store {
        region: Region::Application,
        offset: 304,
        bytes: vec![0],
        pc: 0x4eecb0,
    });
    store(&mut p, 308, 0, 0x4eecb6);
    p.push(Op::Store {
        region: Region::Application,
        offset: 312,
        bytes: vec![0; 2],
        pc: 0x4eecbc,
    });
    p.push(Op::CallerStore {
        relative: -4,
        bytes: vec![7],
        pc: 0x4eecc4,
    });
    for (offset, pc) in [(320, 0x4eecc8), (321, 0x4eecce), (322, 0x4eecd4)] {
        p.push(Op::Store {
            region: Region::Application,
            offset,
            bytes: vec![0],
            pc,
        });
    }
    for (offset, value, pc) in [
        (0, 0x669884, 0x4eecda),
        (4, 0x669878, 0x4eece0),
        (8, 0x669860, 0x4eece7),
        (16, 0x669858, 0x4eecee),
        (72, 0x669810, 0x4eecf5),
    ] {
        store(&mut p, offset, value, pc);
    }
    p.push(Op::Bind);
    p.push(Op::Cut(Cut::DerivedDispatchBound));
    p.push(Op::Cut(Cut::ImportEntry));
    p.push(Op::CoInitialize);
    p.push(Op::Cut(Cut::ComReturned));
    p.push(Op::Publish);
    p.push(Op::Cut(Cut::Published));
    p.push(Op::Version);
    p.push(Op::Cut(Cut::VersionReturned));
    p.push(Op::MutexAllocate);
    p.push(Op::MutexName);
    p.push(Op::MutexPreparation);
    p.push(Op::MutexZero);
    p.push(Op::MutexVptr);
    p.push(Op::MutexCut);
    p.push(Op::MutexCreate);
    p.push(Op::MutexPublish);
    p.push(Op::MutexTemporaryDestroy);
    p.push(Op::MutexReturnedCut);
    p.push(Op::FpuRead);
    p.push(Op::FpuInspect);
    p.push(Op::FpuNormalize);
    p.push(Op::FpuLoad);
    p.push(Op::FpuOriginal);
    p.push(Op::FpuChanged);
    for (offset, bytes, pc) in [
        (260, vec![0], 0x4eee08),
        (176, 0_u32.to_le_bytes().to_vec(), 0x4eee0e),
        (261, vec![1], 0x4eee14),
        (264, 0_u32.to_le_bytes().to_vec(), 0x4eee1b),
    ] {
        p.push(Op::Store {
            region: Region::Application,
            offset,
            bytes,
            pc,
        });
    }
    p.push(Op::Cut(Cut::FpuReturned));
    p.push(Op::Boundary);
    p
}
