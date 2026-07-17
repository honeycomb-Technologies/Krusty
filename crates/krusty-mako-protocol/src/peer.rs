use std::io;
use std::os::fd::AsRawFd;

use tokio::net::UnixStream;

use crate::PeerError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerIdentity {
    pub uid: u32,
    pub gid: u32,
    /// Linux exposes the peer PID; BSD/macOS `getpeereid` does not.
    pub pid: Option<u32>,
}

pub fn current_effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and does not dereference memory.
    unsafe { libc::geteuid() }
}

pub fn verify_same_user(stream: &UnixStream) -> Result<PeerIdentity, PeerError> {
    let peer = peer_identity(stream)?;
    let expected = current_effective_uid();
    if peer.uid != expected {
        return Err(PeerError::DifferentUser {
            expected,
            actual: peer.uid,
        });
    }
    Ok(peer)
}

#[cfg(target_os = "linux")]
pub fn peer_identity(stream: &UnixStream) -> Result<PeerIdentity, PeerError> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: all pointers refer to valid writable storage for `length`, and
    // the file descriptor belongs to a live Unix-domain socket.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(PeerError::Io(io::Error::last_os_error()));
    }
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(PeerError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            "SO_PEERCRED returned an unexpected credential size",
        )));
    }

    Ok(PeerIdentity {
        uid: credentials.uid,
        gid: credentials.gid,
        pid: u32::try_from(credentials.pid).ok(),
    })
}

#[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd"))]
pub fn peer_identity(stream: &UnixStream) -> Result<PeerIdentity, PeerError> {
    let mut uid: libc::uid_t = 0;
    let mut gid: libc::gid_t = 0;
    // SAFETY: `uid` and `gid` are valid output pointers and the descriptor is a
    // live Unix-domain socket. `getpeereid` does not retain the pointers.
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if result != 0 {
        return Err(PeerError::Io(io::Error::last_os_error()));
    }
    Ok(PeerIdentity {
        uid,
        gid,
        pid: None,
    })
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd"
)))]
pub fn peer_identity(_stream: &UnixStream) -> Result<PeerIdentity, PeerError> {
    Err(PeerError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use tokio::net::UnixListener;

    use super::*;

    #[tokio::test]
    async fn verifies_both_ends_as_the_current_user() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("peer.sock");
        let listener = UnixListener::bind(&path).unwrap();

        let connect = tokio::spawn(async move { UnixStream::connect(path).await.unwrap() });
        let (server_stream, _) = listener.accept().await.unwrap();
        let client_stream = connect.await.unwrap();

        let server_peer = verify_same_user(&server_stream).unwrap();
        let client_peer = verify_same_user(&client_stream).unwrap();
        assert_eq!(server_peer.uid, current_effective_uid());
        assert_eq!(client_peer.uid, current_effective_uid());
    }
}
