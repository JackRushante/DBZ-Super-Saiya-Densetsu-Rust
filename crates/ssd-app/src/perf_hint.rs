//! The Android Performance Hint API (ADPF) of the window loop. Each window frame reports how long its work took,
//! against the window's period as the target, so the system keeps the cores at the speed the frame needs.
//!
//! The numbers are computed here on every platform and tested on the host. The calls into `libandroid.so` exist on
//! Android only. They are looked up at run time, because the API is level 33 and the app runs from level 26: where a
//! symbol is missing no session is made and the reports do nothing. Elsewhere a session only keeps its target.

use std::time::Duration;

/// A duration in nanoseconds, saturated to the range of the API's `int64_t`.
fn nanos(time: Duration) -> i64 {
    i64::try_from(time.as_nanos()).unwrap_or(i64::MAX)
}

/// The target work duration for a window whose frames have the period `period`: the period itself, which the API
/// requires to be positive.
fn target_nanos(period: Duration) -> i64 {
    nanos(period).max(1)
}

/// The Performance Hint session of the window loop: made when the loop starts, closed when it ends.
pub struct Session {
    /// The target work duration in nanoseconds that the session has now.
    target_ns: i64,
    #[cfg(target_os = "android")]
    live: Option<android::Live>,
}

impl Session {
    /// A session for a window whose frames have the period `period`, on this thread and the threads of rayon's global
    /// pool (the pool that draws the pictures). Empty when the system has no such API or refuses the session.
    #[must_use]
    pub fn new(period: Duration) -> Self {
        let target_ns = target_nanos(period);
        Self {
            target_ns,
            #[cfg(target_os = "android")]
            live: android::Live::open(target_ns),
        }
    }

    /// Ends a window frame: `work` is its work duration, from its start to just before the present; `period` is the
    /// window's period, which is the target (a change of it is passed on to the session).
    pub fn report_frame(&mut self, work: Duration, period: Duration) {
        let target_ns = target_nanos(period);
        if target_ns != self.target_ns {
            self.target_ns = target_ns;
            #[cfg(target_os = "android")]
            if let Some(live) = &self.live {
                live.update_target(target_ns);
            }
        }
        #[cfg(target_os = "android")]
        if let Some(live) = &self.live {
            live.report(nanos(work));
        }
        #[cfg(not(target_os = "android"))]
        let _ = work;
    }
}

#[cfg(target_os = "android")]
mod android {
    use std::ffi::{CStr, c_int, c_void};
    use std::sync::OnceLock;

    // The entry points of `libandroid.so` (`performance_hint.h`). The manager and the session are opaque pointers.
    type GetManager = unsafe extern "C" fn() -> *mut c_void;
    type CreateSession = unsafe extern "C" fn(*mut c_void, *const i32, usize, i64) -> *mut c_void;
    type UpdateTarget = unsafe extern "C" fn(*mut c_void, i64) -> c_int;
    type ReportWork = unsafe extern "C" fn(*mut c_void, i64) -> c_int;
    type CloseSession = unsafe extern "C" fn(*mut c_void);

    struct Api {
        get_manager: GetManager,
        create_session: CreateSession,
        update_target: UpdateTarget,
        report_work: ReportWork,
        close_session: CloseSession,
    }

    static API: OnceLock<Option<Api>> = OnceLock::new();

