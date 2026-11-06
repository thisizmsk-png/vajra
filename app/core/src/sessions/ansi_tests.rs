use super::sanitize;

#[test]
fn sessions_ansi_keeps_plain_text_and_tabs() {
    assert_eq!(sanitize("hello\tworld"), "hello\tworld");
    assert_eq!(sanitize("naïve ünïcode"), "naïve ünïcode");
}

#[test]
fn sessions_ansi_keeps_sgr() {
    let s = "\x1b[1;31mred\x1b[0m \x1b[38;2;255;0;0mtrue\x1b[m \x1b[38:5:196mx\x1b[0m";
    assert_eq!(sanitize(s), s);
}

#[test]
fn sessions_ansi_keeps_cursor_movement() {
    let s = "\x1b[A\x1b[2B\x1b[3C\x1b[4D\x1b[E\x1b[F\x1b[10G\x1b[5;7H\x1b[H\x1b[1;1f";
    assert_eq!(sanitize(s), s);
}

#[test]
fn sessions_ansi_drops_other_csi() {
    // Erase, private modes (alt screen, bracketed paste), device status report.
    assert_eq!(
        sanitize("a\x1b[2Jb\x1b[?1049hc\x1b[?2004ld\x1b[6ne"),
        "abcde"
    );
    // Intermediate bytes make it a different command even with final 'm'.
    assert_eq!(sanitize("a\x1b[1 mb"), "ab");
    // A private parameter on a cursor final is not plain movement.
    assert_eq!(sanitize("a\x1b[>1Ab"), "ab");
    // Colons are SGR sub-parameters only.
    assert_eq!(sanitize("a\x1b[1:2Hb"), "ab");
}

#[test]
fn sessions_ansi_drops_osc8_hyperlink() {
    let s = "see \x1b]8;;https://evil.example/\x1b\\link\x1b]8;;\x1b\\ end";
    assert_eq!(sanitize(s), "see link end");
}

#[test]
fn sessions_ansi_drops_osc52_clipboard() {
    assert_eq!(sanitize("a\x1b]52;c;cm0gLXJmIH4=\x07b"), "ab");
}

#[test]
fn sessions_ansi_drops_title() {
    assert_eq!(
        sanitize("a\x1b]0;pwned\x07b\x1b]2;x\x1b\\c\x1b]1;y\x07d"),
        "abcd"
    );
}

#[test]
fn sessions_ansi_drops_dcs_and_other_strings() {
    assert_eq!(sanitize("a\x1bP+q544e\x1b\\b"), "ab");
    assert_eq!(
        sanitize("a\x1b_apc\x1b\\b\x1b^pm\x1b\\c\x1bXsos\x1b\\d"),
        "abcd"
    );
}

#[test]
fn sessions_ansi_unterminated_string_drops_to_end() {
    assert_eq!(sanitize("a\x1b]52;c;AAAA"), "a");
    assert_eq!(sanitize("a\x1bPqqq"), "a");
}

#[test]
fn sessions_ansi_drops_c0_and_c1_controls() {
    // BEL, backspace, NUL, DEL.
    assert_eq!(sanitize("a\x07b\x08c\0d\x7fe"), "abcde");
    // 8-bit CSI, OSC, DCS and ST as Unicode C1 chars: the control goes, so
    // nothing downstream can read the rest as a sequence.
    assert_eq!(
        sanitize("a\u{9b}2Jb\u{9d}0;t\u{9c}c\u{90}x\u{9c}d"),
        "a2Jb0;tcxd"
    );
}

#[test]
fn sessions_ansi_drops_lone_and_two_char_escapes() {
    // RIS (full reset), keypad mode, charset select, trailing ESC.
    assert_eq!(sanitize("a\x1bcb\x1b=c\x1b(0d\x1b"), "abcd");
}

#[test]
fn sessions_ansi_drops_overlong_csi() {
    let long = format!("a\x1b[{}mb", "1;".repeat(100));
    assert_eq!(sanitize(&long), "ab");
}

#[test]
fn sessions_ansi_keeps_carriage_return_progress() {
    assert_eq!(sanitize("10%\r50%"), "10%\r50%");
}
