//! Unsupported platforms retain the public boundary without issuing resources
//! or manufacturing an identity. These opaque types are uninhabited.
use std::{
    fmt, io,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Copy, Debug)]
enum Never {}
#[derive(Clone, Debug)]
pub struct Client(Never);
#[derive(Clone, Debug)]
pub struct CreateOperation(Never);
#[derive(Clone, Debug)]
pub struct Lease(Never);
#[derive(Clone, Debug)]
pub struct Failure(Never);
#[derive(Clone, Debug)]
pub enum CreateOutcome {
    Created(Lease),
    Rejected(Failure),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error;
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("named-object authority unavailable on this platform")
    }
}
impl std::error::Error for Error {}
impl Client {
    pub fn connect(_: &Path, _: u32) -> Result<Self, Error> {
        Err(Error)
    }
    pub fn begin_create(&self, _: &[u8]) -> Result<CreateOperation, Error> {
        match self.0 {}
    }
    pub fn epoch(&self) -> [u8; 16] {
        match self.0 {}
    }
    pub fn connection(&self) -> u64 {
        match self.0 {}
    }
    pub fn peer_session(&self) -> Option<u32> {
        match self.0 {}
    }
}
impl CreateOperation {
    pub fn resolve(&self) -> Result<CreateOutcome, Error> {
        match self.0 {}
    }
}
impl Lease {
    pub fn check(&self) -> Result<(), Error> {
        match self.0 {}
    }
    pub fn close(&self) -> Result<(), Error> {
        match self.0 {}
    }
    pub fn is_closed(&self) -> bool {
        match self.0 {}
    }
    pub fn handle(&self) -> u64 {
        match self.0 {}
    }
    pub fn object(&self) -> u64 {
        match self.0 {}
    }
    pub fn epoch(&self) -> [u8; 16] {
        match self.0 {}
    }
    pub fn name(&self) -> &[u8] {
        match self.0 {}
    }
}
impl Failure {
    pub fn code(&self) -> u32 {
        match self.0 {}
    }
}
pub mod server {
    use super::*;
    #[derive(Debug)]
    pub struct Server(Never);
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ServerSummary {
        pub objects_remaining: usize,
        pub handles_remaining: usize,
        pub connections_remaining: usize,
    }
    impl Server {
        pub fn bind(_: &Path, _: u32) -> io::Result<Self> {
            Err(io::Error::new(io::ErrorKind::Unsupported, Error))
        }
        pub fn epoch(&self) -> [u8; 16] {
            match self.0 {}
        }
        pub fn run(self, _: Arc<AtomicBool>) -> io::Result<ServerSummary> {
            match self.0 {}
        }
    }
}
pub use server::{Server, ServerSummary};