    /// The entry points, looked up once: `None` unless the system has all of them.
    fn api() -> Option<&'static Api> {
        API.get_or_init(load).as_ref()
    }

    fn load() -> Option<Api> {
        // SAFETY: opens the system library, which the process has loaded already (SDL uses it). The handle is kept.
        let lib = unsafe { libc::dlopen(c"libandroid.so".as_ptr(), libc::RTLD_NOW) };
        if lib.is_null() {
            return None;
        }
        let symbol = |name: &CStr| {
            // SAFETY: `lib` is a handle from `dlopen` and `name` is NUL-terminated.
            let ptr = unsafe { libc::dlsym(lib, name.as_ptr()) };
            (!ptr.is_null()).then_some(ptr)
        };
        // SAFETY: a function pointer and a data pointer have the same size, and each symbol has the signature of its
        // declaration above, which is the one in `performance_hint.h`.
        unsafe {
            Some(Api {
                get_manager: std::mem::transmute::<*mut c_void, GetManager>(symbol(
                    c"APerformanceHint_getManager",
                )?),
                create_session: std::mem::transmute::<*mut c_void, CreateSession>(symbol(
                    c"APerformanceHint_createSession",
                )?),
                update_target: std::mem::transmute::<*mut c_void, UpdateTarget>(symbol(
                    c"APerformanceHint_updateTargetWorkDuration",
                )?),
                report_work: std::mem::transmute::<*mut c_void, ReportWork>(symbol(
                    c"APerformanceHint_reportActualWorkDuration",
                )?),
                close_session: std::mem::transmute::<*mut c_void, CloseSession>(symbol(
                    c"APerformanceHint_closeSession",
                )?),
            })
        }
    }

    /// A session of the system, made on the window loop's thread.
    pub struct Live {
        session: *mut c_void,
        api: &'static Api,
    }

    impl Live {
        /// The session, or `None` when the system has no such API or refuses it.
        pub fn open(target_ns: i64) -> Option<Self> {
            let api = api()?;
            // SAFETY: a call into `libandroid.so` without arguments.
            let manager = unsafe { (api.get_manager)() };
            if manager.is_null() {
                return None;
            }
            // This thread runs the window's frames, and the pool draws the pictures. The workers of the global pool live
            // as long as the process, as a session needs. `broadcast` makes the pool first when the first picture has
            // not, with the pool's usual size, and returns once every worker has reported its id.
            // SAFETY: `gettid` takes no argument and cannot fail.
            let mut tids = vec![unsafe { libc::gettid() }];
            tids.extend(rayon::broadcast(|_| unsafe { libc::gettid() }));
            // SAFETY: `tids` holds `len` thread ids of this process, and it lives across the call.
            let session =
                unsafe { (api.create_session)(manager, tids.as_ptr(), tids.len(), target_ns) };
            (!session.is_null()).then_some(Self { session, api })
        }

        /// Sets the target work duration of the session, in nanoseconds.
        pub fn update_target(&self, target_ns: i64) {
            // SAFETY: the session is open until it is dropped. The result (0 on success) is not used.
            unsafe { (self.api.update_target)(self.session, target_ns) };
        }

        /// Reports the work duration of a window frame, in nanoseconds.
        pub fn report(&self, work_ns: i64) {
            // SAFETY: as in `update_target`.
            unsafe { (self.api.report_work)(self.session, work_ns) };
        }
    }

    impl Drop for Live {
        fn drop(&mut self) {
            // SAFETY: the session was made by `open` and is closed once, here.
            unsafe { (self.api.close_session)(self.session) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_target_is_the_period_in_nanoseconds() {
        assert_eq!(target_nanos(Duration::from_nanos(16_666_667)), 16_666_667);
        // The emulator's clock at the default NTSC timing (60.0988 Hz) is 16.64 ms a frame.
        let default_period = Duration::from_secs_f64(1.0 / 60.0988);
        assert!((16_630_000..=16_650_000).contains(&target_nanos(default_period)));
    }

    #[test]
    fn the_target_is_positive_and_saturates() {
        assert_eq!(target_nanos(Duration::ZERO), 1);
        assert_eq!(target_nanos(Duration::from_secs(u64::MAX)), i64::MAX);
        assert_eq!(nanos(Duration::from_millis(5)), 5_000_000);
    }

    #[test]
    fn a_session_takes_the_period_of_its_window_as_its_target() {
        let period = Duration::from_micros(16_639);
        let mut session = Session::new(period);
        assert_eq!(session.target_ns, 16_639_000);
        session.report_frame(Duration::from_millis(4), period);
        assert_eq!(session.target_ns, 16_639_000);
        session.report_frame(Duration::from_millis(4), Duration::from_millis(33));
        assert_eq!(session.target_ns, 33_000_000);
    }
}
