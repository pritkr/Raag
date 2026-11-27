# U6: Automated Regression Gates, Test Selector Sync & Live Browser Verification

> **Wave:** 3 (Final Stitch & Verification — Runs ONLY after Wave 1 `U1` and all Wave 2 subagents `U2`–`U5` complete)
> **Findings Verified:** ALL 23 findings (`F-01` through `F-23`)
> **Exclusive Owned Paths:**
> - `tests/interface_remediation_gate.rs` (**NEW**)
> - `tests/phase4_hardening_and_qa.rs`
> - `tests/headless_ui.rs`
> - `tests/phase5_polish_and_ergonomics.rs`
> - `tests/w2c_auth_local_media_test.rs` (only if `on_dialog` sync needed for Remove Folder / Sign Out)
> - `tests/w2d_listen_together_rooms_test.rs` (only if `"Connect"` button removal requires switching selector to `"Create Room"` / `"Join Room"`)
> - `src/previewState.test.ts` (**NEW** or appended to existing TS tests)
> - `CONTRIBUTING.md` (optional short note documenting new automated contrast/OKLCH/a11y gates)

---

## 1. Exact Test Selector Syncs Required After Wave 2

Because Wave 2 upgraded destructive actions to require native `alert_dialog()` confirmation and harmonized casing/punctuation, update these exact lines in existing integration tests:

### 1.1 `tests/headless_ui.rs`
- **Line 99 (`AUTOPLAY RADIO`):** If `U3` updated `queue.rs:31` to `"Autoplay Radio"`, update:
  ```rust
  app.wait_for("Autoplay Radio")?;
  ```
- **Lines 112–115 (`Clear Cache` confirmation dialog `F-05`):**
  Insert `app.on_dialog(...)` immediately before `app.press("Clear Cache")?` so the test verifies the new confirmation dialog path (remember `nsh_testing::Headless` defaults to pressing Cancel when `on_dialog` is unset!):
  ```rust
  app.wait_for("Clear Cache")?;
  // First verify Cancel is a safe no-op
  app.on_dialog(|d| {
      assert!(d.title.contains("Clear offline cache"));
      nsh_testing::DialogAnswer::Press("Cancel".to_owned())
  });
  app.press("Clear Cache")?;
  // Then confirm Clear Cache executes
  app.on_dialog(|_| nsh_testing::DialogAnswer::Press("Clear Cache".to_owned()));
  app.press("Clear Cache")?;
  app.wait_for("Audio chunks: 0.0 MB (0 chunks) • Artwork: 0.0 MB • Metadata: 0 rows")?;
  ```
- *(Note: `tests/headless_ui.rs:58` `app.press("← Back")?` and `:77,97` `app.press("Open Lyrics")?` / `app.press("Up Next Queue")?` remain unchanged and GREEN!)*

### 1.2 `tests/phase5_polish_and_ergonomics.rs`
- **Line 80 (`-5s` -> `−5s`):** Update `app.press("-5s")?;` to `app.press("\u{2212}5s")?;` to match `U3`'s typographic minus update in `lyrics.rs`.

### 1.3 `tests/w2c_auth_local_media_test.rs` & `tests/w2d_listen_together_rooms_test.rs`
- Inspect both files before running `cargo test`:
  - In `w2c_auth_local_media_test.rs`, if any test presses `id:local-remove-root-btn` or `id:desktop-signout-btn`, register `app.on_dialog(|_| DialogAnswer::Press("Remove Folder".to_owned()))` / `DialogAnswer::Press("Sign Out".to_owned())`.
  - In `w2d_listen_together_rooms_test.rs`, if any test presses `"Connect"`, update it to press `"Create Room"` (when join code is empty) or `"Join Room"` (when join code is populated), and if any test presses `"Kick"` or `"Make Host"`, add `on_dialog` confirmation assertions.

---

## 2. Expand `tests/phase4_hardening_and_qa.rs` Contrast & OKLCH Gate (`F-06`, `F-07`, `F-14`)

Extend `wcag_aa_contrast_passes_for_both_light_and_dark_themes` in `tests/phase4_hardening_and_qa.rs` (lines 56–89) to permanently enforce:
1. All **66** contrast pairs across Light and Dark themes (adding `primary` on `background`/`card`/`muted`, `destructive-foreground` on `destructive`, `destructive` on `background`/`card`/`muted`, `status-ok` on `card`/`muted`, `sponsor-badge` on `card`/`muted`, `lyric-inactive` on `background`/`ambient-warm`, `on-cover` on all 6 `cover_gradient_pair` light & dark stops, and `input` non-text contrast `>= 3.0:1` on `background`/`card`).
2. **OKLCH hue separation math (`F-07`):** Compute OKLCH hue angle `H` in Rust (using standard Ottosson sRGB -> linear -> LMS -> OKLab -> `atan2(b, a)` formula) and assert in BOTH `light` and `dark`:
   - `hue_diff(primary, destructive) >= 15.0`
   - `hue_diff(primary, sponsor-badge) >= 15.0`
   - `hue_diff(primary, status-ok) >= 15.0`
   - `hue_diff(destructive, sponsor-badge) >= 15.0`
   - `hue_diff(destructive, status-ok) >= 15.0`
   - `hue_diff(sponsor-badge, status-ok) >= 15.0`
