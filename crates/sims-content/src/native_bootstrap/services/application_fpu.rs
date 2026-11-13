//! Capability retention around an explicitly installed Rust control context.
//! These receipts grant no native FPU or downstream arithmetic support.
use super::*;
use sims_sim::native_application::{Construction, fpu};

#[derive(Default)]
pub(super) struct Environment {
    port: Option<fpu::Port>,
}

pub(super) struct ReadBinding {
    pub(super) heap: Identity,
    pub(super) request: fpu::ReadRequest,
    pub(super) returned: Option<fpu::ReadReturn>,
    pub(super) application_image: Owner,
    port: fpu::Port,
}

pub(super) struct LoadBinding {
    pub(super) heap: Identity,
    pub(super) request: fpu::LoadRequest,
    pub(super) returned: Option<fpu::LoadReturn>,
    // A receipt retains history; this clone retains the actual issuing owner.
    port: fpu::Port,
}

impl BootstrapServices<'_> {
    /// Retain a caller-installed context once. Configuration never installs,
    /// resets, replaces or silently reacquires a thread's control authority.
    pub fn configure_application_fpu(&mut self, port: fpu::Port) -> Result<(), Stop> {
        if self.application_fpu.port.is_some() {
            return Err(unsupported(
                "application FPU authority is already configured",
            ));
        }
        port.check_live()?;
        self.application_fpu.port = Some(port);
        Ok(())
    }

    fn application_fpu_port(&self) -> Result<&fpu::Port, Stop> {
        self.application_fpu.port.as_ref().ok_or_else(|| {
            Stop::Unavailable("no application FPU authority was explicitly configured".into())
        })
    }

    fn require_application_fpu_images(&self, binding: &ReadBinding) -> Result<(), Stop> {
        if !self.memory.has_identity(&binding.heap) {
            return Err(unsupported("application FPU read belongs to another heap"));
        }
        let image = &binding.application_image;
        if image.token != binding.request.application_image().token
            || image.data.len() != 324
            || self.memory.allocation_kind(image.token)? != Kind::Owner
            || self.memory.allocation_bytes(image.token)? != image.data
        {
            return Err(unsupported(
                "application FPU heap image lost its admitted source state",
            ));
        }
        if let Some(wrapper) = binding.request.wrapper_image() {
            if wrapper.data.len() != 12
                || self.memory.allocation_kind(wrapper.token)? != Kind::ApplicationMutexWrapper
                || self.memory.allocation_bytes(wrapper.token)? != wrapper.data
            {
                return Err(unsupported(
                    "application FPU wrapper lost its admitted source state",
                ));
            }
        }
        Ok(())
    }

    // Authentication of a held receipt is separate from admission of a new
    // source cursor. In particular, a cached retry must not copy an old image
    // back over stores synchronized by admit_application_fpu_current.
    fn held_application_fpu_read(&self, returned: &fpu::ReadReturn) -> Result<u32, Stop> {
        let (token, binding) = self
            .capabilities
            .iter()
            .find_map(|(token, cap)| match cap {
                Capability::FpuRead(binding) if binding.returned.as_ref() == Some(returned) => {
                    Some((*token, binding))
                }
                _ => None,
            })
            .ok_or_else(|| unsupported("application FPU read was not issued by these services"))?;
        returned.validate(&binding.request)?;
        self.require_application_fpu_images(binding)?;
        let configured = self.application_fpu_port()?;
        if !configured.owns_read(returned) || !binding.port.owns_read(returned) {
            return Err(unsupported(
                "application FPU read belongs to another context",
            ));
        }
        configured.check_live()?;
        binding.port.check_live()?;
        Ok(token)
    }

    pub(super) fn read_application_control(
        &mut self,
        request: &fpu::ReadRequest,
    ) -> Result<fpu::ReadReturn, Stop> {
        request.validate()?;
        let existing = self.capabilities.iter().find_map(|(token, cap)| match cap {
            Capability::FpuRead(binding)
                if binding.request.application_image().token
                    == request.application_image().token =>
            {
                Some(*token)
            }
            _ => None,
        });
        let token = if let Some(token) = existing {
            let Capability::FpuRead(binding) = &self.capabilities[&token] else {
                unreachable!()
            };
            if binding.request != *request {
                return Err(unsupported(
                    "application FPU read belongs to another invocation",
                ));
            }
            self.require_application_fpu_images(binding)?;
            token
        } else {
            let token = self.next_capability()?;
            let port = self.application_fpu_port()?.clone();
            port.check_live()?;
            // Parent-owned admission authenticates the complete COM/mutex
            // history and current heap before synchronizing sealed images.
            self.admit_application_fpu_read(request)?;
            self.commit(
                token,
                Capability::FpuRead(ReadBinding {
                    heap: self.memory.identity(),
                    request: request.clone(),
                    returned: None,
                    application_image: request.application_image().clone(),
                    port,
                }),
            );
            token
        };
        let Capability::FpuRead(binding) = &self.capabilities[&token] else {
            unreachable!()
        };
        if let Some(returned) = &binding.returned {
            let returned = returned.clone();
            returned.validate(request)?;
            self.held_application_fpu_read(&returned)?;
            return Ok(returned);
        }
        self.application_fpu_port()?.check_live()?;
        binding.port.check_live()?;
        let returned = binding.port.read_control(&binding.request)?;
        let Some(Capability::FpuRead(binding)) = self.capabilities.get_mut(&token) else {
            unreachable!()
        };
        binding.returned = Some(returned.clone()); // stage before any validation
        returned.validate(request)?;
        self.held_application_fpu_read(&returned)?;
        Ok(returned)
    }

    pub(super) fn load_application_control(
        &mut self,
        request: &fpu::LoadRequest,
    ) -> Result<fpu::LoadReturn, Stop> {
        request.validate()?;
        // A genuine receipt alone is insufficient: it must still be held by
        // this issuing heap and the configured and issuing live context.
        let read_token = self.held_application_fpu_read(request.read())?;
        let existing = self.capabilities.iter().find_map(|(token, cap)| match cap {
            Capability::FpuLoad(binding) if binding.request.read() == request.read() => {
                Some(*token)
            }
            _ => None,
        });
        let token = if let Some(token) = existing {
            let Capability::FpuLoad(binding) = &self.capabilities[&token] else {
                unreachable!()
            };
            if !self.memory.has_identity(&binding.heap) || binding.request != *request {
                return Err(unsupported(
                    "application FPU load belongs to another heap or invocation",
                ));
            }
            if !binding.port.owns_read(request.read()) {
                return Err(unsupported(
                    "application FPU load lost its issuing read context",
                ));
            }
            binding.port.check_live()?;
            token
        } else {
            let token = self.next_capability()?;
            let Capability::FpuRead(read) = &self.capabilities[&read_token] else {
                unreachable!()
            };
            let port = read.port.clone();
            self.commit(
                token,
                Capability::FpuLoad(LoadBinding {
                    heap: self.memory.identity(),
                    request: request.clone(),
                    returned: None,
                    port,
                }),
            );
            token
        };
        let Capability::FpuLoad(binding) = &self.capabilities[&token] else {
            unreachable!()
        };
        let returned = if let Some(returned) = &binding.returned {
            returned.clone()
        } else {
            let returned = binding.port.load_control(&binding.request)?;
            let Some(Capability::FpuLoad(binding)) = self.capabilities.get_mut(&token) else {
                unreachable!()
            };
            binding.returned = Some(returned.clone()); // never repeat a staged load
            returned
        };
        returned.validate(request)?;
        Ok(returned)
    }

    /// Check the current source history/heap and return the actual context's
    /// current word. The read's original word remains immutable history.
    pub fn application_fpu_control(&mut self, construction: &Construction) -> Result<u16, Stop> {
        construction.validate()?;
        let read = construction
            .fpu_read()
            .ok_or_else(|| Stop::Unavailable("application has no completed FPU read".into()))?;
        self.held_application_fpu_read(read)?;
        if let Some(load) = construction.fpu_load() {
            let binding = self
                .capabilities
                .values()
                .find_map(|cap| match cap {
                    Capability::FpuLoad(binding) if binding.returned.as_ref() == Some(load) => {
                        Some(binding)
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    unsupported("application FPU load was not issued by these services")
                })?;
            if !self.memory.has_identity(&binding.heap)
                || binding.request.read() != read
                || !binding.port.owns_read(read)
            {
                return Err(unsupported(
                    "application FPU load belongs to another heap or read context",
                ));
            }
            load.validate(&binding.request)?;
            binding.port.check_live()?;
        }
        // This exact parent helper checks cursor/image transitions and updates
        // every relevant held image; no independent image substitution here.
        self.admit_application_fpu_current(construction)?;
        read.current_word()
    }
}
