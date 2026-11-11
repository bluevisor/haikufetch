//! One backend per operating system, each with the same set of functions. The modules call
//! `platform::cpu()` and friends without knowing which one was compiled in.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "haiku")]
mod haiku;
#[cfg(target_os = "haiku")]
pub use haiku::*;
