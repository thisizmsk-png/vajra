//! Vajra Desktop core.

pub mod logging;
pub mod paths;

// Each of these is owned by a later task. Until its src/<name>/mod.rs
// exists, build.rs leaves the cfg unset and the module is empty.
#[cfg(has_mod_bus)]
pub mod bus;
#[cfg(not(has_mod_bus))]
pub mod bus {}

#[cfg(has_mod_sessions)]
pub mod sessions;
#[cfg(not(has_mod_sessions))]
pub mod sessions {}

#[cfg(has_mod_vajra)]
pub mod vajra;
#[cfg(not(has_mod_vajra))]
pub mod vajra {}

#[cfg(has_mod_crew)]
pub mod crew;
#[cfg(not(has_mod_crew))]
pub mod crew {}

#[cfg(has_mod_control)]
pub mod control;
#[cfg(not(has_mod_control))]
pub mod control {}
