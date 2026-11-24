# AGENTS.md — `tests/` (QA & Gate Suites)

Run targeted suites matching changed subsystems; headless UI tests seed deterministic data via `resonance::store::enable_fixture_mode(true)` + `nsh_testing::Headless`.

- `architecture_gates_v2` — NSH `=0.6.1` pin, Gate 0 proofs, EJS bundle + `n`-param cache, `client_matrix.json`, `GPL-3.0-or-later` license, `<3s` cold / `<1.5s` warm / `<100MB` RSS, 50% pre-resolution, format matrix.
- `headless_ui` — Mobile (`390x844` Light/Dark) & Desktop (`1200x800`) end-to-end journeys.
- `nsh_native_audio` — Native `nsh/audio` driver and `NshAudioBackend`.
- `proxy_and_fault_injection` — `LoopbackProxy` 256 KB chunks, `403` re-resolution, upstream faults.
- `phase4_hardening_and_qa` — WCAG AA contrast, `>= 48dp` touch targets, `0ms` reduced motion, zero raw colors in `src/screens/`, Android manifest/receiver checks, virtual list recycling, startup latency.
- `phase5_polish_and_ergonomics` — Mini-player, volume strip, synced lyrics micro-seek, queue summary, desktop shortcuts, zero raw colors in `src/ui/controls.rs`.
- `stage2_and_stage3_verification` — Monotonic proxy `409`, secret redaction, ambient palette WCAG AA, zero blur in virtual lists.
- `core_unit_and_snapshots` — Core cache, solver, lyrics, SponsorBlock, domain unit tests.
