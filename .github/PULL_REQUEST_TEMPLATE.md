## Summary
Describe what this PR changes and why.

## Checklist
- [ ] `cargo check --workspace` passes
- [ ] `cargo test` passes
- [ ] `cargo run --bin resonance -- --self-check` passes
- [ ] No raw color literals (`rgb(`, `rgba(`, `0xFF`) added in `src/screens/` or `src/ui/controls.rs`
- [ ] No blocking I/O on the NSH engine thread (`src/store.rs`, `platform/nsh_audio.rs`)
- [ ] Secrets, cookies, and signed URLs are redacted in logs and `ANDROID_VR`/`IOS` remain cookie-isolated
- [ ] Licensed under `GPL-3.0-or-later`
