//! Kernel-authenticated Unix credentials. No supplied identity or PID lookup.
//!
//! macOS SDK primary declarations: sys/un.h (SOL_LOCAL / LOCAL_PEERTOKEN),
//! unistd.h (getpeereid), mach/message.h (opaque eight-word audit_token_t),
//! bsm/libbsm.h (audit_token_to_euid / audit_token_to_asid), bsm/audit.h
//! (au_asid_t = pid_t; 0 is AU_DEFAUDITSID, -1 is AU_ASSIGN_ASID).
//! A missing audit token leaves session scope unavailable, never UID-as-session.

#![cfg(unix)]

use std::io;
use std::os::unix::net::UnixStream;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Peer {
    pub uid: u32,
    pub session: Option<u32>,
}

unsafe extern "C" {
    fn geteuid() -> u32;
}

pub(crate) fn current_uid() -> u32 {
    // SAFETY: geteuid has no arguments, owns no memory, and returns uid_t.
    unsafe { geteuid() }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::ffi::{c_int, c_void};
    use std::os::fd::AsRawFd;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct AuditToken {
        opaque: [u32; 8],
    }

    unsafe extern "C" {
        fn getpeereid(fd: c_int, uid: *mut u32, gid: *mut u32) -> c_int;
        fn getsockopt(
            fd: c_int,
            level: c_int,
            option: c_int,
            value: *mut c_void,
            length: *mut u32,
        ) -> c_int;
    }
    #[link(name = "bsm")]
    unsafe extern "C" {
        fn audit_token_to_asid(token: AuditToken) -> c_int;
        fn audit_token_to_euid(token: AuditToken) -> u32;
    }

    fn session_id(asid: c_int) -> Option<u32> {
        match asid {
            0 | -1 => None,
            value => Some(value as u32),
        }
    }

    pub(super) fn identity(stream: &UnixStream) -> io::Result<Peer> {
        let mut uid = 0;
        let mut gid = 0;
        // SAFETY: live borrowed socket and writable uid_t/gid_t storage.
        if unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut token = AuditToken { opaque: [0; 8] };
        let mut length = std::mem::size_of::<AuditToken>() as u32;
        // SDK: SOL_LOCAL = 0, LOCAL_PEERTOKEN = 6; socklen_t is u32.
        // SAFETY: correctly sized/aligned audit_token_t storage, valid socket.
        let status = unsafe {
            getsockopt(
                stream.as_raw_fd(),
                0,
                6,
                (&mut token as *mut AuditToken).cast(),
                &mut length,
            )
        };
        if status != 0 {
            return Ok(Peer { uid, session: None });
        }
        if length as usize != std::mem::size_of::<AuditToken>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "peer audit token size",
            ));
        }
        // SAFETY: token was filled by the kernel and validated for full size.
        let token_uid = unsafe { audit_token_to_euid(token) };
        if token_uid != uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "inconsistent peer credentials",
            ));
        }
        // SAFETY: the full kernel token is passed by value using its SDK ABI;
        // interpret its opaque representation only through libbsm.
        let session = session_id(unsafe { audit_token_to_asid(token) });
        Ok(Peer { uid, session })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn audit_sentinels_never_become_session_authority() {
            assert_eq!(session_id(0), None);
            assert_eq!(session_id(-1), None);
            assert_eq!(session_id(42), Some(42));
        }
    }
}

// Other Unix targets deliberately fail closed until their credential ABI has
// been qualified. In particular, no getuid/session stand-in authenticates peers.
pub(crate) fn peer_identity(stream: &UnixStream) -> io::Result<Peer> {
    #[cfg(target_os = "macos")]
    {
        platform::identity(stream)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = stream;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "peer credential authentication unavailable on this Unix target",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn peer_authentication_uses_kernel_or_is_unavailable() {
        let (left, _right) = UnixStream::pair().unwrap();
        #[cfg(target_os = "macos")]
        {
            let peer = peer_identity(&left).unwrap();
            assert_eq!(peer.uid, current_uid());
            assert_ne!(peer.session, Some(0));
            assert_ne!(peer.session, Some(u32::MAX));
        }
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            peer_identity(&left).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
}
