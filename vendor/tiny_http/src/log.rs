// (Landscaper patch: no `error`, which only the accept thread's end used; it doesn't end on an error now.)
#[cfg(feature = "log")]
pub(crate) use log::{debug, warn};

#[cfg(not(feature = "log"))]
macro_rules! _debug {
    (target: $target:expr, $($arg:tt)+) => {};
    ($($arg:tt)+) => {};
}

#[cfg(not(feature = "log"))]
macro_rules! _warn {
    (target: $target:expr, $($arg:tt)+) => {};
    ($($arg:tt)+) => {};
}

#[cfg(not(feature = "log"))]
pub(crate) use {_debug as debug, _warn as warn};
