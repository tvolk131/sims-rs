//! One source-authenticated application image shared by the mutex and FPU
//! capability records. Historical receipt checks never synchronize or call it.
use super::*;
use sims_sim::native_application::{Construction, mutex};

impl BootstrapServices<'_> {
    fn require_issued_application(
        &self,
        application: &Owner,
        com: &com::Return,
    ) -> Result<(), Stop> {
        self.require_mutex_owner(application, Kind::Owner, 324)?;
        if com.application() != application.token || !self.capabilities.values().any(|cap| matches!(cap, Capability::Apartment { heap, application: token, returned } if self.memory.has_identity(heap) && *token == application.token && returned == com)) {
            return Err(unsupported("application image lost its issuing COM history or heap"));
        }
        Ok(())
    }
    fn source_mutex_token(&self, returned: &mutex::Return) -> Result<u32, Stop> {
        self.capabilities
            .iter()
            .find_map(|(token, cap)| match cap {
                Capability::NamedMutex(binding) if binding.returned.as_ref() == Some(returned) => {
                    Some(*token)
                }
                _ => None,
            })
            .ok_or_else(|| unsupported("mutex return was not issued by these services"))
    }
    fn check_source_mutex_images(
        &self,
        token: u32,
        application: &Owner,
        wrapper: &Owner,
        com: &com::Return,
    ) -> Result<(), Stop> {
        let Some(Capability::NamedMutex(binding)) = self.capabilities.get(&token) else {
            return Err(unsupported("source image capability is not a mutex"));
        };
        if !self.memory.has_identity(&binding.heap)
            || binding.request.application_image().token != application.token
            || binding.request.wrapper() != wrapper.token
            || binding.request.com_return() != com
        {
            return Err(unsupported(
                "mutex image belongs to another application, invocation or heap",
            ));
        }
        self.require_mutex_owner(wrapper, Kind::ApplicationMutexWrapper, 12)?;
        if self
            .memory
            .allocation_bytes(binding.application_image.token)?
            != binding.application_image.data
            || self.memory.allocation_bytes(binding.wrapper_image.token)?
                != binding.wrapper_image.data
        {
            return Err(unsupported(
                "mutex heap images changed outside the admitted source cursor",
            ));
        }
        Ok(())
    }
    fn check_source_read_images(
        &self,
        application: &Owner,
        com: &com::Return,
        mutex: Option<&mutex::Return>,
    ) -> Result<(), Stop> {
        // A pending or completed read may have already synchronized this same
        // app. Check every matching record before copying a new source image.
        for cap in self.capabilities.values() {
            if let Capability::FpuRead(binding) = cap {
                if binding.request.application_image().token == application.token
                    && binding.request.com_return() == com
                    && binding.request.mutex_return() == mutex
                {
                    if !self.memory.has_identity(&binding.heap)
                        || self.memory.allocation_bytes(application.token)?
                            != binding.application_image.data
                    {
                        return Err(unsupported(
                            "FPU application image changed outside its issuing heap/source cursor",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    fn synchronize_source_application(
        &mut self,
        application: &Owner,
        com: &com::Return,
        mutex: Option<&mutex::Return>,
        wrapper: Option<&Owner>,
    ) -> Result<(), Stop> {
        // Callers have checked both full allocation extents and every relevant
        // recorded preimage before either mutation. No outside borrow escapes.
        for cap in self.capabilities.values() {
            match cap {
                Capability::NamedMutex(binding)
                    if mutex.is_some()
                        && binding.returned.as_ref() == mutex
                        && binding.request.application_image().token == application.token
                        && binding.request.com_return() == com =>
                {
                    if !self.memory.has_identity(&binding.heap) || wrapper.is_none() {
                        return Err(unsupported(
                            "source synchronization lost its mutex heap/wrapper",
                        ));
                    }
                }
                Capability::FpuRead(binding)
                    if binding.request.application_image().token == application.token
                        && binding.request.com_return() == com
                        && binding.request.mutex_return() == mutex =>
                {
                    if !self.memory.has_identity(&binding.heap) {
                        return Err(unsupported("source synchronization lost its FPU heap"));
                    }
                }
                _ => (),
            }
        }
        self.memory.synchronize_owner(application)?;
        if let Some(wrapper) = wrapper {
            self.memory.synchronize_owner(wrapper)?;
        }
        for cap in self.capabilities.values_mut() {
            match cap {
                Capability::NamedMutex(binding)
                    if mutex.is_some()
                        && binding.returned.as_ref() == mutex
                        && binding.request.application_image().token == application.token
                        && binding.request.com_return() == com =>
                {
                    binding.application_image = application.clone();
                    binding.wrapper_image = wrapper.unwrap().clone();
                }
                Capability::FpuRead(binding)
                    if binding.request.application_image().token == application.token
                        && binding.request.com_return() == com
                        && binding.request.mutex_return() == mutex =>
                {
                    binding.application_image = application.clone();
                }
                _ => (),
            }
        }
        Ok(())
    }
    pub(super) fn admit_application_mutex_current(
        &mut self,
        construction: &Construction,
    ) -> Result<u32, Stop> {
        construction.validate()?;
        let returned = construction
            .mutex_return()
            .ok_or_else(|| Stop::Unavailable("application has no completed mutex return".into()))?;
        let com = construction
            .com_initialization()
            .ok_or_else(|| unsupported("application has no COM history"))?;
        let wrapper = construction
            .mutex_wrapper()
            .ok_or_else(|| unsupported("mutex return lost its source wrapper"))?;
        let token = self.source_mutex_token(returned)?;
        self.require_issued_application(construction.owner(), com)?;
        self.check_source_mutex_images(token, construction.owner(), wrapper, com)?;
        self.check_source_read_images(construction.owner(), com, Some(returned))?;
        self.synchronize_source_application(
            construction.owner(),
            com,
            Some(returned),
            Some(wrapper),
        )?;
        Ok(token)
    }
    pub(super) fn admit_application_fpu_read(
        &mut self,
        request: &fpu::ReadRequest,
    ) -> Result<(), Stop> {
        request.validate()?;
        self.require_issued_application(request.application_image(), request.com_return())?;
        if let (Some(returned), Some(wrapper)) = (request.mutex_return(), request.wrapper_image()) {
            let token = self.source_mutex_token(returned)?;
            self.check_source_mutex_images(
                token,
                request.application_image(),
                wrapper,
                request.com_return(),
            )?;
        }
        self.check_source_read_images(
            request.application_image(),
            request.com_return(),
            request.mutex_return(),
        )?;
        self.synchronize_source_application(
            request.application_image(),
            request.com_return(),
            request.mutex_return(),
            request.wrapper_image(),
        )
    }
    pub(super) fn admit_application_fpu_current(
        &mut self,
        construction: &Construction,
    ) -> Result<(), Stop> {
        construction.validate()?;
        let read = construction
            .fpu_read()
            .ok_or_else(|| Stop::Unavailable("application has no completed FPU read".into()))?;
        let binding = self
            .capabilities
            .values()
            .find_map(|cap| match cap {
                Capability::FpuRead(binding) if binding.returned.as_ref() == Some(read) => {
                    Some(binding)
                }
                _ => None,
            })
            .ok_or_else(|| unsupported("FPU read was not issued by these services"))?;
        if !self.memory.has_identity(&binding.heap)
            || binding.request != *read.request()
            || binding.request.application_image().token != construction.owner().token
            || Some(binding.request.com_return()) != construction.com_initialization()
            || binding.request.mutex_return() != construction.mutex_return()
            || binding.request.wrapper_image().map(|w| w.token)
                != construction.mutex_wrapper().map(|w| w.token)
        {
            return Err(unsupported(
                "FPU read belongs to another source application or heap",
            ));
        }
        if let Some(load) = construction.fpu_load() {
            let owned = self.capabilities.values().any(|cap| matches!(cap, Capability::FpuLoad(binding) if self.memory.has_identity(&binding.heap) && binding.returned.as_ref() == Some(load) && binding.request == *load.request() && binding.request.read() == read));
            if !owned {
                return Err(unsupported(
                    "FPU load was not issued for this application's read",
                ));
            }
        }
        let request = read.request();
        self.require_issued_application(construction.owner(), request.com_return())?;
        if let (Some(returned), Some(wrapper)) =
            (construction.mutex_return(), construction.mutex_wrapper())
        {
            self.check_source_mutex_images(
                self.source_mutex_token(returned)?,
                construction.owner(),
                wrapper,
                request.com_return(),
            )?;
        }
        self.check_source_read_images(
            construction.owner(),
            request.com_return(),
            construction.mutex_return(),
        )?;
        self.synchronize_source_application(
            construction.owner(),
            request.com_return(),
            construction.mutex_return(),
            construction.mutex_wrapper(),
        )
    }
}
