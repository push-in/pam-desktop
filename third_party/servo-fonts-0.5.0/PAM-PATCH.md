# PAM patch provenance

This directory contains the source published as `servo-fonts 0.5.0` from
Servo revision `77fccacc1f1fdce10498d50173aafaa09d02879e` under the Mozilla
Public License 2.0. Test fixtures are intentionally omitted.

PAM carries one source correction in `platform/macos/font.rs`: the optional
Core Foundation language string is retained as an owned value until after
`CTFontCreateForStringWithLanguage` consumes its borrowed reference. The
published expression borrowed a temporary and fails with Rust error `E0716`
on macOS arm64.

Remove this patch when a crates.io Servo release contains the equivalent
lifetime correction and the native compatibility matrix passes without the
local override.
