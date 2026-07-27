pub(crate) enum DaemonSignal {
    DetachClients,
    Shutdown,
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod platform {
    use std::sync::atomic::{AtomicBool, Ordering};

    static SIGHUP: AtomicBool = AtomicBool::new(false);
    static SHUTDOWN: AtomicBool = AtomicBool::new(false);

    extern "C" fn handle_sighup(_: libc::c_int) {
        SIGHUP.store(true, Ordering::SeqCst);
    }
    extern "C" fn handle_shutdown(_: libc::c_int) {
        SHUTDOWN.store(true, Ordering::SeqCst);
    }

    pub(crate) fn install() {
        unsafe {
            libc::signal(
                libc::SIGHUP,
                handle_sighup as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGTERM,
                handle_shutdown as *const () as libc::sighandler_t,
            );
            libc::signal(
                libc::SIGINT,
                handle_shutdown as *const () as libc::sighandler_t,
            );
        }
    }

    pub(crate) fn poll() -> Option<super::DaemonSignal> {
        if SHUTDOWN.swap(false, Ordering::SeqCst) {
            Some(super::DaemonSignal::Shutdown)
        } else if SIGHUP.swap(false, Ordering::SeqCst) {
            Some(super::DaemonSignal::DetachClients)
        } else {
            None
        }
    }
}

#[cfg(not(unix))]
mod platform {
    pub(crate) fn install() {}
    pub(crate) fn poll() -> Option<super::DaemonSignal> {
        None
    }
}

pub(crate) use platform::{install, poll};
