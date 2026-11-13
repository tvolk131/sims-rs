use super::{File, Source};
use sims_sim::{
    native_matrix,
    native_world::startup::{World, reader},
    object_catalog::iff::Stream,
    saved_matrix::{Scratch, Stop},
    saved_world,
};

pub struct Inputs<FileStream, Services> {
    pub world: World,
    pub file: File<FileStream>,
    pub scratch: Scratch,
    pub services: Services,
    pub version: i32,
}

pub struct BeginError<FileStream, Services> {
    pub stop: Stop,
    pub inputs: Inputs<FileStream, Services>,
}

/// Same World, open IFF/stream, cursor, Scratch and services across every return
/// or pending error. No Clone/Deserialize or implicit filesystem restoration.
pub struct ReadOwner<FileStream, Services> {
    read: reader::Owner,
    file: File<FileStream>,
    services: Services,
}

impl<FileStream, Services> ReadOwner<FileStream, Services> {
    pub fn begin(
        inputs: Inputs<FileStream, Services>,
    ) -> Result<Self, BeginError<FileStream, Services>> {
        if let Err(stop) = inputs.file.ready() {
            return Err(BeginError { stop, inputs });
        }
        let Inputs {
            world,
            file,
            scratch,
            services,
            version,
        } = inputs;
        match reader::Owner::new(world, version, scratch) {
            Ok(read) => Ok(Self {
                read,
                file,
                services,
            }),
            Err(error) => Err(BeginError {
                stop: error.stop,
                inputs: Inputs {
                    world: error.world,
                    file,
                    scratch: error.scratch,
                    services,
                    version: error.version,
                },
            }),
        }
    }
    pub fn read(&self) -> &reader::Owner {
        &self.read
    }
    pub fn file(&self) -> &File<FileStream> {
        &self.file
    }
    pub fn services(&self) -> &Services {
        &self.services
    }
    pub fn status(&self) -> Option<i32> {
        self.read.status()
    }
    pub fn into_returned(self) -> Result<Returned<FileStream, Services>, Self> {
        let Self {
            read,
            file,
            services,
        } = self;
        match read.into_returned() {
            Ok(read) => Ok(Returned {
                read,
                file,
                services,
            }),
            Err(read) => Err(Self {
                read,
                file,
                services,
            }),
        }
    }
}

impl<FileStream: Stream, Services: saved_world::Host + native_matrix::Host>
    ReadOwner<FileStream, Services>
{
    pub fn step(&mut self) -> Result<Option<i32>, Stop> {
        self.read.step(&mut Source {
            file: &mut self.file,
            services: &mut self.services,
        })
    }
}

pub struct Returned<FileStream, Services> {
    read: reader::Returned,
    file: File<FileStream>,
    services: Services,
}

impl<FileStream, Services> Returned<FileStream, Services> {
    pub fn status(&self) -> i32 {
        self.read.status()
    }
    pub fn into_parts(
        self,
    ) -> (
        World,
        saved_world::Execution,
        Scratch,
        File<FileStream>,
        Services,
    ) {
        let (world, execution, scratch) = self.read.into_parts();
        (world, execution, scratch, self.file, self.services)
    }
}
