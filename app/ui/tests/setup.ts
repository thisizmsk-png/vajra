// The desktop app ships on macOS, so the suite runs as a Mac whatever the CI host is: the platform modifier
// (`Primary`) is Command here. A test of another platform passes `mac = false` to `matchesShortcut`.
// A test file that opts into the node environment (the build test) has no window.
if (typeof window !== "undefined") Object.defineProperty(window.navigator, "userAgent", { configurable: true, value: "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/537.36 (KHTML, like Gecko) jsdom" });
