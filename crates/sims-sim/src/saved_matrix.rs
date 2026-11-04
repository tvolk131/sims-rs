//! Native431005/43138b/4312ff array reads and478909 light reset.
//! Header dimensions and actual allocation/columns are independent. This is a
//! source/lease component, not a world/room rebuild or an IFF producer.
use serde::{Deserialize, Serialize};

pub const NOOP_RESOURCE_CALLBACK: u32 = 0x46fb64;
pub const HEAD_CONVERTER: u32 = 0x47893a;
/// A safe host profile limit, not a native array-size/error rule.
pub const MAX_ALLOCATION_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Backing {
    pub token: u64,
    /// Relative to data[0], not reconstructed from the current header.
    pub column_offsets: Vec<u32>,
    pub data: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Allocation {
    Null,
    Unavailable,
    /// Native old allocation was freed; its pointer has not been replaced yet.
    Released {
        token: u64,
    },
    Known(Backing),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Matrix {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub allocation: Allocation,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scratch {
    pub requested_converter: u32,
}

/// These are host/profile stops, NEVER native returned array statuses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stop {
    Unavailable(String),
    Unsupported(String),
}
impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(s) | Self::Unsupported(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for Stop {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    Missing { native_status: i32 },
    Found { lease: u64 },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationRequest {
    pub columns: u32,
    pub data_bytes: u32,
    /// Original allocation also includes the column pointer words.
    pub total_bytes: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Allocated {
    Null,
    /// Concrete allocator output; never manufactured native uninitialized bytes.
    Bytes {
        token: u64,
        data: Vec<u8>,
    },
}
pub trait Host {
    fn lookup(&mut self, tag: u32, id: u32, callback: u32) -> Result<Lookup, Stop>;
    fn source(&mut self, lease: u64) -> Result<Vec<u8>, Stop>;
    fn release(&mut self, lease: u64) -> Result<(), Stop>;
    fn free(&mut self, token: u64) -> Result<(), Stop>;
    fn allocate(&mut self, request: AllocationRequest) -> Result<Allocated, Stop>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ResizePhase {
    Inspect,
    Free(u64),
    Dimensions,
    Allocate,
    Accept,
    Complete(bool),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resize {
    width: i32,
    height: i32,
    phase: ResizePhase,
    pending: Option<Allocated>,
}
impl Resize {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            phase: ResizePhase::Inspect,
            pending: None,
        }
    }
    /// Original AL result, distinct from an unavailable allocation service.
    pub fn step(
        &mut self,
        matrix: &mut Matrix,
        host: &mut impl Host,
    ) -> Result<Option<bool>, Stop> {
        match self.phase {
            ResizePhase::Inspect => {
                if matrix.width == self.width && matrix.height == self.height {
                    self.phase = ResizePhase::Complete(true);
                } else {
                    self.phase = match &matrix.allocation {
                        Allocation::Known(b) => ResizePhase::Free(b.token),
                        Allocation::Null => ResizePhase::Dimensions,
                        Allocation::Unavailable => {
                            return Err(Stop::Unavailable(
                                "old matrix allocation unavailable".into(),
                            ));
                        }
                        Allocation::Released { .. } => {
                            return Err(Stop::Unsupported(
                                "resize began with an already released allocation".into(),
                            ));
                        }
                    };
                }
            }
            ResizePhase::Free(token) => {
                host.free(token)?;
                matrix.allocation = Allocation::Released { token };
                self.phase = ResizePhase::Dimensions;
            }
            ResizePhase::Dimensions => {
                matrix.width = self.width;
                matrix.height = self.height;
                if self.width <= 0 || self.height <= 0 {
                    matrix.allocation = Allocation::Null;
                    self.phase = ResizePhase::Complete(true);
                } else {
                    self.phase = ResizePhase::Allocate;
                }
            }
            ResizePhase::Allocate => {
                let request = allocation_request(self.width, self.height, matrix.stride)?;
                self.pending = Some(host.allocate(request)?);
                self.phase = ResizePhase::Accept;
            }
            ResizePhase::Accept => {
                match self
                    .pending
                    .as_ref()
                    .ok_or_else(|| Stop::Unsupported("allocation reply missing".into()))?
                {
                    Allocated::Null => {
                        matrix.allocation = Allocation::Null;
                        self.pending = None;
                        self.phase = ResizePhase::Complete(false);
                    }
                    Allocated::Bytes { data, .. } => {
                        let request = allocation_request(self.width, self.height, matrix.stride)?;
                        if data.len() < request.data_bytes as usize {
                            return Err(Stop::Unsupported(
                                "allocator reply is shorter than its requested data area".into(),
                            ));
                        }
                        let Allocated::Bytes { token, data } = self.pending.take().unwrap() else {
                            unreachable!()
                        };
                        let pitch = (i64::from(self.height) * i64::from(matrix.stride)) as u32;
                        matrix.allocation = Allocation::Known(Backing {
                            token,
                            data,
                            column_offsets: (0..self.width as u32).map(|x| x * pitch).collect(),
                        });
                        self.phase = ResizePhase::Complete(true);
                    }
                }
            }
            ResizePhase::Complete(_) => {}
        }
        Ok(if let ResizePhase::Complete(value) = self.phase {
            Some(value)
        } else {
            None
        })
    }
}
fn allocation_request(width: i32, height: i32, stride: i32) -> Result<AllocationRequest, Stop> {
    if width <= 0 || height <= 0 || stride < 0 {
        return Err(Stop::Unsupported(
            "unsafe matrix allocation dimensions/stride".into(),
        ));
    }
    let body = i64::from(width)
        .checked_mul(i64::from(height))
        .and_then(|n| n.checked_mul(i64::from(stride)))
        .ok_or_else(|| {
            Stop::Unsupported(
                "native matrix allocation arithmetic overflows the qualified host".into(),
            )
        })?;
    let total = body.checked_add(i64::from(width) * 4).ok_or_else(|| {
        Stop::Unsupported("native matrix allocation extent overflows the qualified host".into())
    })?;
    if total > MAX_ALLOCATION_BYTES as i64 || total > i32::MAX as i64 {
        return Err(Stop::Unsupported(
            "native matrix allocation exceeds the qualified host extent".into(),
        ));
    }
    Ok(AllocationRequest {
        columns: width as u32,
        data_bytes: body as u32,
        total_bytes: total as u32,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum ReadPhase {
    Scratch,
    Lookup,
    Source,
    Discriminate,
    ParsedHeader,
    ParsedResize,
    ParsedCopy,
    RawHeader(u8),
    RawResize,
    RawBody,
    Release,
    DropStream,
    Complete(i32),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Read {
    tag: u32,
    id: u32,
    converter: u32,
    phase: ReadPhase,
    pub held_lease: Option<u64>,
    source: Option<Vec<u8>>,
    pub source_cursor: usize,
    resize: Option<Resize>,
    encoding: u32,
    raw_output: usize,
    raw_end: usize,
    raw_stream: bool,
}
impl Read {
    pub fn new(tag: u32, id: u32, requested_converter: u32) -> Self {
        Self {
            tag,
            id,
            converter: requested_converter,
            phase: ReadPhase::Scratch,
            held_lease: None,
            source: None,
            source_cursor: 0,
            resize: None,
            encoding: 0,
            raw_output: 0,
            raw_end: 0,
            raw_stream: false,
        }
    }
    pub fn status(&self) -> Option<i32> {
        if let ReadPhase::Complete(s) = self.phase {
            Some(s)
        } else {
            None
        }
    }
    /// Checkpoint this together with Matrix, Scratch and the actual host state.
    /// A host Err is pending, not a native missing-resource/error return.
    pub fn step(
        &mut self,
        matrix: &mut Matrix,
        scratch: &mut Scratch,
        host: &mut impl Host,
    ) -> Result<Option<i32>, Stop> {
        match self.phase {
            ReadPhase::Scratch => {
                scratch.requested_converter = self.converter;
                self.phase = ReadPhase::Lookup;
            }
            ReadPhase::Lookup => {
                self.phase = match host.lookup(self.tag, self.id, NOOP_RESOURCE_CALLBACK)? {
                    Lookup::Missing { native_status } => ReadPhase::Complete(native_status),
                    Lookup::Found { lease } => {
                        self.held_lease = Some(lease);
                        ReadPhase::Source
                    }
                };
            }
            ReadPhase::Source => {
                self.source = Some(host.source(self.lease()?)?);
                self.phase = ReadPhase::Discriminate;
            }
            ReadPhase::Discriminate => {
                let marker = i16::from_le_bytes(self.span(0, 2)?.try_into().unwrap());
                self.phase = if marker == 0 {
                    self.raw_stream = true;
                    ReadPhase::RawHeader(0)
                } else {
                    ReadPhase::ParsedHeader
                };
            }
            ReadPhase::ParsedHeader => {
                let header = self.span(0, 6)?;
                let stride = i16::from_le_bytes(header[0..2].try_into().unwrap());
                let width = i16::from_le_bytes(header[2..4].try_into().unwrap());
                let height = i16::from_le_bytes(header[4..6].try_into().unwrap());
                matrix.stride = i32::from(stride);
                self.resize = Some(Resize::new(i32::from(width), i32::from(height)));
                self.phase = ReadPhase::ParsedResize;
            }
            ReadPhase::ParsedResize => {
                match self
                    .resize
                    .as_mut()
                    .ok_or_else(|| Stop::Unsupported("parsed resize cursor missing".into()))?
                    .step(matrix, host)?
                {
                    None => {}
                    Some(false) => self.phase = ReadPhase::Complete(-1), // native skips release
                    Some(true) => {
                        self.phase = ReadPhase::ParsedCopy;
                        self.copy_parsed(matrix)?;
                        self.phase = ReadPhase::Release;
                    }
                }
            }
            ReadPhase::ParsedCopy => {
                self.copy_parsed(matrix)?;
                self.phase = ReadPhase::Release;
            }
            ReadPhase::RawHeader(field) => {
                if field > 5 {
                    return Err(Stop::Unsupported(
                        "raw array header cursor is outside its field sequence".into(),
                    ));
                }
                // Two ignored u16s followed by four native i32/u32 values.
                let count = if field < 2 { 2 } else { 4 };
                let bytes = self.span(self.source_cursor, count)?;
                let value = if count == 2 {
                    u32::from(u16::from_le_bytes(bytes.try_into().unwrap()))
                } else {
                    u32::from_le_bytes(bytes.try_into().unwrap())
                };
                self.source_cursor += count;
                match field {
                    2 => matrix.width = value as i32,
                    3 => matrix.height = value as i32,
                    4 => matrix.stride = value as i32,
                    5 => self.encoding = value,
                    _ => {}
                }
                if field == 5 {
                    self.resize = Some(Resize::new(matrix.width, matrix.height));
                    self.phase = ReadPhase::RawResize;
                } else {
                    self.phase = ReadPhase::RawHeader(field + 1);
                }
            }
            ReadPhase::RawResize => {
                match self
                    .resize
                    .as_mut()
                    .ok_or_else(|| Stop::Unsupported("raw resize cursor missing".into()))?
                    .step(matrix, host)?
                {
                    None => {}
                    Some(false) => self.phase = ReadPhase::Release,
                    Some(true) => {
                        let (begin, end) = raw_span(matrix)?;
                        self.raw_output = begin;
                        self.raw_end = end;
                        self.phase = ReadPhase::RawBody;
                    }
                }
            }
            ReadPhase::RawBody => {
                if self.raw_output >= self.raw_end {
                    self.phase = ReadPhase::Release;
                } else if self.encoding == 0 {
                    let count = self.raw_end - self.raw_output;
                    let source = self.span(self.source_cursor, count)?;
                    backing_mut(matrix)?.data[self.raw_output..self.raw_end]
                        .copy_from_slice(source);
                    self.source_cursor += count;
                    self.raw_output = self.raw_end;
                    self.phase = ReadPhase::Release;
                } else {
                    let command =
                        u16::from_le_bytes(self.span(self.source_cursor, 2)?.try_into().unwrap());
                    let count = usize::from(command & 0x7fff);
                    if count == 0 || count > self.raw_end - self.raw_output {
                        return Err(Stop::Unsupported(
                            "unsafe zero/overflow native RLE run".into(),
                        ));
                    }
                    let input_count = if command & 0x8000 == 0 { count } else { 1 };
                    let begin = self.source_cursor + 2;
                    let end = begin + input_count;
                    // Confirm complete aligned input before this run's writes.
                    self.span(begin, input_count + (end & 1))?;
                    let bytes = self.span(begin, input_count)?;
                    if input_count == count {
                        backing_mut(matrix)?.data[self.raw_output..self.raw_output + count]
                            .copy_from_slice(bytes);
                    } else {
                        backing_mut(matrix)?.data[self.raw_output..self.raw_output + count]
                            .fill(bytes[0]);
                    }
                    self.source_cursor = end + (end & 1);
                    self.raw_output += count;
                    if self.raw_output == self.raw_end {
                        self.phase = ReadPhase::Release;
                    }
                }
            }
            ReadPhase::Release => {
                host.release(self.lease()?)?;
                self.held_lease = None;
                self.phase = if self.raw_stream {
                    ReadPhase::DropStream
                } else {
                    self.source = None;
                    ReadPhase::Complete(0)
                };
            }
            ReadPhase::DropStream => {
                self.source = None;
                self.raw_stream = false;
                self.phase = ReadPhase::Complete(0);
            }
            ReadPhase::Complete(_) => {}
        }
        Ok(self.status())
    }
    fn lease(&self) -> Result<u64, Stop> {
        self.held_lease
            .ok_or_else(|| Stop::Unsupported("array source lease absent".into()))
    }
    fn span(&self, offset: usize, count: usize) -> Result<&[u8], Stop> {
        self.source
            .as_ref()
            .and_then(|s| offset.checked_add(count).and_then(|end| s.get(offset..end)))
            .ok_or_else(|| {
                Stop::Unsupported("array source bytes are incomplete for the qualified read".into())
            })
    }
    fn copy_parsed(&self, matrix: &mut Matrix) -> Result<(), Stop> {
        let count = matrix
            .width
            .wrapping_mul(matrix.height)
            .wrapping_mul(matrix.stride) as u32 as usize;
        if count == 0 {
            return Ok(());
        }
        if count > MAX_ALLOCATION_BYTES || matrix.width < 0 {
            return Err(Stop::Unsupported(
                "unsafe parsed-resident copy extent".into(),
            ));
        }
        let source_offset = (matrix.width as usize)
            .checked_mul(4)
            .and_then(|n| 8_usize.checked_add(n))
            .ok_or_else(|| Stop::Unsupported("parsed source column extent overflow".into()))?;
        let input = self.span(source_offset, count)?;
        let backing = backing_mut(matrix)?;
        let begin = *backing
            .column_offsets
            .first()
            .ok_or_else(|| Stop::Unsupported("parsed destination column zero absent".into()))?
            as usize;
        let end = begin
            .checked_add(count)
            .ok_or_else(|| Stop::Unsupported("parsed destination extent overflow".into()))?;
        let target = backing
            .data
            .get_mut(begin..end)
            .ok_or_else(|| Stop::Unsupported("parsed copy exceeds retained allocation".into()))?;
        target.copy_from_slice(input);
        Ok(())
    }
}
fn backing_mut(matrix: &mut Matrix) -> Result<&mut Backing, Stop> {
    match &mut matrix.allocation {
        Allocation::Known(b) => Ok(b),
        Allocation::Unavailable => Err(Stop::Unavailable("matrix backing unavailable".into())),
        _ => Err(Stop::Unsupported(
            "native array would access a null/released backing".into(),
        )),
    }
}
fn raw_span(matrix: &Matrix) -> Result<(usize, usize), Stop> {
    if matrix.width <= 0 || matrix.height < 0 || matrix.stride < 0 {
        return Err(Stop::Unsupported("unsafe raw matrix header".into()));
    }
    let backing = match &matrix.allocation {
        Allocation::Known(b) => b,
        Allocation::Unavailable => {
            return Err(Stop::Unavailable("raw matrix backing unavailable".into()));
        }
        _ => {
            return Err(Stop::Unsupported(
                "raw matrix lacks its preallocated backing".into(),
            ));
        }
    };
    let begin = *backing
        .column_offsets
        .first()
        .ok_or_else(|| Stop::Unsupported("raw first column absent".into()))?
        as usize;
    let last = *backing
        .column_offsets
        .get(matrix.width as usize - 1)
        .ok_or_else(|| {
            Stop::Unsupported("raw source dimensions exceed retained column table".into())
        })? as usize;
    let end = (matrix.height as usize)
        .checked_mul(matrix.stride as usize)
        .and_then(|n| last.checked_add(n))
        .ok_or_else(|| Stop::Unsupported("raw backing extent overflow".into()))?;
    if end < begin || end > backing.data.len() || end - begin > MAX_ALLOCATION_BYTES {
        return Err(Stop::Unsupported(
            "raw source dimensions do not fit retained allocation".into(),
        ));
    }
    Ok((begin, end))
}

///478909: reset used entries only, preserving vector extent and capacity tail.
pub fn reset_lights(used: usize, allocation: &mut [[u8; 4]]) -> Result<(), Stop> {
    let cells = allocation
        .get_mut(..used)
        .ok_or_else(|| Stop::Unsupported("light vector used extent exceeds allocation".into()))?;
    cells.fill([0x7f; 4]);
    Ok(())
}
