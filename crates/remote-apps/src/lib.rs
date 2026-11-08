//! Seamless remote app windows.
//!
//! The host runs `wprsd` (a rootless Wayland compositor) with
//! `xwayland-xdg-shell`, so X11 apps use a normal `DISPLAY`. A bridge on the
//! host connects to `wprsd` as its client and speaks the Terminator display
//! protocol ([`protocol`]) over the SSH channel. The local daemon owns that
//! channel; the GUI renders each remote toplevel as a tab.
//!
//! The wprs protocol (rkyv) is not stable across wprs versions, so it never
//! leaves the host. Only [`protocol`] crosses the network, and it is
//! versioned.

#[cfg(all(target_os = "linux", feature = "bridge"))]
pub mod bridge;
pub mod compose;
pub mod protocol;
