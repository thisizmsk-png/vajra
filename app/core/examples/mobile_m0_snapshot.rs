//! Manual export harness: stdin is a bounded, local typed read model. No user HOME/state is opened.
use std::io::{self, Read};
use vajra_desktop::projects::contract::{Project, Session};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    io::stdin().take(256 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 256 * 1024 {
        return Err("input_limit".into());
    }
    let v: serde_json::Value = serde_json::from_slice(&bytes)?;
    let projects: Vec<Project> = serde_json::from_value(v["projects"].clone())?;
    let sessions: Vec<Session> = serde_json::from_value(v["sessions"].clone())?;
    let scope: Vec<String> = serde_json::from_value(v["scope"].clone())?;
    let attention: Vec<(String, u32)> = serde_json::from_value(v["attention"].clone())?;
    println!(
        "{}",
        vajra_desktop::mobile_m0::snapshot(&projects, &sessions, &attention, &scope)?
    );
    Ok(())
}
