use serde_json::{Value, json};
use sims_vm::scheduler::clock::draw::actions::image::{bitmap, surface};
const IM: u32 = 0x10000100;
const SF: u32 = 0x10001000;
const OTHER: u32 = 0x10004000;
fn bytes(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn state(header: &[u8], surface: &[u8], refs: &[u32; 4]) -> Value {
    json!({"header":hex(header),"surface":hex(surface),"refs":refs})
}
struct Host {
    surface: Vec<u8>,
    refs: [u32; 4],
    events: Vec<Value>,
}
struct Platform<'a> {
    header: &'a [u8],
    refs: &'a [u32; 4],
    events: &'a mut Vec<Value>,
}
impl surface::Backend for Platform<'_> {
    fn release(&mut self, fields: &mut [u8], interface: u32) -> Result<(), String> {
        self.events.push(
            json!({"request":{"ComRelease":interface},"state":state(self.header,fields,self.refs)}),
        );
        Ok(())
    }
    fn free(&mut self, fields: &mut [u8], surface: u32) -> Result<(), String> {
        self.events
            .push(json!({"request":{"Free":surface},"state":state(self.header,fields,self.refs)}));
        Ok(())
    }
}
impl bitmap::Backend for Host {
    fn virtual_call(
        &mut self,
        h: &mut [u8],
        owner: u32,
        slot: u16,
        args: &[u32],
    ) -> Result<(), String> {
        if owner == SF {
            let mut platform = Platform {
                header: h,
                refs: &self.refs,
                events: &mut self.events,
            };
            match slot {
                4 => surface::close(&mut self.surface, &mut platform),
                0 => surface::destroy(SF, &mut self.surface, args[0], &mut platform),
                _ => unreachable!(),
            }
        } else {
            assert_eq!(slot, 0);
            self.events.push(json!({"request":{"Delete":{"owner":owner,"flags":args[0]}},"state":state(h,&self.surface,&self.refs)}));
            Ok(())
        }
    }
    fn release_image(&mut self, h: &mut [u8], image: u32) -> Result<(), String> {
        let n = ((image - OTHER) / 256) as usize;
        self.refs[n] = self.refs[n].wrapping_sub(1);
        if self.refs[n] == 0 {
            self.virtual_call(h, image, 0, &[1])?;
        }
        Ok(())
    }
    fn destroy_string(&mut self, _: &mut [u8], _: u32) -> Result<(), String> {
        Err("string service not included in this corpus".into())
    }
    fn free(&mut self, h: &mut [u8], owner: u32) -> Result<(), String> {
        self.events
            .push(json!({"request":{"Free":owner},"state":state(h,&self.surface,&self.refs)}));
        Ok(())
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_bitmap_and_unlocked_surface_destruction_order() {
    let data: Value =
        serde_json::from_str(include_str!("fixtures/native-bitmap-destruction.json")).unwrap();
    assert_eq!(data["cases"].as_array().unwrap().len(), 320);
    for (i, row) in data["cases"].as_array().unwrap().iter().enumerate() {
        let c = &row["input"];
        let mut header = bytes(c["header"].as_str().unwrap());
        let mut host = Host {
            surface: bytes(c["surface"].as_str().unwrap()),
            refs: [1; 4],
            events: vec![],
        };
        bitmap::destroy(
            IM,
            &mut header,
            c["flags"].as_u64().unwrap() as u32,
            &mut host,
        )
        .unwrap();
        assert_eq!(
            json!({"events":host.events,"state":state(&header,&host.surface,&host.refs)}),
            row["expected"],
            "case {i}"
        );
    }
}
