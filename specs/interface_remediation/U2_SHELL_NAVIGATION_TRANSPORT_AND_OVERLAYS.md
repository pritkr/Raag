# U2: Shell, Navigation, Transport, Overlays & Shared Media Controls

> **Wave:** 2 (Parallel after Wave 1 `U1` passes `cargo check`)
> **Findings Resolved:** `F-01`, `F-02`, `F-03`, `F-04`, `F-05`, `F-06`, `F-07`, `F-08`, `F-09`, `F-11`, `F-13`, `F-15`, `F-16`, `F-17`, `F-18`, `F-20`, `F-21`, `F-22`
> **Exclusive Owned Paths:**
> - `src/screens/shell.rs`
> - `src/screens/auth_sign_in_sheet.rs`
> - `src/ui/controls/navigation.rs`
> - `src/ui/controls/transport.rs`
> - `src/ui/controls/sheets.rs`
> - `src/ui/controls/context_menu.rs`
> - `src/ui/controls/cast_sheet.rs`
> - `src/ui/controls/sleep_timer_sheet.rs`
> - `src/ui/controls/speed_sheet.rs`
> - `src/ui/controls/media_rows.rs`
> - `src/ui/controls/cards.rs`
> - `src/ui/controls/vault_widgets.rs`
>
> **HARD RULE:** Zero `rgb(`, `rgba(`, or `0xFF` anywhere in these files (not even in comments!).
> **12:41 Sync Baseline:** Preserve `src/ui/controls/ambient.rs` (`ambient_shader_backdrop`, `live_equalizer_indicator`, `animated_play_pause_icon`, `animated_like_heart`), `back_button()`, `keyboard_shortcuts_popover()` (`id:shortcuts-help-btn`), and existing `.focus_visible` / reduced-motion transitions added at 12:41. Focus on all remaining open defects: `Escape` modal cascade, `nav_pill_button` focus, `color_on_cover`, `color_status_ok`, `confirm_destructive`, visible seekbar thumb, active state dots/labels, scrollable max-width sheets, `labeled_input`, vault cache contrast & `48dp` quota pills, and recoverable errors.

---

## 1. File-by-File Exact Changes

### 1.1 `src/ui/controls/navigation.rs`
- **Fix `F-03` (Missing `Escape` dismissal for 2 modal overlays):**
  At lines `164-179` inside `dispatch_global_shortcut` (`"escape"` branch), extend the modal dismissal cascade to cover ALL 6 modal overlays in strict top-down precedence:
  ```rust
  "escape" => {
      if store::add_to_playlist_track().is_some() {
          store::close_add_to_playlist_sheet();
          true
      } else if store::active_track_sheet().is_some() {
          store::close_track_sheet();
          true
      } else if store::is_sleep_timer_sheet_open() {
          store::close_sleep_timer_sheet();
          true
      } else if store::is_speed_sheet_open() {
          store::close_speed_sheet();
          true
      } else if store::auth_sheet_open() {
          store::close_auth_sheet();
          true
      } else if store::cast_sheet_open() {
          store::set_cast_sheet_open(false);
          true
      } else {
          false
      }
  }
  ```
- **Fix `F-01` (Focus indicators on nav controls):**
  Audit `nav_pill_button` and `mobile_bottom_nav_bar` buttons; replace raw `button()` with `focusable_button()` from `crate::ui::controls::primitives`.

---

### 1.2 `src/screens/shell.rs`
1. **Fix `F-06` (Cover monogram color):**
   At line `99` (mini-artwork fallback monogram), replace `.text_color(move || theme().primary_foreground)` with `.text_color(color_on_cover)`.
2. **Fix `F-02` (WCAG 2.5.3 Label in Name on Back button — preserving `tests/headless_ui.rs:58`):**
   At lines `221-235`, the Back button renders visible text `"← Back"` while setting `.accessibility_label("Go Back")`.
   - **Action:** Replace `button()` with `focusable_button()` and **DELETE** `.accessibility_label("Go Back")` (so the accessible name is derived directly from `"← Back"`, matching WCAG 2.5.3 AND keeping `tests/headless_ui.rs:58` `app.press("← Back")?` 100% green!).
