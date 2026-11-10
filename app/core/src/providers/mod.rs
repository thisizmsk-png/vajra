//! Provider selection and native login checks (FG-09). `contract` is the wire shape every surface reads; the detection
//! and login check come after it. The stock CLI owns its login: Vajra never reads, stores or forwards a CLI credential.
//! The one exception (D3) is `keychain`: a pasted API key, written once into the Keychain and never read back.

pub mod codex_home;
pub mod contract;
pub mod detect;
pub mod errors;
pub mod keychain;
pub mod usage;
