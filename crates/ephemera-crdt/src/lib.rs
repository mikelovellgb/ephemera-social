//! Conflict-free replicated data types (CRDTs) for the Ephemera platform.
//!
//! Provides convergent state types that can be merged across nodes without
//! coordination. Every CRDT in this module satisfies:
//!
//! - **Commutativity:** `merge(a, b) == merge(b, a)`
//! - **Associativity:** `merge(merge(a, b), c) == merge(a, merge(b, c))`
//! - **Idempotency:** `merge(a, a) == a`

mod clock;
mod gcounter;
mod lwwregister;
mod orset;

pub use clock::HybridClock;
pub use gcounter::GCounter;
pub use lwwregister::LwwRegister;
pub use orset::OrSet;