3. **Fix `F-09` & `F-18` (Status toast truncation & live status semantics):**
   At lines `447-487` (`status_toast`):
   - **DELETE** `.number_of_lines(1)` on line `465` so long error/status messages wrap cleanly across lines instead of clipping permanently.
   - Add `.role(AccessibilityRole::Status)` (verify exact `AccessibilityRole` variant in `nsh_core`; if `Status` is unavailable in `0.7.0`, use `AccessibilityRole::Alert` or keep a persistent outer container mounted with `.accessibility_label("Status notification")`).
   - Replace dismiss `button()` with `focusable_button()`.
4. **Fix `F-09` (Sidebar keyboard shortcut legend truncation):**
   At line `738` (`"Shortcuts: Space · ←/→ · N/P · L · M"` inside the 236px sidebar):
   - **DELETE** `.number_of_lines(1)` so the shortcut guide wraps cleanly inside 236px instead of truncating. Keep the exact string `"Shortcuts: Space · ←/→ · N/P · L · M"` so `tests/phase5_polish_and_ergonomics.rs:144` passes unchanged!
5. **Fix `F-01` (Missing focus rings across shell buttons):**
   Replace raw `button()` with `focusable_button()` at lines `221, 257, 780, 799, 1092, 1170, 1265, 1350, 1369, 1388`.
6. **Fix `F-15` (Concentric tab inner radius):**
   At lines `994-1010` (Desktop side-panel `tabs()`): outer container has `.rounded(radius_card)` (`12px`) and `.p_1()` (`4px`). Set inner `tab()` and indicator radius to `radius_tab_inner` (`8px` = `12 - 4`) instead of `theme().radius` (`10px`).
7. **Fix `F-04` (Mobile header active tab signaled by color alone):**
   At lines `1350-1404` (Mobile top header `Rooms`, `Queue`, `Settings` icon buttons):
   - Pair the active `theme().primary` icon color with BOTH:
     - Active background tint (`if active { pill_glass_hover() } else { pill_glass_surface() }`) + active border (`if active { theme().primary } else { glass_hairline() }`)
     - `active_state_dot(move || store::active_tab() == target_tab)` placed below the icon in a `flex_col().items_center().gap_0p5()`
     - Stateful `.accessibility_label(move || if active() { "Queue (current)" } else { "Queue" })` (while keeping `.test_id("nav-queue")` / `"nav-settings"` / `"nav-listen-together"` intact for all headless tests!).
8. **Verify `shell.rs:860` top search input:**
   Already calls `set_text_input_focused(true/false)` — ensure it also has `.accessibility_label("Search songs, albums, artists, podcasts")` and `.focus(|s| s.border_color(move || theme().ring))`.

---

### 1.3 `src/screens/auth_sign_in_sheet.rs`
1. **Fix `F-08` (Fixed `480px` width overflows `390px` mobile & `320px` narrow viewports + non-scrolling modal):**
   - At line `85`, change `.w(px(480.0))` to `.w_full().max_w(px(480.0))`.
   - Wrap the inner modal content inside `scroll().max_h(percentage(0.85))` so all steps & buttons remain reachable at 200% zoom or short viewports.
2. **Fix `F-16` & `F-01` (Cookie paste input missing label & shortcut suppression):**
   - At lines `299-304`, replace raw `input()` with `labeled_input(...)` (adds visible label `"Browser Cookie Header"`, `.focus(...)` ring, and `set_text_input_focused(true/false)` so typing letters in the cookie string never fires playback shortcuts!).
   - Replace all raw `button()` controls in the sheet with `focusable_button()`.
3. **Fix `F-17` (40px pill buttons inside 48px wrapper):**
   - Because `U1` upgrades `hero_pill_button` and `glass_pill_button` to `48dp` on mobile, remove redundant `view().min_h(px(MOBILE_MIN_TOUCH_TARGET))` wrappers at lines `92-110` (cheaper fix: **Delete**).
4. **Fix `F-11` (Recovery guidance on OAuth errors):**
   - At lines `189-268`, when `tv_oauth_status` represents a failure/timeout, format through `text_format::format_recoverable_error("Sign-in didn't complete", &err, "Request a new TV code or use Browser Cookie Sign-In below.")` and render a `"Try Again"` button.

---