3. **Zero unused `chart-*` tokens (`F-14`):** Assert `vars.get("chart-1").is_none()` through `"chart-5"` in both Light and Dark themes.
4. **Update `srgb_to_linear` threshold** from legacy `0.03928` (`:38`) to exact IEC 61966-2-1 / WCAG 2.1 `0.04045`.

---

## 3. Create `tests/interface_remediation_gate.rs` (NEW Comprehensive Regression Suite)

Write a new integration test file covering every single finding (`F-01`–`F-23`) with both static source-invariant checks and live `nsh_testing::Headless` end-to-end checks:

### 3.1 Static Architecture & Accessibility Gates
1. **`zero_raw_colors_and_no_unused_chart_tokens` (`F-06`, `F-14`):**
   - Scan `src/screens/*.rs` and `src/ui/controls/*.rs` -> zero `rgb(`, `rgba(`, `0xFF`.
   - Confirm `src/ui/controls/primitives.rs` exports `focusable_button`, `confirm_destructive`, `destructive_button`, `labeled_input`, `active_state_dot`.
2. **`all_inputs_suppress_global_shortcuts_and_use_input_border` (`F-06`, `F-16`):**
   - Scan `src/screens/*.rs` and `src/ui/controls/*.rs`: verify zero raw `input()` calls exist outside `src/ui/controls/primitives.rs` (every screen input goes through `labeled_input()`).
3. **`no_hidden_horizontal_scrollbars_or_truncated_errors` (`F-09`, `F-10`):**
   - Scan `src/screens/*.rs`: assert zero `.horizontal().scrollbar(false)` calls.
   - Scan `src/screens/shell.rs`: assert `status_toast` does NOT apply `.number_of_lines(1)` to error messages.
4. **`no_completed_download_using_primary_or_destructive_color` (`F-07`):**
   - Verify `DownloadStatus::Completed` in `src/ui/controls/media_rows.rs`, `src/screens/album.rs`, and `src/screens/now_playing.rs` uses `color_status_ok`, NEVER `theme().primary`.
5. **`no_us_uk_spelling_mismatch` (`F-20`):**
   - Assert `"\"Cancelled\""` does not appear in `src/screens/` or `src/ui/`.
6. **`web_bridge_tokens_and_a11y_hygiene` (`F-01`, `F-02`, `F-12`, `F-14`, `F-19`):**
   - Read `src/App.tsx` and assert:
     - `!src.contains("outline: 'none'")`
     - `!src.contains("Toggle Color Scheme")`
     - `!src.contains("#d92638")`
     - `!src.contains("text-[10px]") && !src.contains("text-[11px]")`
     - `src.contains("<h1")`
     - `src.contains("paddingInlineStart")`
     - `src.contains("overscrollBehavior")` or `index.css` contains `overscroll-behavior: contain`.

### 3.2 Live `nsh_testing::Headless` Functional & Reflow Tests
7. **`destructive_confirmations_cancel_is_noop_and_confirm_executes` (`F-05`):**
   - Test **Clear Up Next** in `Queue`: press `"Clear Up Next"` with `Cancel` -> queue count unchanged; press again with `"Clear Up Next"` -> up-next cleared.
   - Test **Delete Playlist** in `Library`: create playlist `"Temp Mix"`, press `"label:Delete playlist Temp Mix"` with `Cancel` -> still present; press again with `"Delete Playlist"` -> removed.
   - Test **Clear Cache** in `Settings`: Cancel leaves stats non-zero; Confirm zeroes cache readout.
8. **`escape_key_dismisses_all_modal_sheets` (`F-03`):**
   - Open `auth_sign_in_sheet` (`id:open-auth-sheet-btn`) -> assert visible -> `app.key("Escape")?` -> assert closed!
   - Open `cast_picker_sheet` -> assert visible -> `app.key("Escape")?` -> assert closed!
   - Open `sleep_timer_sheet` -> `app.key("Escape")?` -> assert closed!
   - Open `speed_sheet` -> `app.key("Escape")?` -> assert closed!
   - Open `create_playlist_sheet` -> `app.key("Escape")?` -> assert closed!
   - Open `add_to_playlist_sheet` -> `app.key("Escape")?` -> assert closed!
