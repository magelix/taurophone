/// macOS grants these per build, and without them the app fails silently.
#[derive(Clone, serde::Serialize)]
pub struct Permissions {
    pub input_monitoring: bool,
    pub accessibility: bool,
}

pub fn check() -> Permissions {
    Permissions {
        input_monitoring: platform::can_listen_to_events(),
        accessibility: platform::can_post_events(),
    }
}

/// Opens the macOS dialogs for the permissions that are missing.
pub fn request_missing(needs_input_monitoring: bool) {
    if needs_input_monitoring && !platform::can_listen_to_events() {
        platform::request_listen_access();
    }
    if !platform::can_post_events() {
        platform::request_post_access();
    }
}

pub fn can_post_events() -> bool {
    platform::can_post_events()
}

#[cfg(target_os = "macos")]
mod platform {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
        fn CGPreflightPostEventAccess() -> bool;
        fn CGRequestPostEventAccess() -> bool;
    }

    pub fn can_listen_to_events() -> bool {
        unsafe { CGPreflightListenEventAccess() }
    }

    pub fn request_listen_access() {
        unsafe {
            CGRequestListenEventAccess();
        }
    }

    pub fn can_post_events() -> bool {
        unsafe { CGPreflightPostEventAccess() }
    }

    pub fn request_post_access() {
        unsafe {
            CGRequestPostEventAccess();
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn can_listen_to_events() -> bool {
        true
    }

    pub fn request_listen_access() {}

    pub fn can_post_events() -> bool {
        true
    }

    pub fn request_post_access() {}
}
