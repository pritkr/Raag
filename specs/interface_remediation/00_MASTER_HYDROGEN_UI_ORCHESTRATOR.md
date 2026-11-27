# Master Hydrogen / Antigravity Interface Remediation Orchestrator (`specs/interface_remediation`)

> **Project:** Resonance (`Raag`) — Pure Rust YouTube Music Client (`GPL-3.0-or-later`, NSH `=0.7.0`) + React 19 / Tailwind CSS v4 Web Preview Bridge
> **Source Audit:** Cross-discipline interface review across **Accessibility**, **Layout**, **Writing**, **Typography**, **Colors**, and **UI** (23 consolidated root-cause findings: **11 `HIGH` escalation triggers** + **12 `MEDIUM`/`LOW` defects**).
> **Target Verdict Transition:** `Block` -> `Approve` (zero `HIGH` findings, 100% WCAG 2.1 AA contrast & OKLCH hue compliance, 320px / 390px / 1440px / 200% zoom reflow verified, full keyboard & screen-reader coverage).
> **Mode:** Pure planning / specification package for Hydrogen & Antigravity subagents. **Do not skip Wave 1 (`U1`)** — Waves 2A–2D depend on the shared tokens and primitives established in `U1`.

---

## 1. Execution Architecture & Parallel Wave DAG

To allow **Hydrogen / Antigravity subagents** to execute in parallel with **zero merge conflicts**, remediation is organized into **3 waves**:

