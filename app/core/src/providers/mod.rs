//! Provider selection and native login checks (FG-09). `contract` is the wire shape every surface reads; the detection
//! and login check come after it. Vajra never reads, stores or forwards a credential: the stock CLI owns login.

pub mod contract;
pub mod detect;
pub mod usage;
