//! Version 1: u32 LE body length, u8 version, u8 tag, then LE fields.
//! Length excludes the four-byte prefix; the complete frame is at most 16 KiB.
//! There are no optional trailing fields.
//! A stream must be retired after any codec error, including a partial write.

use std::io::{self, Read, Write};

pub(crate) const MAX_FRAME_BYTES: usize = 16 * 1024;
pub(crate) const MAX_NAME_BYTES: usize = 4096;
const MAX_BODY_BYTES: usize = MAX_FRAME_BYTES - 4;
const VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    Hello,
    Create { op: u64, name: Vec<u8> },
    Close { op: u64, handle: u64 },
    Check { op: u64, handle: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    Hello {
        epoch: [u8; 16],
        connection: u64,
        uid: u32,
        session: Option<u32>,
    },
    Created {
        op: u64,
        handle: u64,
        object: u64,
    },
    Rejected {
        op: u64,
        code: u32,
    },
    Closed {
        op: u64,
    },
    Live {
        op: u64,
        object: u64,
    },
    Unavailable {
        op: u64,
        code: u32,
    },
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn push_u64(body: &mut Vec<u8>, n: u64) {
    body.extend_from_slice(&n.to_le_bytes());
}
fn push_u32(body: &mut Vec<u8>, n: u32) {
    body.extend_from_slice(&n.to_le_bytes());
}

fn write_frame<W: Write>(writer: &mut W, body: &[u8]) -> io::Result<()> {
    if !(2..=MAX_BODY_BYTES).contains(&body.len()) {
        return Err(input("frame length"));
    }
    writer.write_all(&(body.len() as u32).to_le_bytes())?;
    writer.write_all(body)
}

fn read_frame<R: Read>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut length = [0; 4];
    loop {
        match reader.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    reader.read_exact(&mut length[1..])?;
    let length = u32::from_le_bytes(length) as usize;
    // Reject before allocating an attacker-selected length.
    if !(2..=MAX_BODY_BYTES).contains(&length) {
        return Err(invalid("frame length"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    if body[0] != VERSION {
        return Err(invalid("protocol version"));
    }
    Ok(Some(body))
}

struct Fields<'a> {
    bytes: &'a [u8],
}

impl<'a> Fields<'a> {
    fn take<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        if self.bytes.len() < N {
            return Err(invalid("missing field"));
        }
        let (value, rest) = self.bytes.split_at(N);
        self.bytes = rest;
        let mut result = [0; N];
        result.copy_from_slice(value);
        Ok(result)
    }
    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take::<1>()?[0])
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take()?))
    }
    fn id(&mut self) -> io::Result<u64> {
        let id = self.u64()?;
        if id == 0 {
            return Err(invalid("zero operation or identity"));
        }
        Ok(id)
    }
    fn end(self) -> io::Result<()> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(invalid("trailing fields"))
        }
    }
}

pub(crate) fn write_request<W: Write>(writer: &mut W, request: &Request) -> io::Result<()> {
    let mut body = vec![VERSION];
    match request {
        Request::Hello => body.push(1),
        Request::Create { op, name } => {
            if *op == 0 || name.len() > MAX_NAME_BYTES {
                return Err(input("create bounds"));
            }
            body.push(2);
            push_u64(&mut body, *op);
            push_u32(&mut body, name.len() as u32);
            body.extend_from_slice(name);
        }
        Request::Close { op, handle } | Request::Check { op, handle } => {
            if *op == 0 {
                return Err(input("zero operation"));
            }
            body.push(if matches!(request, Request::Close { .. }) {
                3
            } else {
                4
            });
            push_u64(&mut body, *op);
            push_u64(&mut body, *handle);
        }
    }
    write_frame(writer, &body)
}

pub(crate) fn read_request<R: Read>(reader: &mut R) -> io::Result<Option<Request>> {
    let Some(body) = read_frame(reader)? else {
        return Ok(None);
    };
    let mut fields = Fields { bytes: &body[1..] };
    let request = match fields.u8()? {
        1 => Request::Hello,
        2 => {
            let op = fields.id()?;
            let length = fields.u32()? as usize;
            if length > MAX_NAME_BYTES || fields.bytes.len() != length {
                return Err(invalid("name length"));
            }
            let name = fields.bytes.to_vec();
            fields.bytes = &[];
            Request::Create { op, name }
        }
        3 => Request::Close {
            op: fields.id()?,
            handle: fields.u64()?,
        },
        4 => Request::Check {
            op: fields.id()?,
            handle: fields.u64()?,
        },
        _ => return Err(invalid("request tag")),
    };
    fields.end()?;
    Ok(Some(request))
}