```
┌────────────────────────────────────────────────────────────────────────────┐
│  WAVE 1 (Sequential Foundation — 1 Subagent)                               │
│  U1: Theme Tokens, Calibrated Palette, Shared A11y/UI Primitives & Copy    │
│      Owns: theme.json, src/ui/tokens.rs, src/ui/icons.rs,                  │
│            src/ui/controls/primitives.rs, src/ui/controls/mod.rs,          │
│            src/ui/text_format.rs (new), src/ui/mod.rs                      │
└─────────────────────────────────────┬──────────────────────────────────────┘
                                      │
                           [Compile Gate 1: cargo check]
                                      │
┌─────────────────────────────────────▼──────────────────────────────────────┐
│  WAVE 2 (4 Parallel Hydrogen Subagents — 100% Disjoint File Ownership)     │
├────────────────────┬────────────────────┬──────────────────┬───────────────┤
│ U2: Shell, Nav,    │ U3: Playback Core  │ U4: Browse, Lib, │ U5: Web Host  │
│ Transport, Sheets  │ Screens            │ Rooms, Local,    │ & nsh_preview │
│ & Shared Controls  │                    │ Settings Screens │ Serializer    │
│                    │                    │                  │               │
│ • shell.rs         │ • now_playing.rs   │ • home.rs        │ • src/App.tsx │
│ • auth_sign_in_    │ • lyrics.rs        │ • explore.rs     │ • src/index.  │
│   sheet.rs         │ • queue.rs         │ • search.rs      │   css         │
│ • navigation.rs    │ • album.rs         │ • library.rs     │ • index.html  │
│ • transport.rs     │ • artist.rs        │ • listen_        │ • server.ts   │
│ • sheets.rs        │                    │   together.rs    │ • src/bin/    │
│ • context_menu.rs  │                    │ • local_files.rs │   nsh_preview │
│ • cast_sheet.rs    │                    │ • settings.rs    │   .rs         │
│ • sleep_timer_     │                    │ • settings_dsp_  │               │
│   sheet.rs         │                    │   card.rs        │               │
│ • speed_sheet.rs   │                    │ • settings_      │               │
│ • media_rows.rs    │                    │   updater_card.rs│               │
│ • cards.rs         │                    │                  │               │
│ • vault_widgets.rs │                    │                  │               │
└────────────────────┴────────────────────┴──────────────────┴───────────────┘
                                      │
                           [Compile Gate 2: cargo check + bun run lint]
                                      │
┌─────────────────────────────────────▼──────────────────────────────────────┐
│  WAVE 3 (Final Stitch, Test Sync & Visual Verification — 1 Subagent)       │
│  U6: Automated Regression Gates, Test Selector Sync & Live Browser Audit   │
│      Owns: tests/interface_remediation_gate.rs (new),                      │
│            tests/phase4_hardening_and_qa.rs, tests/headless_ui.rs,         │
│            tests/phase5_polish_and_ergonomics.rs, CONTRIBUTING.md          │
└────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. Hard Repository Invariants (Enforced by Automated Gates)

Every subagent MUST verify its diff against these non-negotiable rules before finishing:

1. **NSH Version Pin (`Cargo.toml`):** Keep `nsh-core`, `nsh-base`, `nsh-engine`, `nsh-testing` pinned at `=0.7.0`. **Never invent an NSH API** — use only verified `nsh_core` / `nsh_base` APIs documented in `.agents/skills/nsh/references/rust-ui.md` and listed in each workstream spec.
2. **Zero Raw Color Literals in UI (`tests/phase4_hardening_and_qa.rs:111`, `tests/phase5_polish_and_ergonomics.rs:128`, `tests/w1a_ui_craft_test.rs:237`):**
   - **CRITICAL:** These tests run literal substring checks `!src.contains("rgb(") && !src.contains("rgba(") && !src.contains("0xFF")` across EVERY `.rs` file in `src/screens/` and `src/ui/controls/` — **including comments and docstrings!** Never write `rgb(`, `rgba(`, or `0xFF` even in a comment inside `src/screens/` or `src/ui/controls/`.
3. **Touch Targets (`AGENTS.md`, `tests/phase4_hardening_and_qa.rs:94`):** Keep `MOBILE_MIN_TOUCH_TARGET = 48.0`. Every mobile interactive control's **actual button hit area** (not merely an outer non-interactive wrapper `view()`) must be `>= 48dp` via `.min_h(px(MOBILE_MIN_TOUCH_TARGET))` / `.min_w(px(MOBILE_MIN_TOUCH_TARGET))` or `.hit_slop(EdgeInsets::all(px(...)))`.
4. **Zero Blur in Virtual Lists:** Never add `.backdrop_blur(...)` inside any `virtual_list` cell closure.
5. **UI Thread Non-Blocking:** Never perform blocking filesystem/SQLite/network calls in `src/store/*` or `src/screens/*`. Confirmations use `spawn(async move { alert_dialog()...show().await })` (never blocking).
6. **Zero App Duplication in TypeScript:** `src/App.tsx` remains a pure host/preview renderer for `nsh_preview` — never re-implement app screens or playback state in TSX.
7. **JS Tooling:** Use `bun` / `bunx` (never `npm` / `npx` / `yarn` / `pnpm`).
8. **Licensing:** Keep `GPL-3.0-or-later` compliance; add zero proprietary dependencies.

---

## 3. Existing Test Selector Compatibility Contract (CRITICAL!)

Before modifying any button text, accessibility label, or confirmation flow, subagents MUST honor these existing assertions in `tests/` (discovered via full-repo selector audit):

| Existing Test & Line | Selector / Assertion | Required Handling Strategy |
| :--- | :--- | :--- |
| `tests/headless_ui.rs:58` | `app.press("← Back")?` | In `shell.rs:221`, **delete** mismatching `.accessibility_label("Go Back")` and keep visible text `"← Back"` (or set label `"← Back"`). Satisfies WCAG 2.5.3 Label in Name AND keeps test green without changing test! |
| `tests/headless_ui.rs:77,97` & `tests/phase5_polish_and_ergonomics.rs:63` | `app.press("Open Lyrics")?` & `app.press("Up Next Queue")?` | In `now_playing.rs:1509-1522`, **keep** the single canonical `"Open Lyrics"` + `"Up Next Queue"` action pair on Mobile Now Playing and **delete the redundant inner `"Up Next Queue"` button** inside `NowPlayingTab::UpNext`. Both tests stay green! |
| `tests/headless_ui.rs:99` | `app.wait_for("AUTOPLAY RADIO")?` | Keep `"AUTOPLAY RADIO"` text in `queue.rs:31` OR update `tests/headless_ui.rs:99` in `U6` if changing to `"Autoplay Radio"`. Spec chooses keeping `"Autoplay Radio"` consistently across all screens and updating `tests/headless_ui.rs:99` cleanly in `U6` (documented in both `U3` & `U6`). |
| `tests/headless_ui.rs:112-114` | `app.press("Clear Cache")?` -> `app.wait_for("Audio chunks: 0.0 MB...")?` | Wrapping `"Clear Cache"` in `alert_dialog()` requires adding `app.on_dialog(\|_\| DialogAnswer::Press("Clear Cache".to_owned()));` right before line 113 in `tests/headless_ui.rs` (handled in `U6`!). Note: `nsh_testing::Headless` defaults to pressing Cancel when `on_dialog` is unset (`rust-testing.md:151`). |
| `tests/phase4_hardening_and_qa.rs:57-89` | 9 mandatory `theme.json` WCAG contrast pairs (`>= 7.0` for fg/bg, card-fg/card, lyric-active/bg; `>= 4.5` for primary, liked, secondary, accent, muted-fg, control) | The calibrated `theme.json` in `U1` has been mathematically proven to pass ALL 9 existing assertions + 57 additional contrast & OKLCH assertions with **0 failures**. |
| `tests/phase4_hardening_and_qa.rs:154,179-181` | `app.wait_for("4 tracks in queue • 15:26 total")?`, `app.press("label:Move Down >> nth=0")?`, `app.press("label:Move Up >> nth=1")?`, `app.press("label:Remove from Queue >> nth=0")?` | In `queue.rs`, keep exact pluralized format `"{n} track(s) in queue • {total} total"` (so `4` renders `"4 tracks in queue • 15:26 total"`) and move the 3 `48dp` row action buttons to a responsive second row below the track metadata on narrow viewports (`< 420px`) instead of hiding them in a menu! All 3 `label:` selectors stay directly pressable! |
| `tests/phase5_polish_and_ergonomics.rs:79-80` | `app.press("+5s")?` and `app.press("-5s")?` | Keep `"-5s"` or update `tests/phase5_polish_and_ergonomics.rs:80` to `"−5s"` atomically in `U6` (recommended: update both in `U3` + `U6` for typographic minus consistency, with explicit test sync). |
| `tests/phase5_polish_and_ergonomics.rs:144` | `desktop.wait_for("Shortcuts: Space · ←/→ · N/P · L · M")?` | Removing `.number_of_lines(1)` in `shell.rs:738` preserves the exact string while allowing clean wrapping inside the 236px sidebar. |
| `tests/w1a_ui_craft_test.rs:41` | `media_rows.contains("if store::is_track_liked(&track_id_color)")` | Preserve that exact line in `src/ui/controls/media_rows.rs` when adding active state cues. |
| `tests/w1a_ui_craft_test.rs:249-263` | `library.rs` must not contain `settings_sections` and must contain `nav-settings-shortcut`; `now_playing.rs` must not contain `Opus 251 • Proxy` | Preserve verbatim in `U4` and `U3`. |

---

## 4. Complete 23-Finding Traceability Matrix

Every finding from the cross-discipline audit (all 11 `HIGH` escalation triggers + all 12 `MEDIUM`/`LOW` findings, including all 8 previously cap-excluded findings) is assigned to an explicit workstream and verification gate:

| ID | Sev | Domain | Root Cause Summary | Workstream(s) | Verification Gate |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **F-01** | HIGH | Accessibility | Missing visible focus indicators (`App.tsx` `outline: 'none'` + NSH buttons missing `.focus_visible`) | `U1`, `U2`, `U3`, `U4`, `U5` | `U6` (`focus_visible` AST/grep gate + Playwright Tab audit) |
| **F-02** | HIGH | Accessibility | Missing accessible names (Trash, 20 EQ `-/+`, 4 emoji reactions) & WCAG 2.5.3 Label in Name mismatches (`App.tsx` Light/Dark, `now_playing`, `library`, `shell`) | `U2`, `U3`, `U4`, `U5` | `U6` (Headless `label:` assertions + Playwright a11y tree check) |
| **F-03** | HIGH | Accessibility | Pointer-only clickable `<div>` (`App.tsx:287`), missing `Escape` on `auth_sign_in_sheet`/`cast_picker_sheet`, hover-only `12px` seekbar thumb | `U2`, `U5` | `U6` (Headless `Escape` tests for all 6 modals + Playwright keyboard check) |
| **F-04** | HIGH | Accessibility | Active/toggle state conveyed by color alone (Dislike, Shuffle, Repeat All, Lyrics/Queue toggles, Timer/Speed, mobile header nav, vault storage bar) | `U1`, `U2`, `U3` | `U6` (Headless stateful `label:` assertions + visual dot/border check) |
| **F-05** | HIGH | Accessibility | Unconfirmed destructive actions with neutral styling (Delete Playlist, Clear Cache, Clear Up Next, Delete Download, Kick, Make Host, Remove Local Folder) | `U1`, `U2`, `U3`, `U4` | `U6` (`on_dialog` cancel=no-op & confirm=executes tests) |
| **F-06** | HIGH | Colors | Failing WCAG 2.1 contrast pairs across `theme.json`, `listen_together` warning (`2.77:1`), dark `primary` text (`3.11–4.06:1`), opacity-dimmed lyrics (`2.06–2.64:1`), light `status-ok` on `muted` (`4.10:1`), light cover gradients (`1.98–3.61:1`), `App.tsx` hover (`4.43:1`), `input` borders (`1.40–1.52:1`) | `U1`, `U2`, `U3`, `U4`, `U5` | `U6` (66-pair WCAG 2.1 contrast math gate in `phase4` + `interface_remediation_gate.rs`) |
| **F-07** | HIGH | Colors | OKLCH hue collision (`< 4°`) between brand `primary` and `destructive` + crimson `primary` used on `DownloadStatus::Completed` | `U1`, `U2`, `U3` | `U6` (OKLCH `>= 15°` hue-separation math gate + `color_status_ok` grep gate) |
| **F-08** | HIGH | Layout | Fixed widths (`auth_sign_in_sheet` 480px, updater bar 240px, mobile artwork 324px, `queue.rs` row button crush) & non-scrolling modal overlays clipping at 320px / 390px / 200% zoom | `U2`, `U3`, `U4` | `U6` (Headless 320×640, 390×844, 1280×800 tests + Playwright 320px / 200% zoom) |
| **F-09** | HIGH | Layout | Single-line `.number_of_lines(1)` truncating errors (`status_toast`, download failures), shortcut legend, and titles with no full view | `U2`, `U3`, `U4` | `U6` (Source gate banning `number_of_lines(1)` on toasts/errors) |
| **F-10** | HIGH | Layout | Horizontal scroll strips hiding off-screen chips/cards via `.scrollbar(false)` with zero overflow cue | `U3`, `U4` | `U6` (Source gate banning bare `.horizontal().scrollbar(false)` without pager controls) |
| **F-11** | HIGH | Writing | Raw JSON error in `App.tsx:557`, raw `{err}` without recovery steps in Cast/Downloads/Updater, misleading filtered empty state in `local_files.rs:336` | `U1`, `U2`, `U4`, `U5` | `U6` (Unit tests on error formatters + Headless filtered-empty test + Playwright check) |
| **F-12** | MEDIUM | Typography | Fixed `px` fontSize & `px`/`1.25` line-height in `App.tsx`, `14px` `<input>` iOS zoom, sub-12px `text-[11px]`/`text-[10px]` | `U5` | `U6` (Playwright computed font-size & line-height check) |
| **F-13** | MEDIUM | Writing | Inconsistent tab/action casing (`UP NEXT` vs `Up Next`, `Go`, `Resume` vs `Retry`), redundant `"Connect"` button in `listen_together.rs`, naive `"1 songs"`/`"1 tracks"` pluralization | `U1`, `U2`, `U3`, `U4` | `U6` (Unit tests on `pluralize()` + Headless copy assertions) |
| **F-14** | MEDIUM | Colors | 10 unused `chart-1`..`chart-5` tokens in `theme.json`; raw hex/rgba in `App.tsx` & `index.css`; hardcoded RGB palettes in `tokens.rs` | `U1`, `U5` | `U6` (Token usage gate confirming zero unused `chart-*` keys & CSS vars in `index.css`) |
| **F-15** | MEDIUM | UI | Non-concentric nested radii (`now_playing`, tabs, `App.tsx`), duplicated track duration (`album.rs:298`), duplicated mobile `"Up Next Queue"` (`now_playing.rs:1509`), bare `transition` & missing `overscrollBehavior: 'contain'` (`App.tsx`) | `U1`, `U2`, `U3`, `U5` | `U6` (Headless `album` duration count check + Playwright computed styles) |
| **F-16** | MEDIUM | Accessibility | Placeholder-only `input()` controls without programmatic labels & missing `set_text_input_focused` on 6 inputs (`search`, `settings`, `auth_sign_in_sheet`, `listen_together`, `local_files`, `sheets`) | `U1`, `U2`, `U4` | `U6` (Source gate verifying all `input()` go through `labeled_input()` + shortcut suppression test) |
| **F-17** | MEDIUM | Accessibility | Mobile touch targets `< 48dp` (`cards.rs:85` 28×24, `library.rs:1030` 28×28, `vault_widgets.rs:188` 36px, `hero_pill_button`/`glass_pill_button` 40px) | `U1`, `U2`, `U4` | `U6` (Source & Headless target size verification) |
| **F-18** | MEDIUM | Accessibility | Dynamic `status_toast` without persistent status role (`shell.rs:447`), missing `<h1>` (`App.tsx:388`), decorative icons missing `aria-hidden="true"` (`App.tsx`) | `U2`, `U5` | `U6` (Playwright accessibility snapshot verifying `heading[level=1]` & clean icon labels) |
| **F-19** | MEDIUM | Layout | Physical directional properties (`left`, `paddingLeft`/`Right`, `textAlign: 'left'`) in `App.tsx` instead of logical properties | `U5` | `U6` (Grep/Playwright RTL check on `App.tsx`) |
| **F-20** | MEDIUM | Writing | Empty states missing next-step CTA (`home`, `search`, `queue`, `lyrics`, `library`) & mixed UK/US spelling (`"Cancelled"` vs `"Canceled"`) | `U2`, `U3`, `U4` | `U6` (Headless empty-state CTA verification + spelling grep) |
| **F-21** | MEDIUM | Typography | Keyboard punctuation stand-ins (`...` -> `…`, `-` -> `−`, straight `"..."` -> `“…”`) & baked-in uppercase copy | `U1`, `U2`, `U3`, `U4`, `U5` | `U6` (Unit tests + grep audit) |
| **F-22** | MEDIUM | Typography | Uncapped prose measure (`artist.rs:284`, `settings.rs:130`) & missing `tabular_nums()` on dynamic numbers (`App.tsx:490`, `cast_sheet.rs:209`, `search.rs:167,272`, `local_files.rs:285`) | `U2`, `U3`, `U4`, `U5` | `U6` (Source check for `.max_w(px(660.0))` & `.tabular_nums()`) |
| **F-23** | LOW | UI | Unreferenced `motion::duration_ms` in UI, inconsistent press opacities (`0.70–0.85`), unscaled `stroke-width="2"` on `12–14px` SVGs in `icons.rs` | `U1`, `U5` | `U6` (Consistent press feedback & icon stroke verification) |

---

## 5. Copy-Pasteable Hydrogen / Antigravity Subagent Dispatch Prompts

### Wave 1 — Foundation (Run First, Solo)
- **Subagent U1:**
  > *"Read `AGENTS.md`, `specs/interface_remediation/00_MASTER_HYDROGEN_UI_ORCHESTRATOR.md`, and `specs/interface_remediation/U1_THEME_TOKENS_AND_SHARED_PRIMITIVES.md`. Implement U1 strictly within its Exclusive Owned Paths (`theme.json`, `src/ui/tokens.rs`, `src/ui/icons.rs`, `src/ui/text_format.rs`, `src/ui/mod.rs`, `src/ui/controls/primitives.rs`, `src/ui/controls/mod.rs`). Verify with `cargo check --workspace` and `cargo test --test phase4_hardening_and_qa wcag_aa_contrast`."*

### Wave 2 — Parallel Execution (Launch All 4 Simultaneously After Wave 1 Passes `cargo check`)
- **Subagent U2:**
  > *"Read `AGENTS.md`, `specs/interface_remediation/00_MASTER_HYDROGEN_UI_ORCHESTRATOR.md`, and `specs/interface_remediation/U2_SHELL_NAVIGATION_TRANSPORT_AND_OVERLAYS.md`. Implement U2 strictly within its Exclusive Owned Paths. Consume the U1 helpers (`focusable_button`, `confirm_destructive`, `labeled_input`, `active_state_dot`, `color_on_cover`, `color_destructive_foreground`, `text_format`). Zero raw color literals (even in comments). Verify with `cargo check --workspace`."*
- **Subagent U3:**
  > *"Read `AGENTS.md`, `specs/interface_remediation/00_MASTER_HYDROGEN_UI_ORCHESTRATOR.md`, and `specs/interface_remediation/U3_NOW_PLAYING_LYRICS_QUEUE_ALBUM_ARTIST.md`. Implement U3 strictly within its Exclusive Owned Paths (`now_playing.rs`, `lyrics.rs`, `queue.rs`, `album.rs`, `artist.rs`). Consume U1 helpers and preserve the Test Selector Compatibility Contract. Zero raw color literals (even in comments). Verify with `cargo check --workspace`."*
- **Subagent U4:**
  > *"Read `AGENTS.md`, `specs/interface_remediation/00_MASTER_HYDROGEN_UI_ORCHESTRATOR.md`, and `specs/interface_remediation/U4_HOME_EXPLORE_SEARCH_LIBRARY_ROOMS_LOCAL_SETTINGS.md`. Implement U4 strictly within its Exclusive Owned Paths (`home.rs`, `explore.rs`, `search.rs`, `library.rs`, `listen_together.rs`, `local_files.rs`, `settings.rs`, `settings_dsp_card.rs`, `settings_updater_card.rs`). Consume U1 helpers. Zero raw color literals (even in comments). Verify with `cargo check --workspace`."*
- **Subagent U5:**
  > *"Read `AGENTS.md`, `specs/interface_remediation/00_MASTER_HYDROGEN_UI_ORCHESTRATOR.md`, and `specs/interface_remediation/U5_WEB_PREVIEW_BRIDGE_AND_SERIALIZER.md`. Implement U5 strictly within its Exclusive Owned Paths (`src/App.tsx`, `src/index.css`, `index.html`, `server.ts`, `src/bin/nsh_preview.rs`). Verify with `bun run lint` and `cargo check --bin nsh_preview`."*

### Wave 3 — Final Stitch, Test Sync & Visual Verification (Run After All 4 Wave-2 Subagents Complete)
- **Subagent U6:**
  > *"Read `AGENTS.md`, `specs/interface_remediation/00_MASTER_HYDROGEN_UI_ORCHESTRATOR.md`, and `specs/interface_remediation/U6_VERIFICATION_GATES_AND_TEST_UPDATES.md`. Implement `tests/interface_remediation_gate.rs`, update `tests/phase4_hardening_and_qa.rs`, `tests/headless_ui.rs`, `tests/phase5_polish_and_ergonomics.rs`, and `CONTRIBUTING.md`. Run `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `bun run lint`, and live Chromium verification via `playwright-cli` across 1440×900, 390×844, 320×700, and 200% zoom in Light & Dark modes."*
