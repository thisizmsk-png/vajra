//! ADR-9 item 7: while any run is active, hold a macOS activity assertion so
//! App Nap does not throttle the windowless app. Released when the last run ends.

/// Begins and ends one activity. `Token` is whatever the OS hands back.
pub trait Activity {
    type Token;
    fn begin(&mut self) -> Self::Token;
    fn end(&mut self, token: Self::Token);
}

pub struct NapGuard<A: Activity> {
    os: A,
    held: Option<A::Token>,
}

impl<A: Activity> NapGuard<A> {
    pub fn new(os: A) -> Self {
        NapGuard { os, held: None }
    }

    /// Call with the current active run count; idempotent.
    pub fn update(&mut self, active_runs: usize) {
        match (active_runs > 0, self.held.take()) {
            (true, None) => self.held = Some(self.os.begin()),
            (true, Some(t)) => self.held = Some(t),
            (false, Some(t)) => self.os.end(t),
            (false, None) => {}
        }
    }

    pub fn held(&self) -> bool {
        self.held.is_some()
    }
}

#[cfg(target_os = "macos")]
pub mod macos {
    use objc2::rc::Retained;
    use objc2::runtime::{NSObjectProtocol, ProtocolObject};
    use objc2_foundation::{NSActivityOptions, NSProcessInfo, NSString};

    pub struct ProcessInfo;

    impl super::Activity for ProcessInfo {
        type Token = Retained<ProtocolObject<dyn NSObjectProtocol>>;
        fn begin(&mut self) -> Self::Token {
            // User-initiated work; the system may still sleep when idle.
            let reason = NSString::from_str("Vajra Desktop runs are active");
            NSProcessInfo::processInfo()
                .beginActivityWithOptions_reason(NSActivityOptions::UserInitiatedAllowingIdleSystemSleep, &reason)
        }
        fn end(&mut self, token: Self::Token) {
            // SAFETY: the token came from beginActivityWithOptions_reason on this process.
            unsafe { NSProcessInfo::processInfo().endActivity(&token) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake {
        begun: u32,
        ended: u32,
    }
    impl Activity for &mut Fake {
        type Token = u32;
        fn begin(&mut self) -> u32 {
            self.begun += 1;
            self.begun
        }
        fn end(&mut self, t: u32) {
            assert_eq!(t, self.begun, "ends the token it began");
            self.ended += 1;
        }
    }

    #[test]
    fn held_while_any_run_is_active_and_released_after_the_last() {
        let mut f = Fake::default();
        {
            let mut g = NapGuard::new(&mut f);
            g.update(0);
            assert!(!g.held());
            g.update(1);
            g.update(3);
            g.update(1);
            assert!(g.held());
            g.update(0);
            g.update(0);
            assert!(!g.held());
            g.update(2);
        }
        assert_eq!((f.begun, f.ended), (2, 1), "one assertion per active period");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_assertion_begins_and_ends() {
        let mut g = NapGuard::new(macos::ProcessInfo);
        g.update(1);
        assert!(g.held());
        g.update(0);
        assert!(!g.held());
    }
}
