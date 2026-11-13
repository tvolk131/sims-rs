//! Actual port-authority returns for the recovered CreateMutex call. Numeric
//! words are logical source capabilities, never constructors for live leases.
use super::{platform::MutexRequest, unsupported};
use crate::saved_matrix::Stop;
use std::rc::Rc;

fn unavailable(error: sims_kernel::Error) -> Stop {
    Stop::Unavailable(format!("named-object authority: {error}"))
}

#[derive(Debug)]
enum Outcome {
    Created(sims_kernel::Lease),
    Rejected(sims_kernel::Failure),
}
#[derive(Debug)]
struct Returned {
    request: MutexRequest,
    word: u32,
    outcome: Outcome,
}
/// Immutable history; clones share one actual backend cleanup obligation.
#[derive(Debug, Clone)]
pub struct Return(Rc<Returned>);
impl PartialEq for Return {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for Return {}
impl Return {
    pub fn request(&self) -> &MutexRequest {
        &self.0.request
    }
    pub fn word(&self) -> u32 {
        self.0.word
    }
    pub fn api_error(&self) -> Option<u32> {
        match &self.0.outcome {
            Outcome::Rejected(error) => Some(error.code()),
            _ => None,
        }
    }
    pub fn object(&self) -> Option<u64> {
        match &self.0.outcome {
            Outcome::Created(lease) => Some(lease.object()),
            _ => None,
        }
    }
    pub fn validate(&self, request: &MutexRequest) -> Result<(), Stop> {
        request.validate()?;
        if self.0.request != *request {
            return Err(unsupported(
                "mutex return has another source invocation or owner image",
            ));
        }
        match &self.0.outcome {
            Outcome::Created(lease)
                if self.0.word != 0 && lease.name() == request.name_bytes()? =>
            {
                Ok(())
            }
            Outcome::Rejected(error) if self.0.word == 0 && error.code() != 0 => Ok(()),
            _ => Err(unsupported("mutex return lost its actual outcome role")),
        }
        // Historical validation deliberately sends no IPC and remains valid
        // after acknowledged close or authority loss.
    }
    /// Checks backend liveness only. BootstrapServices additionally authenticates
    /// the issuing heap, source invocation and current owner images.
    pub fn check_live(&self) -> Result<(), Stop> {
        match &self.0.outcome {
            Outcome::Created(lease) => lease.check().map_err(unavailable),
            Outcome::Rejected(_) => Err(Stop::Unavailable(
                "API-null mutex has no live reference".into(),
            )),
        }
    }
    pub fn close(&self) -> Result<(), Stop> {
        match &self.0.outcome {
            Outcome::Created(lease) => lease.close().map_err(unavailable),
            Outcome::Rejected(_) => Ok(()),
        }
    }
}

/// Reserve before dispatch and retain across errors. The kernel operation owns
/// its staged/uncertain reply even when resolution cannot return success.
#[derive(Debug)]
pub struct Pending {
    request: MutexRequest,
    word: u32,
    operation: sims_kernel::CreateOperation,
    returned: Option<Return>,
}
impl Pending {
    pub fn begin(
        request: &MutexRequest,
        client: &sims_kernel::Client,
        word: u32,
    ) -> Result<Self, Stop> {
        request.validate()?;
        if word == 0 {
            return Err(unsupported("mutex capability must be reserved before IPC"));
        }
        let operation = client
            .begin_create(request.name_bytes()?)
            .map_err(unavailable)?;
        Ok(Self {
            request: request.clone(),
            word,
            operation,
            returned: None,
        })
    }
    pub fn resolve(&mut self) -> Result<Return, Stop> {
        if let Some(returned) = &self.returned {
            return Ok(returned.clone());
        }
        let (word, outcome) = match self.operation.resolve().map_err(unavailable)? {
            sims_kernel::CreateOutcome::Created(lease) => (self.word, Outcome::Created(lease)),
            sims_kernel::CreateOutcome::Rejected(error) => (0, Outcome::Rejected(error)),
        };
        let returned = Return(Rc::new(Returned {
            request: self.request.clone(),
            word,
            outcome,
        }));
        self.returned = Some(returned.clone());
        Ok(returned)
    }
}
