//! Projects (ADR-15 section 1): a project is a folder the owner added, and the unit of navigation and permission. It is
//! not a sandbox. `contract` is the versioned record format every other part builds on; the store and the migration of
//! old campaigns come after it.

pub mod contract;
pub mod identity;
pub mod migrate;
pub mod store;
