# U3: Playback & Detail Screens (`Now Playing`, `Lyrics`, `Queue`, `Album`, `Artist`)

> **Wave:** 2 (Parallel after Wave 1 `U1` passes `cargo check`)
> **Findings Resolved:** `F-01`, `F-02`, `F-04`, `F-05`, `F-06`, `F-07`, `F-08`, `F-09`, `F-10`, `F-13`, `F-15`, `F-20`, `F-21`, `F-22`
> **Exclusive Owned Paths:**
> - `src/screens/now_playing.rs`
> - `src/screens/lyrics.rs`
> - `src/screens/queue.rs`
> - `src/screens/album.rs`
> - `src/screens/artist.rs`
>
> **HARD RULE:** Zero `rgb(`, `rgba(`, or `0xFF` anywhere in these 5 files (not even in comments!). Preserve `"Opus 251 • Proxy"` absence in `now_playing.rs` (`tests/w1a_ui_craft_test.rs:261`).

---

## 1. File-by-File Exact Changes

### 1.1 `src/screens/now_playing.rs`
1. **Fix `F-08` & `F-15` (320px overflow & non-concentric artwork radius):**
   - At line `1503` (mobile `focal_artwork(268.0)`): `268 + 20(p_2p5) + 32(p_4) + 4(borders) = 324px > 320px`!
   - Compute `let art_size = if is_narrow_viewport() { 236.0 } else { 268.0 };` (`236 + 20 + 32 + 4 = 292px <= 320px`).
   - At lines `244-253` (`focal_artwork`): outer container uses `.rounded(radius_panel)` (`18px`) + `.p_2p5()` (`10px`) + `.border_1()` (`1px`). Change inner artwork `.rounded(radius_card)` (`12px`) to **`.rounded(radius_panel_inner)`** (`7px` = `18 - 10 - 1`, geometrically concentric!).
   - At line `311` (fallback monogram inside `focal_artwork`): replace `.text_color(move || theme().primary_foreground)` with **`.text_color(color_on_cover)`** (`F-06`).
2. **Fix `F-02`, `F-05`, `F-07` (Download action button Label-in-Name, danger hue collision, unconfirmed delete):**
   - At lines `398-492`:
     - Replace static `.accessibility_label("Download Current Track")` with a dynamic label matching the visible label text (`"Download"`, `"Queued"`, `"Downloading {pct}%"`, `"Paused — Resume"`, `"Offline — Remove"`, `"Retry Download"`), satisfying WCAG 2.5.3 Label in Name!
     - At line `453`, replace `Some(DownloadStatus::Completed) => theme().primary` with **`Some(DownloadStatus::Completed) => color_status_ok()`** (`F-07`).
     - At lines `431-433`, wrap `store::delete_download(&rec.video_id)` inside `confirm_destructive("Remove offline download?", format!("Remove “{}” from offline storage?", track.title), "Remove download", move || store::delete_download(&vid))` (`F-05`).
3. **Fix `F-04` (Color-only active states on Header Timer/Speed, Dislike, Shuffle, Repeat All):**
   - At lines `189-218` (Sleep Timer & Playback Speed header icon buttons): pair active `color_player_accent()` with `active_state_dot(is_active)` + subtle active pill border `theme().primary` + stateful `.accessibility_label(...)` (`"Sleep Timer: 15:00"` / `"Sleep Timer"`, `"Playback Speed: 1.25×"` / `"Playback Speed"`).
   - At lines `542-548` (Dislike): add `active_state_dot(is_disliked)` + active pill highlight + `.accessibility_label(move || if is_disliked() { "Dislike: Active" } else { "Dislike" })`.
   - At lines `658-664` (Shuffle): add `active_state_dot(is_shuffled)` + active pill highlight + `.accessibility_label(move || if is_shuffled() { "Toggle Shuffle: On" } else { "Toggle Shuffle: Off" })`.
   - At lines `722-745` (`RepeatMode::All`): reuse the exact indicator dot already drawn for `RepeatMode::One` on `RepeatMode::All` as well (e.g., show dot for `RepeatMode::All | RepeatMode::One`), plus dynamic `.accessibility_label(move || match mode() { RepeatMode::Off => "Cycle Repeat Mode: Off", RepeatMode::All => "Cycle Repeat Mode: All", RepeatMode::One => "Cycle Repeat Mode: One" })`.