### 1.4 `src/ui/controls/transport.rs`
1. **Fix `F-01` (Focus indicators on all 12 player bar buttons):**
   Replace raw `button()` with `focusable_button()` at lines `148, 251, 279, 319, 346, 379, 430, 452, 476, 548, 622, 706`.
2. **Fix `F-03` & `F-17` (Hover-only `12px` seekbar thumb):**
   At lines `21-55` (`seek_slider`) and `101-129` (desktop top-edge seek slider):
   - Increase interactive track hit height to `h(px(24.0))` on desktop and `h(px(48.0))` on mobile (keeping the visual bar 4px centered vertically inside the hit container!).
   - Remove `opacity-0` hiding on the thumb; render a persistent `14px` visible thumb (growing to `16px` on hover/focus) with `.focus_visible(...)` ring so touch and keyboard users can locate and scrub playback position.
3. **Fix `F-04` (Color-only active states on Dislike, Shuffle, Repeat All, Lyrics & Queue toggles):**
   - At lines `288-294` (Dislike), `328-334` (Shuffle), `388-403` (`RepeatMode::All`), and `501-546` (Desktop Lyrics & Queue side panel toggles):
     - Add `active_state_dot(is_active)` inside each button AND subtle active pill highlight (`bg(move || if is_active() { pill_glass_surface() } else { transparent })`).
     - Update `.accessibility_label(...)` dynamically to include state while keeping existing test-compatible prefixes where needed (note: verify `tests/` — none of `Player Bar Dislike`, `Player Bar Shuffle`, `Player Bar Repeat Mode` are pressed by label in `tests/`, so `"Shuffle: On"` / `"Shuffle: Off"`, `"Repeat: All"` / `"Repeat: One"` / `"Repeat: Off"`, `"Dislike: Active"` / `"Dislike"`, `"Lyrics Panel: Open"` / `"Lyrics Panel"` are safe and much clearer!).
4. **Fix `F-06` (Cover monogram color):**
   At lines `210` and `680`, replace `.text_color(move || theme().primary_foreground)` with `.text_color(color_on_cover)`.

---

### 1.5 `src/ui/controls/sheets.rs`, `sleep_timer_sheet.rs`, `speed_sheet.rs`, `cast_sheet.rs`
1. **Fix `F-08` (Non-scrolling modal overlays clip actions at 200% zoom or short viewports):**
   - In `modal_panel_container` (`sheets.rs`) and directly in `track_action_sheet` (`sheets.rs:105-240`), `add_to_playlist_sheet` (`sheets.rs:293-410`), `sleep_timer_sheet` (`sleep_timer_sheet.rs:131-237`), `speed_sheet` (`speed_sheet.rs:104-252`), and `cast_picker_sheet` (`cast_sheet.rs:46-227`):
     - Ensure panel width is `.w_full().max_w(px(420.0))` (never fixed width overflowing 320px).
     - Wrap the sheet content body inside `scroll().max_h(percentage(0.85))` so tall lists of 11+ track actions or presets always scroll cleanly without clipping top headers or the `Close` button.
2. **Fix `F-01` (Focus indicators):**
   Replace every raw `button()` with `focusable_button()` in `sheets.rs` (`:52,263,346`), `sleep_timer_sheet.rs` (`:85,168,217`), `speed_sheet.rs` (`:68,141,164,196,234`), and `cast_sheet.rs`.
3. **Fix `F-16` (Unlabeled playlist name input in `add_to_playlist_sheet`):**
   At `sheets.rs:321-335`, replace raw `input()` with `labeled_input(...)` (`label: "New playlist name"`, automatic `set_text_input_focused`).
4. **Fix `F-05` (Unconfirmed delete download in `sheets.rs:170`):**
   Route `DownloadStatus::Completed => confirm_destructive("Remove offline download?", format!("Remove “{}” from offline storage?", track.title), "Remove download", move || store::delete_download(&vid_dl))`.
