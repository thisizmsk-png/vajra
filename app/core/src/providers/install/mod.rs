//! ADR-17 step 2 (17b): one-click install, with consent, of a catalog harness whose source bhishma has verified.
//! Nothing here runs for an unverified entry: `setup::list` keeps it "Install it yourself". Every part is read-only
//! except `run`, which starts only with a fresh consent token for the exact plan the user saw.

pub mod admin;
pub mod closure;
pub mod env;
pub mod tar;
pub mod tools;
