//! Flatpak diagnostic sanitization.

#![cfg(all(test, unix))]

use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;

use crate::services::flatpak_failure::{
    FlatpakCommandFailure, RENDERED_FAILURE_CHAR_CAP, sanitize_diagnostic,
};

fn status_code(code: i32) -> ExitStatus {
    ExitStatus::from_raw(code << 8)
}

#[test]
fn sanitizer_consumes_seven_and_eight_bit_terminal_sequences() {
    let raw = concat!(
        "\u{1b}[31mred\u{1b}[0m ",
        "\u{1b}]0;secret\u{7}visible ",
        "\u{9b}32mgreen\u{9b}0m ",
        "\u{9d}0;secret\u{9c}after\nline\ttab\u{7}"
    );
    let clean = sanitize_diagnostic(raw);
    assert_eq!(clean, "red visible green after line tab");
    assert!(!clean.chars().any(char::is_control));

    let args = vec!["list".to_owned()];
    let raw_bytes = b"\x9b31mred\x9b0m \x9d0;secret\x9cafter";
    let message =
        FlatpakCommandFailure::from_stderr_bytes(&args, status_code(1), raw_bytes).message();
    assert!(message.contains("red after"), "{message:?}");
    assert!(!message.contains("31m") && !message.contains("0;secret"));

    let args = vec!["install".to_owned()];
    let huge = "界".repeat(5000);
    let message =
        FlatpakCommandFailure::from_stderr_bytes(&args, status_code(1), huge.as_bytes()).message();
    assert!(message.chars().count() <= RENDERED_FAILURE_CHAR_CAP);
    assert!(crate::i18n::test_display_text(&message).ends_with('…'));
    assert!(message.contains("install the app"));
}