4. **Fix `F-13` & `F-15` (Tab casing consistency & concentric tab radius):**
   - At lines `897-965`: outer `tabs()` uses `.rounded(radius_card)` (`12px`) + `.p_1()` (`4px`). Change inner `tab()` radius from `theme().radius` (`10px`) to **`radius_tab_inner`** (`8px`).
   - Standardize tab copy from `"UP NEXT"` / `"LYRICS"` / `"RELATED"` to **`"Up Next"` / `"Lyrics"` / `"Related"`** (matching `shell.rs:1012-1062`!).
5. **Fix `F-06` (Inactive lyrics contrast in Now Playing preview):**
   - At line `1126`, **DELETE** `.opacity(0.62)` / `.opacity(0.72)` on inactive lyric lines and rely solely on calibrated `color_lyric_inactive()` (`5.61:1` light / `5.77:1` dark on `ambient-warm`).
6. **Fix `F-15` (Duplicated `"Up Next Queue"` button on Mobile — preserving `tests/headless_ui.rs:77,97`!):**
   - At lines `1509-1522`, Mobile Now Playing renders `mobile_now_playing_tabs_card()` (which already contains an `"Up Next Queue"` button inside the `UpNext` tab) AND a second `"Open Lyrics"` + `"Up Next Queue"` button row immediately beneath it.
   - **CRITICAL TEST COMPATIBILITY:** `tests/headless_ui.rs:77` presses `"Open Lyrics"`, `tests/headless_ui.rs:97` presses `"Up Next Queue"`, and `tests/phase5_polish_and_ergonomics.rs:63` presses `"Open Lyrics"`.
   - **Action:** **DELETE** the redundant `"Up Next Queue"` button inside the `NowPlayingTab::UpNext` tab body and **KEEP** the single canonical `"Open Lyrics"` + `"Up Next Queue"` action bar at lines `1509-1522`! Both buttons remain immediately visible regardless of which tab is selected, eliminating duplication while keeping every headless test 100% green!
7. **Fix `F-21` (Typographic minus on `-15s`):**
   - Check `tests/headless_ui.rs:73`: tests only press `"+15s"` (not `"-15s"`), so updating `"-15s"` at line `617` to `"−15s"` (matching `"−"` at `:849`) is completely safe. Keep `.accessibility_label("Rewind 15 seconds")`.

---

### 1.2 `src/screens/lyrics.rs`
1. **Fix `F-06` (Opacity-crushed inactive lyrics fail even 3:1 large text):**
   - At lines `161-165`: **DELETE** `.opacity(0.55)` / `.opacity(0.62)` / `.opacity(0.72)` on past and upcoming lyric lines. Use `color_lyric_inactive()` at full `1.0` opacity (`6.27:1` light / `6.80:1` dark on background; `5.61:1` / `5.77:1` on `ambient-warm`), differentiating active vs. inactive via `FontWeight::BOLD` + larger `text_xl` vs `text_base` + `color_lyric_active()` vs `color_lyric_inactive()`.
2. **Fix `F-01` (Missing focus rings):**
   - Replace raw `button()` with `focusable_button()` at lines `150` (each clickable lyric line) and `309` (`"Sync to current line"` floating pill).
3. **Fix `F-20` (Empty state missing next action):**
   - At lines `288-293`, pass an actionable CTA to `status_card`: `Some("Open Search")`, `Some(Box::new(|| store::set_tab(AppTab::Search)))` (or `"Retry Lyrics"` when a track is loaded).
4. **Fix `F-21` (`-5s` typographic minus — synced with `U6`):**
   - At line `473`, update `"-5s"` -> `"−5s"` (`U+2212`) and note in `U6` to update `tests/phase5_polish_and_ergonomics.rs:80` from `app.press("-5s")?` to `app.press("−5s")?`. (Alternatively, keep `accessibility_label("Seek back 5 seconds")`.)

---

### 1.3 `src/screens/queue.rs`
1. **Fix `F-08` (320px header squeeze & row title crush — preserving `tests/phase4_hardening_and_qa.rs:179-181`!):**
   - At lines `221-262` (Queue header & Shuffle/Repeat bar): add `.flex_wrap()` to both `.flex_row()` containers so `"Save Queue"` / `"Clear Up Next"` and `"Shuffle"` / `"Repeat"` wrap gracefully at `320px`.
   - At lines `60-194` (Queue virtual list row): currently places `44px` artwork + flex title/artist + THREE `48px` buttons (`Move Up`, `Move Down`, `Remove from Queue` = `~236px` fixed) on one horizontal row, crushing titles to `~60px` at `320px`!
   - **Solution (100% test-compatible):** When `is_narrow_viewport()` (`width < 380.0`), switch the row container to `.flex_col().gap_2()`:
     - Row 1: `44px` artwork + full-width title & artist + duration (zero crushing!)
     - Row 2: Right-aligned `.flex_row().justify_end().gap_2()` holding the exact 3 `48dp` `focusable_button()` controls (`Move Up`, `Move Down`, `Remove from Queue`) with identical `.accessibility_label(...)` values!
     - At `>= 380px` (including `390×844` mobile & desktop), keep the compact single-row layout with tighter `.gap_1()` and ensure `title` has `.min_w(px(0.0)).flex_1()`.
     - **Result:** Zero clipping at 320px AND `tests/phase4_hardening_and_qa.rs:179-181` (`label:Move Down`, `label:Move Up`, `label:Remove from Queue`) passes unchanged!
