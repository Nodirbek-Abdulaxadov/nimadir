//! Guest text, made safe to show where the guest does not own the screen.
//!
//! A guest's log line reaches a terminal, the shell's log file and the
//! chrome's questions. In a terminal, an escape sequence is a command — it can
//! retitle the window, move the cursor, overwrite what was printed; in a log, a
//! newline is a forged entry; and anywhere, a bidirectional-override character
//! can make `moc.live` read as `evil.com` — or an address read as a different
//! one in the question that asks whether to open it.

/// `s` with every control character and every bidirectional formatting
/// character written out as an escape (`\u{202e}`), so what is shown is what
/// is there.
pub fn visible(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() || is_bidi_control(c) {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    out
}

/// The characters that reorder text around them: embeddings, overrides,
/// isolates and marks (Unicode's bidirectional formatting characters).
fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
    )
}
