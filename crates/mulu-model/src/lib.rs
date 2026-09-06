//! `mulu-model` — the versioned input format of mulu's finite core.
//!
//! * [`schema`]   — `finite-product` v1 (serde types, unknown fields rejected)
//! * [`validate`] — id ranges, duplicates, determinism, unknown enums
//! * [`core`]     — normalisation to the integer-indexed `CoreModel` the Lean
//!   worker reads, plus the reverse name maps
//! * [`reference`] — slow, obviously-correct reference algorithms used to
//!   cross-check the worker (bug detection, *not* a proof)
//! * [`hash`]     — sha256 over bytes / canonical JSON

pub mod core;
pub mod hash;
pub mod reference;
pub mod schema;
pub mod validate;

pub use core::{CoreModel, Normalized};
pub use schema::{Control, FiniteProduct};
pub use validate::ValidationError;