9. **`stateful_accessible_names_and_label_in_name` (`F-02`, `F-04`):**
   - On Now Playing: toggle Shuffle -> accessible label transitions `"Shuffle: Off"` <-> `"Shuffle: On"`.
   - Toggle Repeat -> `"Repeat: Off"` -> `"Repeat: All"` -> `"Repeat: One"`.
   - Toggle Dislike -> `"Dislike track"` <-> `"Remove dislike"`.
   - On Settings DSP (when active): assert `"label:Decrease 31Hz band"` and `"label:Increase 16kHz band"` exist.
10. **`reflow_320px_390px_and_desktop_light_dark_smoke` (`F-08`, `F-15`):**
    - Start `Headless` at `Size::new(Points(320.0), Points(640.0))` in BOTH `ColorScheme::Light` and `ColorScheme::Dark`.
    - Navigate across Home, Explore, Search, Library, Now Playing, Lyrics, Queue, Settings, and open `auth_sign_in_sheet`.
    - Assert zero panics, zero missing primary CTAs, and zero duplicated track durations on Album detail screen (`F-15`).
11. **`filtered_empty_states_show_clear_cta` (`F-11`, `F-20`):**
    - In `local_files` with seeded tracks, type `"zzzz-no-match"` into `id:local-files-filter-input` -> assert it shows `"No local tracks match"` and `"Clear filter"` (NEVER `"No local tracks indexed yet"`!) -> press `"Clear filter"` -> tracks return!

---

## 4. TypeScript Unit Tests (`src/previewState.test.ts` or `node --test`)

Run with `bun test` or `node --test`:
- Test `formatHostError(503, '{"error":"Pure Rust NSH Preview Engine is starting up","detail":"fetch failed"}')` returns clean copy without `{`, `}`, or `"fetch failed"`.
- Test `formatHostError(500, 'internal')` returns actionable recovery copy with `"Retry Connection"`.
- Test `previewReducer` monotonic sequence fencing remains green.

---

## 5. Mandatory End-to-End Verification Runbook (Execute ALL Steps in Order!)

Run every command below and confirm **0 failures** before concluding Wave 3:

```bash
# 1. Pure Rust workspace typecheck & clippy
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings

# 2. Full Rust unit, contract, headless UI, contrast, OKLCH & remediation test suites
cargo test --workspace

# 3. Smoke binaries
cargo run --bin resonance -- --self-check
cargo run --bin resonance -- --fixture

# 4. Build nsh_preview binary so live web host serves real Pure Rust UI (not 503!)
cargo build --bin nsh_preview

# 5. TypeScript lint & production web bundle check
bun run lint
bun run build
```

### Live Chromium Verification via `playwright-cli` (MANDATORY!)

After `cargo build --bin nsh_preview` succeeds and `http://localhost:3000` serves the live Rust tree (`curl -s http://localhost:3000/api/health` returns `{"ok":true,"nshReady":true}`):

1. Open `http://localhost:3000` in headless Chromium via `playwright-cli`.
2. **Capture & inspect screenshots + accessibility snapshots across 4 viewport/zoom matrices in BOTH Dark and Light themes:**
   - Desktop `1440×900` (Dark + Light)
   - Mobile `390×844` (Dark + Light)
   - Compact `320×700` (click `320px` preset, verify zero horizontal overflow `document.documentElement.scrollWidth <= window.innerWidth`)
   - `200%` text/page zoom at `1280×800` (verify zero clipped controls or overlapping text)
3. **Keyboard Tab & Escape audit in Chromium:**
   - Press `Tab` across the host header & NSH preview surface -> confirm visible Saffron-Copper focus ring on every focused interactive element.
   - Confirm `<h1>` exists in accessibility snapshot (`heading [level=1]`).
   - Confirm zero console errors and zero failed network requests (`>= 400`).
4. Clean up `.playwright-cli/` temporary artifacts when finished!

---

## 6. Final Verdict Criteria (`Block` -> `Approve`)

Wave 3 may declare **`Approve`** ONLY when:
- All 11 `HIGH` findings (`F-01`..`F-11`) and all 12 `MEDIUM`/`LOW` findings (`F-12`..`F-23`) pass their automated gates.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `bun run lint`, and `bun run build` all pass with **0 errors and 0 warnings**.
- Live Chromium visual inspection at `1440×900`, `390×844`, `320×700`, and `200%` zoom confirms clean Light & Dark rendering of the real Pure Rust NSH preview engine (`nshReady: true`).
