use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

/// Get the peer's UID from a Unix domain socket.
/// Returns None if the platform doesn't support it or if the call fails.
pub(crate) fn get_peer_uid(stream: &UnixStream) -> Option<u32> {
    #[allow(unused_variables)]
    let fd = stream.as_raw_fd();

    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let ret = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut libc::c_void,
                &mut len,
            )
        };
        if ret == 0 { Some(cred.uid) } else { None }
    }

    #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "openbsd"))]
    {
        // macOS/BSD: getpeereid() syscall
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        let ret = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        if ret == 0 { Some(uid) } else { None }
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd"
    )))]
    {
        let _ = stream;
        None
    }
}

/// Resolve a UID to a username.
pub(crate) fn get_username_for_uid(uid: u32) -> String {
    let mut buf = vec![0u8; 256];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };

    let ret = unsafe {
        libc::getpwuid_r(
            uid,
            &mut pwd,
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            &mut result,
        )
    };
    if ret == 0 && !result.is_null() {
        let name_ptr = unsafe { (*result).pw_name };
        if !name_ptr.is_null() {
            return unsafe { std::ffi::CStr::from_ptr(name_ptr) }
                .to_string_lossy()
                .into_owned();
        }
    }
    // Fallback: return uid as string
    uid.to_string()
}
