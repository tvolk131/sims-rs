//! Built-in, untextured materials selected by native 5d41db before trying an
//! image resource. Names are case-sensitive. These are material colors, not
//! invented replacement BMPs; the renderer's lighting remains a separate policy.
#[derive(Debug, Clone, PartialEq)]
pub struct Builtin {
    pub diffuse: [f32; 4],
    pub ambient: [f32; 4],
    pub specular: [f32; 4],
    pub emissive: [f32; 4],
    pub power: f32,
}

pub fn builtin(name: &str) -> Option<Builtin> {
    let f = f32::from_bits;
    let (diffuse, ambient) = match name {
        "red" => ([1., 0., 0.], [1., 0., 0.]),
        "blue" => ([0., 0., 1.], [0., 0., 1.]),
        "black" => ([0.; 3], [0.; 3]),
        "grey" | "gray" => ([0.5; 3], [0.5; 3]),
        "yellow" => ([1., 1., 0.], [1., 1., 0.]),
        "green" => ([0., 1., 0.], [0., 1., 0.]),
        "default" | "white" => ([1.; 3], [1.; 3]),
        "BeigeMatte" => ([0.6, 0.6, 0.1], [0.6, 0.6, 0.1]),
        "Jeans" => (
            [f(0x3ea6a6a7), f(0x3ef8f8f9), f(0x3f33b3b4)],
            [f(0x3e808081), f(0x3e868687), f(0x3eb2b2b3)],
        ),
        "Shoes" => ([f(0x3de8e8e9); 3], [0.; 3]),
        "T-shirt" => (
            [f(0x3f72f2f3), f(0x3f65e5e6), f(0x3f65e5e6)],
            [f(0x3f47c7c8), f(0x3f65e5e6), f(0x3f65e5e6)],
        ),
        "Flesh" => (
            [f(0x3f75f5f6), f(0x3f38b8b9), f(0x3f2aaaab)],
            [f(0x3f75f5f6), f(0x3f38b8b9), f(0x3f2aaaab)],
        ),
        "Roof" => (
            [0.4, f(0x3eb4b4b5), f(0x3ea4a4a5)],
            [0.4, f(0x3eb4b4b5), f(0x3ea4a4a5)],
        ),
        _ => return None,
    };
    Some(Builtin {
        diffuse: [diffuse[0], diffuse[1], diffuse[2], 1.],
        ambient: [ambient[0], ambient[1], ambient[2], 1.],
        specular: [0.; 4],
        emissive: [0.; 4],
        power: 5.,
    })
}

impl Builtin {
    /// 5d41db's material payload. Ramp size comes from the renderer/device;
    /// it is not a universal game constant. Built-in colors have no texture.
    pub fn words(&self, ramp_size: u32) -> [u32; 20] {
        let mut words = [0; 20];
        words[0] = 80;
        for (block, color) in words[1..17].chunks_exact_mut(4).zip([
            self.diffuse,
            self.ambient,
            self.specular,
            self.emissive,
        ]) {
            block.copy_from_slice(&color.map(f32::to_bits));
        }
        words[17] = self.power.to_bits();
        words[19] = ramp_size;
        words
    }
    /// Current renderer's flat albedo representation; keep the original float
    /// material alongside it so lighting need not be reconstructed from bytes.
    pub fn texture(&self) -> crate::avatar::Texture {
        crate::avatar::Texture {
            width: 1,
            height: 1,
            rgba: self
                .diffuse
                .map(|x| (x * 255.).round().clamp(0., 255.) as u8)
                .to_vec(),
        }
    }
}
