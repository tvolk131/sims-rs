//! Explicit version-query input and the pending named-mutex call contract.
//! A Rust-port policy is declared input, not native GetVersion execution.
use super::unsupported;
use crate::{native_world::resources::strings, saved_matrix::Stop};
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct VersionRequest {
    pub instruction: u32,
    pub import_slot: u32,
    pub return_pc: u32,
    pub application: u64,
    invocation: Rc<()>,
}
impl PartialEq for VersionRequest {
    fn eq(&self, other: &Self) -> bool {
        self.instruction == other.instruction
            && self.import_slot == other.import_slot
            && self.return_pc == other.return_pc
            && self.application == other.application
            && Rc::ptr_eq(&self.invocation, &other.invocation)
    }
}
impl Eq for VersionRequest {}
impl VersionRequest {
    pub(super) fn new(application: u64) -> Self {
        Self {
            instruction: 0x4eeb82,
            import_slot: 0x666124,
            return_pc: 0x4eeb88,
            application,
            invocation: Rc::new(()),
        }
    }
    pub fn validate(&self) -> Result<(), Stop> {
        if self.instruction != 0x4eeb82
            || self.import_slot != 0x666124
            || self.return_pc != 0x4eeb88
            || self.application == 0
            || self
                .application
                .checked_add(324)
                .is_none_or(|n| n > 1_u64 << 32)
        {
            return Err(unsupported(
                "version request differs from its recovered caller",
            ));
        }
        Ok(())
    }
}

/// No default or OS detection is provided. This policy must be supplied
/// explicitly and must never be described as an original process's OS state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionPolicy {
    RawDword(u32),
}
impl VersionPolicy {
    pub fn reply(self, request: &VersionRequest) -> Result<VersionReturn, Stop> {
        request.validate()?;
        let Self::RawDword(raw) = self;
        Ok(VersionReturn {
            request: request.clone(),
            raw,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionReturn {
    request: VersionRequest,
    raw: u32,
}
impl VersionReturn {
    pub fn raw(&self) -> u32 {
        self.raw
    }
    pub fn nt_name(&self) -> bool {
        (self.raw & 0x80000000) == 0 && (self.raw as u8) > 4
    }
    pub fn validate(&self, request: &VersionRequest) -> Result<(), Stop> {
        request.validate()?;
        if self.request != *request {
            return Err(unsupported("version return has another invocation"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperRequest {
    pub instruction: u32,
    pub bytes: u32,
    pub version: VersionReturn,
}
impl WrapperRequest {
    pub(super) fn new(version: &VersionReturn) -> Self {
        Self {
            instruction: if version.nt_name() {
                0x4eed19
            } else {
                0x4eed67
            },
            bytes: 12,
            version: version.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.version.validate(&self.version.request)?;
        if *self != Self::new(&self.version) {
            return Err(unsupported(
                "mutex wrapper allocation differs from its version branch",
            ));
        }
        Ok(())
    }
    pub fn application(&self) -> u64 {
        self.version.request.application
    }
}

/// Sealed source entry. Complete current images come only from the validated
/// cursor; callers cannot substitute arbitrary heap snapshots or call operands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutexRequest {
    pub(super) instruction: u32,
    pub(super) import_slot: u32,
    pub(super) return_pc: u32,
    pub(super) security_attributes: u32,
    pub(super) initial_owner: bool,
    pub(super) name: strings::State,
    pub(super) wrapper: u64,
    pub(super) version: VersionReturn,
    application_image: crate::native_world_children::Owner,
    wrapper_image: crate::native_world_children::Owner,
    com: super::com::Return,
}
impl MutexRequest {
    pub(super) fn new(
        version: &VersionReturn,
        application: &crate::native_world_children::Owner,
        wrapper: &crate::native_world_children::Owner,
        name: &strings::State,
        com: &super::com::Return,
    ) -> Self {
        Self {
            instruction: 0x5d9509,
            import_slot: 0x66629c,
            return_pc: 0x5d950f,
            security_attributes: 0,
            initial_owner: false,
            name: name.clone(),
            wrapper: wrapper.token,
            version: version.clone(),
            application_image: application.clone(),
            wrapper_image: wrapper.clone(),
            com: com.clone(),
        }
    }
    pub fn validate(&self) -> Result<(), Stop> {
        self.name.validate()?;
        self.version.validate(&self.version.request)?;
        if *self
            != Self::new(
                &self.version,
                &self.application_image,
                &self.wrapper_image,
                &self.name,
                &self.com,
            )
            || self.wrapper == 0
            || self.wrapper.checked_add(12).is_none_or(|n| n > 1_u64 << 32)
            || self.application_image.token != self.version.request.application
            || self.application_image.data.len() != 324
            || self.wrapper_image.data.len() != 12
            || self.wrapper_image.data[..4] != 0x673b18_u32.to_le_bytes()
            || self.wrapper_image.data[8..] != 0_u32.to_le_bytes()
            || self.com.application() != self.application_image.token
        {
            return Err(unsupported(
                "CreateMutex entry differs from its recovered caller",
            ));
        }
        self.name_bytes()?;
        Ok(())
    }
    pub fn wrapper(&self) -> u64 {
        self.wrapper
    }
    pub fn name_state(&self) -> &strings::State {
        &self.name
    }
    pub fn version(&self) -> &VersionReturn {
        &self.version
    }
    pub fn application_image(&self) -> &crate::native_world_children::Owner {
        &self.application_image
    }
    pub fn wrapper_image(&self) -> &crate::native_world_children::Owner {
        &self.wrapper_image
    }
    pub fn com_return(&self) -> &super::com::Return {
        &self.com
    }
    /// Exact bounded ANSI C-prefix bytes; no UTF-8 conversion, case folding,
    /// path normalization or namespace rewriting is admitted here.
    pub fn name_bytes(&self) -> Result<&[u8], Stop> {
        self.name.validate()?;
        let end = self
            .name
            .backing
            .data
            .iter()
            .take(4097)
            .position(|b| *b == 0)
            .ok_or_else(|| unsupported("mutex name exceeds the owned CString profile"))?;
        Ok(&self.name.backing.data[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selector_keeps_raw_bits_and_checks_only_the_original_sign_and_low_byte() {
        for (raw, expected) in [
            (0, false),
            (4, false),
            (5, true),
            (255, true),
            (0x01000004, false),
            (0x7fff0005, true),
            (0x80000005, false),
            (0xffffffff, false),
        ] {
            let request = VersionRequest::new(0x10000000);
            let returned = VersionPolicy::RawDword(raw).reply(&request).unwrap();
            assert_eq!(returned.raw(), raw);
            assert_eq!(returned.nt_name(), expected);
            returned.validate(&request).unwrap();
            assert!(returned.validate(&VersionRequest::new(0x10000000)).is_err());
        }
    }
}