pub(crate) fn write_reply<W: Write>(writer: &mut W, reply: &Reply) -> io::Result<()> {
    let mut body = vec![VERSION];
    match reply {
        Reply::Hello {
            epoch,
            connection,
            uid,
            session,
        } => {
            if *connection == 0 {
                return Err(input("zero connection"));
            }
            body.push(0x81);
            body.extend_from_slice(epoch);
            push_u64(&mut body, *connection);
            push_u32(&mut body, *uid);
            body.push(u8::from(session.is_some()));
            if let Some(session) = session {
                push_u32(&mut body, *session);
            }
        }
        Reply::Created { op, handle, object } => {
            if *op == 0 || *handle == 0 || *object == 0 {
                return Err(input("zero created identity"));
            }
            body.push(0x82);
            push_u64(&mut body, *op);
            push_u64(&mut body, *handle);
            push_u64(&mut body, *object);
        }
        Reply::Rejected { op, code } | Reply::Unavailable { op, code } => {
            if *op == 0 {
                return Err(input("zero operation"));
            }
            body.push(if matches!(reply, Reply::Rejected { .. }) {
                0x83
            } else {
                0x86
            });
            push_u64(&mut body, *op);
            push_u32(&mut body, *code);
        }
        Reply::Closed { op } => {
            if *op == 0 {
                return Err(input("zero operation"));
            }
            body.push(0x84);
            push_u64(&mut body, *op);
        }
        Reply::Live { op, object } => {
            if *op == 0 || *object == 0 {
                return Err(input("zero live identity"));
            }
            body.push(0x85);
            push_u64(&mut body, *op);
            push_u64(&mut body, *object);
        }
    }
    write_frame(writer, &body)
}