2. **Fix `F-05` (Unconfirmed `"Clear Up Next"` with neutral styling):**
   - At line `260`, replace `secondary_button("Clear Up Next", store::clear_up_next)` with:
     ```rust
     destructive_button("Clear Up Next", || {
         confirm_destructive(
             "Clear Up Next queue?",
             "All upcoming tracks will be removed. The currently playing track will continue.",
             "Clear Up Next",
             store::clear_up_next,
         );
     })
     ```
   - *(Verified: `tests/` never presses `"Clear Up Next"`, so adding confirmation breaks zero existing tests!)*
3. **Fix `F-06` (Cover monogram color):**
   - At line `499`, replace `.text_color(move || theme().primary_foreground)` with **`.text_color(color_on_cover)`**.
4. **Fix `F-13` & `F-20` (Pluralization, heading casing, empty state CTA):**
   - At lines `47` and `242`, replace `format!("{count} tracks...")` with `text_format::pluralize(count, "track", "tracks")` so `1` renders `"1 track in queue • ..."` and `4` still renders `"4 tracks in queue • 15:26 total"` (preserving `tests/phase4_hardening_and_qa.rs:154`!).
   - At line `31`, standardize `"AUTOPLAY RADIO"` -> `"Autoplay Radio"` (matching `now_playing.rs:979,1355`; `U6` updates `tests/headless_ui.rs:99`).
   - At lines `197-202`, pass an actionable CTA to empty queue `status_card`: `Some("Explore Music")`, `Some(Box::new(|| store::set_tab(AppTab::Explore)))`.

---

### 1.4 `src/screens/album.rs` & `src/screens/artist.rs`
1. **Fix `F-15` (Duplicated track duration in `album.rs`):**
   - At lines `280-303` of `album.rs`, `track_row_view` (`media_rows.rs:372`) ALREADY renders the track duration! **DELETE** the duplicate duration `text(...)` block at lines `297-302`.
2. **Fix `F-10` (Hidden horizontal scroll shelves with no cue):**
   - At `album.rs:317` and `artist.rs:79`: **DELETE** `.scrollbar(false)` so the platform scroll indicator is visible (and optionally add compact `‹` / `›` desktop pager buttons matching `home.rs` / `explore.rs`).
3. **Fix `F-17` (Redundant 48px wrappers around `hero_pill_button`/`glass_pill_button`):**
   - At `album.rs:213-263` and `artist.rs:226-269`: now that `U1` gives `hero_pill_button` / `glass_pill_button` native `48dp` mobile height, delete the redundant `view().min_h(px(MOBILE_MIN_TOUCH_TARGET))` wrappers.
4. **Fix `F-22` (Uncapped desktop biography measure):**
   - At `artist.rs:284` (artist biography card/paragraph): add `.max_w(px(PROSE_MAX_WIDTH))` (`660px`, `~65–72ch`) so prose never stretches edge-to-edge at `1160px`.
5. **Fix `F-13` (Pluralization & consistent action verbs):**
   - At `album.rs:72-73`, use `text_format::pluralize(count, "song", "songs")` instead of `format!("{count} songs")`.
   - Standardize `"Play"` / `"Start Radio"` consistently across `album.rs` and `artist.rs`.
   - Replace any raw `button()` with `focusable_button()`.

---

## 2. U3 Verification Checklist
1. `cargo check --workspace` passes with zero errors.
2. Zero occurrences of `rgb(`, `rgba(`, or `0xFF` across all 5 owned files.
3. Confirm `now_playing.rs` does not contain `Opus 251 • Proxy` and keeps `"Open Lyrics"` + `"Up Next Queue"` once on mobile.
4. Confirm `album.rs` renders duration exactly once per track row.
5. Confirm `lyrics.rs` has zero `.opacity(0.55)` / `.opacity(0.62)` / `.opacity(0.72)` on lyric lines.