5. **Fix `F-11`, `F-20`, `F-21`, `F-22` (Copy, recovery, spelling, typography):**
   - `cast_sheet.rs:62`: Replace `format!("Cast error: {err}")` with `text_format::format_recoverable_error("Couldn't connect to Cast device", &err, "Ensure both devices share the same Wi-Fi network and try scanning again.")` + add `"Scan Again"` button in the error state.
   - `cast_sheet.rs:209,217-218`: Add `.tabular_nums()` on `Receiver Volume: {vol_pct}%`; change `"Vol -"` / `"Vol +"` to `"Volume −"` / `"Volume +"` (with `.accessibility_label("Decrease Cast Volume")` / `"Increase Cast Volume"`).
   - `sleep_timer_sheet.rs:53`: Standardize UK `"Sleep timer cancelled"` -> US `"Sleep timer canceled"` (`F-20`).
   - `sheets.rs:154`: Replace ASCII `"Add to Playlist..."` -> `"Add to Playlist…"` (`F-21`).

---

### 1.6 `src/ui/controls/context_menu.rs`, `media_rows.rs`, `cards.rs`, `vault_widgets.rs`
1. **Fix `F-07` & `F-05` (`DownloadStatus::Completed` color & unconfirmed deletion):**
   - `media_rows.rs:63,349`: Replace `theme().primary` with `color_status_ok()` for `DownloadStatus::Completed` (`F-07` — green check/icon, never red!).
   - `media_rows.rs:48` & `context_menu.rs:190`: Guard `store::delete_download(&vid)` behind `confirm_destructive("Remove offline download?", ..., "Remove download", ...)` (`F-05`).
   - `media_rows.rs:27`: Make `.accessibility_label` reflect state (`"Download track"`, `"Download queued"`, `"Pause download"`, `"Resume download"`, `"Retry download"`, `"Remove offline download"`).
   - `context_menu.rs:92-98`: Standardize `"Retry Download"` for `Failed | Cancelled` (matching `sheets.rs:98`), `"Resume Download"` for `Paused`, `"Remove Download"` for `Completed`. Replace `"Add to Playlist..."` (`:160`) with `"Add to Playlist…"`.
   - **CRITICAL PRESERVATION:** Keep `if store::is_track_liked(&track_id_color)` verbatim in `media_rows.rs` (`tests/w1a_ui_craft_test.rs:41`).
2. **Fix `F-01` (Focus indicators):**
   Replace raw `button()` with `focusable_button()` in `context_menu.rs:105`, `media_rows.rs:25,132,213,281,388`, `cards.rs:32,80`, `vault_widgets.rs:35,186,232,250`.
3. **Fix `F-17` (Undersized touch targets):**
   - `cards.rs:85-86` (`Play Next` button is `28×24px`): on mobile (`!is_desktop_layout()`), give it `min_w(px(MOBILE_MIN_TOUCH_TARGET)).min_h(px(MOBILE_MIN_TOUCH_TARGET))` or `.hit_slop(EdgeInsets::all(px(12.0)))` so hit target reaches `>= 48dp`!
   - `vault_widgets.rs:188` (Quota preset pills): change `.min_h(px(36.0))` to `.min_h(px(if is_desktop_layout() { 36.0 } else { MOBILE_MIN_TOUCH_TARGET }))`.
4. **Fix `F-04` & `F-06` (`vault_widgets.rs:162-180` invisible dark cache bar):**
   - In dark mode, `theme().accent` (`#2c201b`) on `theme().muted` (`#282523`) has **`1.04:1` contrast** (completely invisible).
   - Change the cache bar segment background from `theme().accent` to `theme().muted_foreground` (`6.04:1` contrast on `muted` in dark, `5.65:1` in light!) and add small inline legend indicators next to `"X downloads • Y cache"` (matching swatch dots + labels) so segment meaning never relies on color alone.
5. **Fix `F-09` (Title readability):**
   - Raise primary title in `cards.rs:67` to `.number_of_lines(2)` so long album/mix titles remain legible without breaking card grid alignment.

---

## 2. U2 Verification Checklist
1. `cargo check --workspace` passes with zero errors.
2. Zero occurrences of `rgb(`, `rgba(`, or `0xFF` across all 12 owned files.
3. Verify `dispatch_global_shortcut("escape", false)` closes `auth_sign_in_sheet` and `cast_picker_sheet`.
4. Confirm `shell.rs:221` no longer sets mismatching `.accessibility_label("Go Back")` and `shell.rs:465,738` no longer have `.number_of_lines(1)`.
5. Confirm `media_rows.rs` still contains `if store::is_track_liked(&track_id_color)`.
