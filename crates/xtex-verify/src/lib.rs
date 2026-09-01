//! The verifier: the ONE place this project touches the network.
//!
//! Fully decoupled from the deterministic compiler — compilation and
//! checking never wait on this crate, never invoke it, and stay
//! reproducible without it. Its sole output is the dated record
//! (`.xtexverified`) that the offline check reads. Excluded from the
//! workspace so its dependencies never reach the compiler's lock file.

pub mod bucket;
pub mod compare;
pub mod run;
pub mod sources;
pub mod transport;

pub(crate) use xtex_core::verification::civil_days as civil;