pub(crate) fn read_reply<R: Read>(reader: &mut R) -> io::Result<Option<Reply>> {
    let Some(body) = read_frame(reader)? else {
        return Ok(None);
    };
    let mut fields = Fields { bytes: &body[1..] };
    let reply = match fields.u8()? {
        0x81 => {
            let epoch = fields.take()?;
            let connection = fields.id()?;
            let uid = fields.u32()?;
            let session = match fields.u8()? {
                0 => None,
                1 => Some(fields.u32()?),
                _ => return Err(invalid("session discriminant")),
            };
            Reply::Hello {
                epoch,
                connection,
                uid,
                session,
            }
        }
        0x82 => Reply::Created {
            op: fields.id()?,
            handle: fields.id()?,
            object: fields.id()?,
        },
        0x83 => Reply::Rejected {
            op: fields.id()?,
            code: fields.u32()?,
        },
        0x84 => Reply::Closed { op: fields.id()? },
        0x85 => Reply::Live {
            op: fields.id()?,
            object: fields.id()?,
        },
        0x86 => Reply::Unavailable {
            op: fields.id()?,
            code: fields.u32()?,
        },
        _ => return Err(invalid("reply tag")),
    };
    fields.end()?;
    Ok(Some(reply))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn all_requests_roundtrip_and_concatenate() {
        let requests = [
            Request::Hello,
            Request::Create {
                op: 1,
                name: b"Global\\MiXeD".to_vec(),
            },
            Request::Create {
                op: 2,
                name: vec![0xff; MAX_NAME_BYTES],
            },
            Request::Close { op: 3, handle: 0 },
            Request::Check {
                op: u64::MAX,
                handle: u64::MAX,
            },
        ];
        let mut wire = Vec::new();
        for request in &requests {
            write_request(&mut wire, request).unwrap();
        }
        let mut wire = Cursor::new(wire);
        for request in requests {
            assert_eq!(read_request(&mut wire).unwrap(), Some(request));
        }
        assert_eq!(read_request(&mut wire).unwrap(), None);
    }

    #[test]
    fn all_replies_roundtrip() {
        let replies = [
            Reply::Hello {
                epoch: [0xa5; 16],
                connection: 1,
                uid: 0,
                session: None,
            },
            Reply::Hello {
                epoch: [1; 16],
                connection: 2,
                uid: 501,
                session: Some(42),
            },
            Reply::Created {
                op: 1,
                handle: 2,
                object: 3,
            },
            Reply::Rejected { op: 2, code: 5 },
            Reply::Closed { op: 3 },
            Reply::Live { op: 4, object: 3 },
            Reply::Unavailable { op: 5, code: 1 },
        ];
        for reply in replies {
            let mut wire = Vec::new();
            write_reply(&mut wire, &reply).unwrap();
            assert_eq!(read_reply(&mut wire.as_slice()).unwrap(), Some(reply));
        }
    }

    #[test]
    fn every_partial_frame_is_an_error_but_boundary_eof_is_none() {
        let mut wire = Vec::new();
        write_request(
            &mut wire,
            &Request::Create {
                op: 1,
                name: b"abc".to_vec(),
            },
        )
        .unwrap();
        assert!(read_request(&mut &wire[..0]).unwrap().is_none());
        for length in 1..wire.len() {
            assert!(read_request(&mut &wire[..length]).is_err(), "{length}");
        }
        wire.clear();
        write_reply(
            &mut wire,
            &Reply::Hello {
                epoch: [3; 16],
                connection: 8,
                uid: 1,
                session: Some(4),
            },
        )
        .unwrap();
        assert_eq!(read_reply(&mut &wire[..0]).unwrap(), None);
        for length in 1..wire.len() {
            assert!(read_reply(&mut &wire[..length]).is_err(), "{length}");
        }
    }

    #[test]
    fn wire_version_tags_and_integer_endianness_are_fixed() {
        let mut wire = Vec::new();
        write_request(&mut wire, &Request::Hello).unwrap();
        assert_eq!(wire, [2, 0, 0, 0, 1, 1]);
        wire.clear();
        write_request(
            &mut wire,
            &Request::Create {
                op: 0x0807060504030201,
                name: b"X".to_vec(),
            },
        )
        .unwrap();
        assert_eq!(
            wire,
            [15, 0, 0, 0, 1, 2, 1, 2, 3, 4, 5, 6, 7, 8, 1, 0, 0, 0, b'X']
        );
    }

    #[test]
    fn malformed_version_tags_lengths_and_trailing_fields_fail() {
        for body in [
            vec![2, 1],
            vec![1, 0],
            vec![1, 1, 0],
            vec![1, 0x81],
            vec![1],
        ] {
            let mut wire = (body.len() as u32).to_le_bytes().to_vec();
            wire.extend_from_slice(&body);
            assert!(read_request(&mut wire.as_slice()).is_err());
        }
        for length in [
            0u32,
            1,
            (MAX_BODY_BYTES + 1) as u32,
            (MAX_FRAME_BYTES + 1) as u32,
            u32::MAX,
        ] {
            assert!(read_request(&mut length.to_le_bytes().as_slice()).is_err());
        }
        let mut body = vec![1, 2];
        push_u64(&mut body, 1);
        push_u32(&mut body, (MAX_NAME_BYTES + 1) as u32);
        body.resize(body.len() + MAX_NAME_BYTES + 1, b'a');
        let mut wire = Vec::new();
        write_frame(&mut wire, &body).unwrap();
        assert!(read_request(&mut wire.as_slice()).is_err());
    }

    #[test]
    fn invalid_reply_fields_and_zero_ops_fail_without_partial_write() {
        let mut wire = Vec::new();
        assert!(write_request(&mut wire, &Request::Check { op: 0, handle: 1 }).is_err());
        assert!(wire.is_empty());
        assert!(
            write_request(
                &mut wire,
                &Request::Create {
                    op: 1,
                    name: vec![0; MAX_NAME_BYTES + 1]
                }
            )
            .is_err()
        );
        assert!(wire.is_empty());
        let reply = Reply::Hello {
            epoch: [1; 16],
            connection: 1,
            uid: 1,
            session: None,
        };
        write_reply(&mut wire, &reply).unwrap();
        *wire.last_mut().unwrap() = 2;
        assert!(read_reply(&mut wire.as_slice()).is_err());
        wire.clear();
        write_request(&mut wire, &Request::Close { op: 1, handle: 1 }).unwrap();
        wire[6..14].fill(0);
        assert!(read_request(&mut wire.as_slice()).is_err());
    }

    #[test]
    fn replies_reject_unknown_tags_missing_fields_trailing_bytes_and_zero_ids() {
        for body in [vec![1, 0], vec![1, 0x82], vec![2, 0x84]] {
            let mut wire = (body.len() as u32).to_le_bytes().to_vec();
            wire.extend_from_slice(&body);
            assert!(read_reply(&mut wire.as_slice()).is_err());
        }
        let mut body = vec![1, 0x84];
        push_u64(&mut body, 1);
        body.push(0);
        let mut wire = Vec::new();
        write_frame(&mut wire, &body).unwrap();
        assert!(read_reply(&mut wire.as_slice()).is_err());
        body = vec![1, 0x82];
        push_u64(&mut body, 1);
        push_u64(&mut body, 0);
        push_u64(&mut body, 2);
        wire.clear();
        write_frame(&mut wire, &body).unwrap();
        assert!(read_reply(&mut wire.as_slice()).is_err());
    }

    struct ShortReader<'a> {
        bytes: &'a [u8],
        interrupt: bool,
    }
    impl Read for ShortReader<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if self.interrupt {
                self.interrupt = false;
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.interrupt = true;
            if self.bytes.is_empty() || output.is_empty() {
                return Ok(0);
            }
            output[0] = self.bytes[0];
            self.bytes = &self.bytes[1..];
            Ok(1)
        }
    }

    #[test]
    fn fragmented_and_interrupted_reads_keep_framing() {
        let request = Request::Create {
            op: 7,
            name: b"Local\\exact".to_vec(),
        };
        let mut wire = Vec::new();
        write_request(&mut wire, &request).unwrap();
        let mut reader = ShortReader {
            bytes: &wire,
            interrupt: true,
        };
        assert_eq!(read_request(&mut reader).unwrap(), Some(request));
        assert_eq!(read_request(&mut reader).unwrap(), None);
    }
}
