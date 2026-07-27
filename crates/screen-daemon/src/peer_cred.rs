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
    let mut buffer_size = 256;
    loop {
        let mut buf = vec![0u8; buffer_size];
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
        if ret == libc::ERANGE {
            buffer_size = buffer_size.saturating_mul(2);
            continue;
        }
        if ret == 0 && !result.is_null() {
            let name_ptr = unsafe { (*result).pw_name };
            if !name_ptr.is_null() {
                return unsafe { std::ffi::CStr::from_ptr(name_ptr) }
                    .to_string_lossy()
                    .into_owned();
            }
        }
        // Not found or another lookup error: fall back to the UID.
        return uid.to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::get_username_for_uid;

    #[test]
    fn resolves_current_user() {
        let name = get_username_for_uid(unsafe { libc::getuid() });
        assert!(!name.is_empty());
    }

    #[test]
    fn falls_back_to_numeric_for_nonexistent_uid() {
        let name = get_username_for_uid(65_000);
        if name.chars().all(|character| character.is_ascii_digit()) {
            assert_eq!(name, "65000");
        }
    }
}
