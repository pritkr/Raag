# U4: Browse, Library, Listen Together, Local Files & Settings Screens

> **Wave:** 2 (Parallel after Wave 1 `U1` passes `cargo check`)
> **Findings Resolved:** `F-01`, `F-02`, `F-04`, `F-05`, `F-06`, `F-08`, `F-09`, `F-10`, `F-11`, `F-13`, `F-16`, `F-17`, `F-20`, `F-21`, `F-22`
> **Exclusive Owned Paths:**
> - `src/screens/home.rs`
> - `src/screens/explore.rs`
> - `src/screens/search.rs`
> - `src/screens/library.rs`
> - `src/screens/listen_together.rs`
> - `src/screens/local_files.rs`
> - `src/screens/settings.rs`
> - `src/screens/settings_dsp_card.rs`
> - `src/screens/settings_updater_card.rs`

> **CRITICAL REPO INVARIANTS:**
> - **Zero raw color literals** (`rgb(`, `rgba(`, `0xFF`) anywhere in `src/screens/` — **not even in comments or docstrings!**
> - Preserve `tests/w1a_ui_craft_test.rs:249-255`: `library.rs` MUST NOT contain `settings_sections` and MUST keep `test_id("nav-settings-shortcut")`.
> - Preserve `tests/headless_ui.rs:58` (`"← Back"`), `:102` (`id:nav-search`), `:104` (`id:search-input`), `:112-114` (`"Clear Cache"`), `:128` (`"Quick Picks"`), `:130` (`"SponsorBlock (auto-skip non-music)"`).
> - Preserve `tests/phase5_polish_and_ergonomics.rs:102-105` (`"✓ System Theme"`, `"Dark Theme"`, `"✓ Dark Theme"`, `"label:Offline Mode (local chunk cache only)"`).
> - Use `U1` shared helpers (`focusable_button`, `confirm_destructive`, `destructive_button`, `labeled_input`, `color_destructive_foreground`, `PROSE_MAX_WIDTH`, `text_format::{pluralize, format_recoverable_error, quote_term}`).

---

## 1. `src/screens/home.rs`

### 1.1 Pluralization, Curly Quotes & Eyebrow Casing (`F-13`, `F-21`)
- **Lines 70–77 (`collection_detail_view`):** Replace naive `format!("{song_count} songs • {total_mins} min")` with `pluralize(song_count, "song", "songs")`:
  ```rust
  let songs_txt = crate::ui::text_format::pluralize(song_count, "song", "songs");
  let stats_line = match playlist.subtitle.as_deref() {
      Some(sub) if !sub.is_empty() => format!("{sub} • {songs_txt} • {total_mins} min"),
      _ => format!("{songs_txt} • {total_mins} min"),
  };
  ```
- **Lines 117, 370, 424, 511:** Replace baked-in uppercase strings (`"PLAYLIST"`, `"YOUR RECENT ROTATION"`, `"START RADIO FROM A SONG"`, `"MIXES & ALBUMS"`) with natural Title Case / Sentence Case (`"Playlist"`, `"Your recent rotation"`, `"Start radio from a song"`, `"Mixes & albums"`) and apply `.tracking_wide()` (or letter spacing if supported; otherwise clean `text_xs().font_weight(FontWeight::SEMIBOLD)`).
- **Lines 164–167:** Replace straight quotes in `format!("Radio started from \"{}\"", radio_title)` with `format!("Radio started from {}", quote_term(&radio_title))`.
- **Line 130 (`stats_line`):** Add `.tabular_nums()` (`F-22`) and allow wrapping (`.number_of_lines(2)`) so long playlist subtitles do not clip track counts on `320px`.

### 1.2 Labeled Collection Filter Input & Filtered Empty CTA (`F-16`, `F-20`)
- **Lines 190–209:** Migrate raw `input()` (`test_id("collection-filter-input")`) to `labeled_input(LabeledInputProps { ... })` with accessible label `"Filter tracks in this list"` (compact/sr-only label mode so horizontal toolbar layout stays tight), `border_color(move || theme().input)` (`>= 3.37:1`), and automatic `set_text_input_focused` focus/blur management.
- **Lines 234–250:** When `filtered.is_empty()` and `!query.is_empty()`, include a recovery CTA button `"Clear filter"` calling `store::set_collection_search_query("")` so users are never stranded on a dead-end filtered empty state (`F-20`)! When `query.is_empty()`, include CTA `"Back to Home"` calling `store::close_collection`.

### 1.3 Focus Rings & Horizontal Overflow Cue (`F-01`, `F-10`)
- **Line 561 (mobile collection card `nsh_base::button()`):** Replace `nsh_base::button()` with `focusable_button()` (`F-01`, `F-23`).
- **Lines 631–675 (Mood chips strip):**
  - Keep `scroll().horizontal()` WITH visible scroll affordance (never `.scrollbar(false)`), and wrap the strip in a compact row with Left/Right pager/hint indicator or wrap chips cleanly on narrow screens (`is_narrow_viewport()`) using `.flex_row().flex_wrap().gap_2()` when `window().width.0 < 380.0` so no chips vanish off-screen without cue on `320px` (`F-10`, `F-08`).
  - On desktop/tablet (`>= 380.0`), ensure `scroll().horizontal()` leaves scrollbar enabled (default) AND shows an inline `"Scroll →"` subtle muted hint or previous/next chip-scroll buttons when `chips.len() > 6`.

### 1.4 Recoverable Home Error Copy (`F-11`)
- **Lines 686–696:** Format `store::home_error()` via `format_recoverable_error("Couldn't load YouTube Music home feed", &err, "Check your internet connection and select Retry.")`.

---

## 2. `src/screens/explore.rs`

### 2.1 Focus Rings, Horizontal Strip Discoverability & Narrow Reflow (`F-01`, `F-08`, `F-10`)
- **Lines 194–258 (`new_releases_section`):**
  - The section header ALREADY has Previous/Next pager buttons (`explore_section_header`). Ensure the horizontal strip (`scroll().horizontal()`) NEVER calls `.scrollbar(false)` AND shows a `"Page {page + 1} / {total_pages}"` tabular-nums counter next to the chevron buttons so off-screen items are unmistakably discoverable (`F-10`).
  - On narrow viewports (`< 380px`), scale `media_square_card` width from `168.0` to `144.0` so two items peek cleanly across the right edge as a natural scroll cue (`F-08`, `F-10`).
- **All raw `button()` calls in `explore.rs` (Quick category tiles, Mood & Genre cards around lines 655–690, inline playlist cards):**
  - Replace every `button()` with `focusable_button()` (`F-01`, `F-23`).
  - Verify Mood/Genre cards (`:665–688`) that paint text on `cover_gradient_pair` use `color_on_cover()` instead of `theme().primary_foreground` (`F-06`)! (Remember in Dark mode `theme().primary_foreground` is dark `#0a0a0c`, which would fail contrast on dark mood card gradients!)
- **Error Card (`:703–712`):** Format `store::explore_error()` through `format_recoverable_error("Couldn't load Explore feed", &err, "Check your connection and select Retry.")` (`F-11`).

---

## 3. `src/screens/search.rs`

### 3.1 Input Labeling, Shortcut Suppression & Action Label (`F-13`, `F-16`, `F-21`)
- **Lines 286–347:**
  - Replace raw `input()` (`:302–322`) with `labeled_input(...)` (preserving `.test_id("search-input")`, `.return_key_type(ReturnKeyType::Search)`, `.on_change`, `.on_submit`), adding:
    - `.accessibility_label("Search songs, artists, albums, and playlists")`
    - `.on_focus(|| set_text_input_focused(true))` and `.on_blur(|| set_text_input_focused(false))` (**CRITICAL FIX `F-16`:** previously `search.rs` omitted `set_text_input_focused` completely, so typing `l`, `m`, `n`, `p`, `Space`, `/` inside the Search box triggered global player shortcuts!).
  - Replace `.placeholder("Search songs, artists, albums...")` with typographic ellipsis `"Search songs, artists, albums…"` (`F-21`).
  - Update outer search pill container border (`:297`) from `glass_hairline` to `move || theme().input` (`>= 3.37:1` WCAG 1.4.11 non-text contrast, `F-06`).
  - **Line 344:** Change vague `primary_button("Go", ...)` to specific `primary_button("Search", ...)` (`F-13`). (Check `tests/` first — `grep` confirmed zero tests press `"Go"`.)

### 3.2 Pluralization, Curly Quotes & Tabular Numbers (`F-13`, `F-21`, `F-22`)
- **Line 272:** Replace `format!("{count} results for \"{q}\"")` with:
  ```rust
  SharedString::from(format!(
      "{} for {}",
      crate::ui::text_format::pluralize(count, "result", "results"),
      crate::ui::text_format::quote_term(&q)
  ))
  ```
  And add `.tabular_nums()` on line 276 (`F-22`)!
- **Line 167 (`plays_text`):** Add `.tabular_nums()` (`F-22`).

### 3.3 Focus Rings, Empty State CTA & Recoverable Errors (`F-01`, `F-10`, `F-11`, `F-20`)
- **Lines 97, 183, 215, 326, 375, 474, 540:** Replace every raw `button()` with `focusable_button()`.
- **Line 184 (Like track button in `search_track_row`):** Use dynamic state-reflecting accessible name:
  ```rust
  .accessibility_label(move || {
      if is_liked() {
          SharedString::from("Unlike track")
      } else {
          SharedString::from("Like track")
      }
  })
  ```
- **Lines 401–414 (Search error):** Format `store::search_error()` via `format_recoverable_error("Search couldn't complete", &err, "Check your connection and select Retry.")`.
- **Lines 424–432 (Empty search results):** Distinguish empty query vs. zero matches for an active query (`F-20`):
  - If `store::search_query().trim().is_empty()`: show `"Start searching"` / `"Type a song, artist, album, or playlist above, or explore trending music."` with CTA `"Browse Explore"` -> `store::set_tab(AppTab::Explore)`.
  - If query is non-empty: show `format!("No matches for {}", quote_term(query))` / `"Check your spelling, switch filter to All, or clear your search."` with CTA `"Clear search"` -> clears `query_sig` & calls `store::run_search("")` + `store::set_search_filter(SearchFilter::All)`.
- **Lines 453–462 (Horizontal Albums & Collections strip):** Never suppress scrollbar; add a subtle right-aligned count (`pluralize(albums.len(), "item", "items")`) in the section header so off-screen cards are discoverable (`F-10`).

---

## 4. `src/screens/library.rs`

### 4.1 Missing Accessible Name + Destructive Confirmation on Delete Playlist (`F-01`, `F-02`, `F-05`)
- **Lines 670–685 (`render_playlists_view`):**
  - Previously: bare `IconKind::Trash` button with **zero** `.accessibility_label(...)`, **zero** `.focus_visible(...)`, neutral muted icon color, and **immediate irreversible** `store::delete_playlist(pid_del.clone())`!
  - Replace with:
    ```rust
    let pid_del = pid_delete.clone();
    let pl_title_del = pl_title_for_del.clone();
    row.child(
        focusable_button()
            .test_id(format!("delete-playlist-{}", pid_del))
            .accessibility_label(format!("Delete playlist {pl_title_del}"))
            .min_w(px(MOBILE_MIN_TOUCH_TARGET))
            .min_h(px(MOBILE_MIN_TOUCH_TARGET))
            .items_center()
            .justify_center()
            .rounded(radius_pill)
            .hover(|s| s.bg(pill_glass_hover))
            .on_press(move || {
                let id = pid_del.clone();
                let title = pl_title_del.clone();
                confirm_destructive(
                    format!("Delete “{title}”?"),
                    "This removes the playlist from your library. Songs remain available on YouTube Music.",
                    "Delete Playlist",
                    move || store::delete_playlist(id.clone()),
                );
            })
            .child(icon(IconKind::Trash, 16.0, move || theme().destructive)),
    )
    ```

### 4.2 Destructive Confirmation, Error Recovery & Spelling Consistency in Downloads (`F-05`, `F-09`, `F-11`, `F-13`, `F-20`)
- **Line 204:** Change `"Cancelled"` to US English `"Canceled"` (consistent with `"Cancel"` button labels across the app, `F-20`).
- **Lines 410–425 (Download error subtitle):** Remove `.number_of_lines(1)` on error/failure text so failure reasons wrap up to 3 lines (`F-09`) and format failure details via `format_recoverable_error("Download failed", &err, "Select Retry to resume.")` (`F-11`).
- **Lines 458–485 (`is_resumable` button):** Distinguish paused vs. failed status (`F-13`):
  - When `rec.status == DownloadStatus::Failed`: visible text `"Retry"`, `.accessibility_label("Retry download")`.
  - When `rec.status == DownloadStatus::Paused`: visible text `"Resume"`, `.accessibility_label("Resume download")`.
- **Lines 486–510 (Delete Download button):** Wrap `store::delete_download(&vid_delete)` in `confirm_destructive(...)`, style label in `theme().destructive`, and replace `button()` with `focusable_button()` (`F-01`, `F-05`):
  ```rust
  confirm_destructive(
      format!("Remove “{track_title}” from downloads?"),
      "The offline audio file will be removed from this device. You can download it again later.",
      "Remove Download",
      move || store::delete_download(&vid_delete),
  );
  ```

### 4.3 Search Clear Button Touch Target, Filter Input & Label-in-Name (`F-01`, `F-02`, `F-16`, `F-17`)
- **Lines 560–575 (Create playlist `input()`):** Migrate to `labeled_input(...)` with label `"New playlist name"`, `.on_focus(|| set_text_input_focused(true))`, `.on_blur(|| set_text_input_focused(false))`, and `theme().input` border (`F-06`, `F-16`).
- **Lines 1000–1045 (Library filter bar):**
  - Update outer pill border (`:1003`) from `glass_hairline` to `move || theme().input` (`F-06`).
  - Add `.accessibility_label("Filter current library list")` to `input()` (`:1008`).
  - **Lines 1028–1043 (Clear list filter button):** Increase `.w(px(28.0)).h(px(28.0))` to `.w(px(MOBILE_MIN_TOUCH_TARGET)).h(px(MOBILE_MIN_TOUCH_TARGET))` (or `w(px(36.0)).h(px(36.0)).hit_slop(EdgeInsets::all(px(6.0)))` on desktop and `48×48` on mobile) and switch to `focusable_button()` (`F-01`, `F-17`).
- **Line 1084 (`nav-settings-shortcut`):** Switch to `focusable_button()` (`F-01`). Keep `test_id("nav-settings-shortcut")` untouched!
- **Filter pill `accessibility_label` (`F-02`):** Ensure every `filter_pill` button's accessible name contains its exact visible text (e.g., `"Playlists"`, `"Liked songs"`, `"Downloads"` — never a mismatched synonym).

### 4.4 Horizontal Filter Strip Discoverability (`F-10`)
- **Line 1103:** Currently `scroll().horizontal().scrollbar(false)` hides off-screen filter pills (`Downloads`, `Local Files`, `Pinned`) on mobile with zero indicator!
- **Fix:** Remove `.scrollbar(false)` AND on viewports `< 640px` render a responsive wrapping row (`.flex_row().flex_wrap().gap_2()`) OR provide explicit Left/Right horizontal scroll cue buttons + visible thin scrollbar so users immediately see `Local Files` and `Downloads` without guessing (`F-10`). Wrapping `.flex_row().flex_wrap().gap_2()` is the cleanest, zero-hidden-control solution on all widths!

### 4.5 Pluralization & Empty State CTAs (`F-13`, `F-20`)
- **Lines 153, 280, 897, 1203:** Replace naive `"{count} tracks"` / `"{total} tracks"` with `pluralize(count, "track", "tracks")`. Add `.tabular_nums()` on counts.
- **Lines 160, 356, 585, 753, 879, 1189:** Add helpful next-step CTAs to every empty `status_card`:
  - If library search filter is active (`!query.is_empty()`): CTA `"Clear filter"` -> clears `search_query` and calls `store::set_library_search_query("")`.
  - Otherwise: CTA `"Browse Home"` (`store::set_tab(AppTab::Home)`) or `"Open Search"` (`store::set_tab(AppTab::Search)`).

---

## 5. `src/screens/listen_together.rs`

### 5.1 Critical Contrast Fix on Warning Banner (`F-06`)
- **Lines 253–268 (`local_track_warning` banner):**
  - Previously: `.bg(move || theme().destructive)` with `.text_color(move || theme().primary_foreground)` (`2.77:1` in Dark mode — **CRITICAL FAIL**!).
  - **Fix:** Replace `.text_color(move || theme().primary_foreground)` with `.text_color(color_destructive_foreground)` (**5.98:1** Light / **7.11:1** Dark!).

### 5.2 Emoji Reaction Accessible Names & Focus Rings (`F-01`, `F-02`)
- **Lines 289–305:** Currently `["🔥", "❤️", "🎧", "✨"]` render raw `button()` with NO `.accessibility_label(...)` and NO `.focus_visible(...)`!
- **Fix:** Map `(emoji, spoken_name)` tuples:
  ```rust
  [
      ("🔥", "Send fire reaction"),
      ("❤️", "Send heart reaction"),
      ("🎧", "Send headphones reaction"),
      ("✨", "Send sparkles reaction"),
  ]
  ```
  Use `focusable_button().accessibility_label(spoken_name)` with `.min_w(px(MOBILE_MIN_TOUCH_TARGET)).min_h(px(MOBILE_MIN_TOUCH_TARGET))`.

### 5.3 Destructive Confirmations for Host Actions (`F-05`)
- **Lines 367–379 (`Make Host` & `Kick` buttons):**
  - Wrap `"Make Host"` in `confirm_destructive(...)`:
    - Title: `format!("Transfer host control to {}?", peer_name)`
    - Description: `"You will become a guest and will no longer control room playback or participants."`
    - Confirm label: `"Transfer Host"`
  - Replace `"Kick"` with `destructive_button("Kick", ...)` wrapped in `confirm_destructive(...)`:
    - Title: `format!("Remove {} from room?", peer_name)`
    - Description: `"They will be disconnected from synchronized playback immediately."`
    - Confirm label: `"Remove Participant"`

### 5.4 Labeled Inputs, Shortcut Suppression & Eliminate Redundant "Connect" CTA (`F-01`, `F-13`, `F-16`, `F-21`)
- **Lines 110, 136 (Sub-mode tabs `Radio Mix` / `Listen Together`):** Replace `button()` with `focusable_button()`.
- **Lines 401–458 (3 unstyled, unlabeled `input()` calls):**
  - Migrate all 3 inputs (`Display name`, `Relay URL`, `Room code`) to `labeled_input(...)` with visible `<label>` text (`"Display name"`, `"Relay URL"`, `"Room code"`), `set_text_input_focused` on focus/blur, and `theme().input` border (`>= 3.37:1`).
  - Replace `"Relay URL (https://...)"` placeholder with `"https://relay.raag.audio"` (no `...` stand-in, `F-21`).
- **Lines 416–475 (Redundant `"Connect"` button `F-13`):**
  - Currently the card shows THREE competing buttons: `"Create Room"`, `"Join Room"`, AND a third `"Connect"` button that conditionally creates or joins depending on whether `join_code_draft` is empty!
  - **Fix:** Remove the ambiguous third `"Connect"` button and organize the form cleanly into two unambiguous sections:
    1. **Your identity:** `"Display name"` + `"Relay URL"` inputs -> primary button `"Create Room"`.
    2. **Or join an existing room:** `"Room code"` input -> secondary/glass button `"Join Room"` (disabled or showing inline validation message `"Enter a room code (e.g. RAAG-7K4M) to join."` when `join_code_draft.trim().is_empty()`).
  - Verify `tests/w2d_listen_together_rooms_test.rs` before removing `"Connect"` — if any test presses `"Connect"`, keep `"Join Room"` / `"Create Room"` aligned with `w2d_listen_together_rooms_test.rs` (check in U6!).

### 5.5 Tabular Numbers on Clock Offset & Counts (`F-22`)
- **Lines 232, 313, 334:** Add `.tabular_nums()` to `Synced ±{} ms`, `Up Next ({})`, and `Participants ({})`.

---

## 6. `src/screens/local_files.rs`

### 6.1 Misleading Filtered Empty State (`F-11`, `F-20`)
- **Lines 329–341:**
  - Previously: When `records` is non-empty (user has 200 local tracks) but `filter_query` matches 0 tracks, it falsely claimed `"No local tracks indexed yet. Add a music folder above."`!
  - **Fix:** Check `records.is_empty()` vs. `!q.is_empty()`:
    ```rust
    if all_tracks.is_empty() {
        let is_filtered = !records.is_empty() && !q.is_empty();
        let raw_q = filter_query.get();
        return view()
            .flex_1()
            .p_4()
            .child(if is_filtered {
                status_card(
                    format!("No local tracks match {}", quote_term(raw_q.trim())),
                    "Try a different title, artist, or album, or clear the filter.",
                    Some("Clear filter"),
                    Some(Box::new(move || filter_query.set(String::new()))),
                )
            } else {
                status_card(
                    "No local tracks indexed yet",
                    "Add a music folder above to scan FLAC, MP3, Opus, M4A, or WAV files.",
                    None,
                    None,
                )
            })
            .into_element();
    }
    ```

### 6.2 Destructive Confirmation on Remove Folder (`F-05`, `F-01`)
- **Lines 206–220 (`local-remove-root-btn`):**
  - Switch `button()` to `focusable_button()` and wrap `remove_cb(path_for_btn.clone())` in `confirm_destructive(...)`:
    - Title: `"Remove music folder?"`
    - Description: `format!("Raag will stop indexing tracks from “{}”. Your audio files on disk will not be deleted.", path_for_btn)`
    - Confirm label: `"Remove Folder"`
  - **Test Check:** Verify `tests/w2c_auth_local_media_test.rs` — if a test presses `id:local-remove-root-btn`, `U6` registers `app.on_dialog(|_| DialogAnswer::Press("Remove Folder".to_owned()))`.

### 6.3 Labeled Inputs, Shortcut Suppression, Pluralization & Tabular Numbers (`F-01`, `F-13`, `F-16`, `F-21`, `F-22`)
- **Lines 123–130 (`local-folder-path-input`) & 301–307 (`local-files-filter-input`):**
  - Migrate both to `labeled_input(...)` with `set_text_input_focused` on focus/blur, `theme().input` border, and accessible labels `"Music folder path"` / `"Filter local tracks"`.
  - Replace `"Filter local tracks..."` with `"Filter local tracks…"` (`F-21`).
- **Lines 160, 200, 285, 432, 483:**
  - Use `pluralize(root.track_count, "track", "tracks")`, `pluralize(track_count, "track", "tracks")`, `pluralize(root_count, "folder", "folders")`, `pluralize(count, "track", "tracks")` (`F-13`).
  - Add `.tabular_nums()` on lines 163, 201, 286, 433, 484 (`F-22`).
- **Lines 253, 406, 461 (Sub-tab buttons, Album rows, Artist rows):** Replace `button()` with `focusable_button()` (`F-01`).
- **Lines 102–104 (Picker error):** Use `format_recoverable_error("Couldn't open folder picker", &err.to_string(), "Enter the full folder path below and select Add Path.")` (`F-11`).

---

## 7. `src/screens/settings.rs`

### 7.1 Destructive Confirmation on Clear Cache (`F-05`, `F-06`)
- **Line 400:** Currently `secondary_button("Clear Cache", store::clear_caches)` wipes all cached audio chunks, artwork, and metadata in one tap with zero confirmation and neutral styling!
- **Fix:** Replace with `destructive_button("Clear Cache", ...)` wrapped in `confirm_destructive(...)`:
  ```rust
  destructive_button("Clear Cache", || {
      confirm_destructive(
          "Clear offline cache?",
          "Removes cached audio chunks, artwork, and metadata. Pinned playlists and account settings stay intact.",
          "Clear Cache",
          store::clear_caches,
      );
  })
  ```
  *(Exact visible label `"Clear Cache"` and confirm label `"Clear Cache"` are preserved for `tests/headless_ui.rs:112-114`, synced with `on_dialog` in `U6`!)*

### 7.2 Labeled Cookie Input, Prose Measure & Consistent Focus (`F-01`, `F-16`, `F-21`, `F-22`)
- **Line 134 (`Anonymous mode active...` paragraph):** Add `.max_w(px(PROSE_MAX_WIDTH))` (`660px`, ~65–72ch) (`F-22`).
- **Lines 181–198 (`desktop-cookie-input`):**
  - Migrate to `labeled_input(...)` with visible label `"Browser Cookie header"`, `.masked(true)` (prevents shoulder-surfing of `SAPISID` credentials!), `.on_focus(|| set_text_input_focused(true))`, `.on_blur(|| set_text_input_focused(false))`, and placeholder `"SAPISID=…; __Secure-3PAPISID=…"` (`F-16`, `F-21`).
- **Lines 144, 200, 233:** Replace `button()` with `focusable_button()` (preserving `.test_id(...)` and existing styles).
- **Line 233 (`Sign Out`):** Optionally wrap `store::sign_out()` in `confirm_destructive("Sign out of YouTube Music?", "Local playlists and offline cache remain on this device.", "Sign Out", ...)` (check `tests/w2c_auth_local_media_test.rs` first — if a test clicks `id:desktop-signout-btn` directly, sync `on_dialog` in `U6`).
- **Lines 388–394 (`cache-stats-readout`):** Keep exact string `"Audio chunks: 0.0 MB (0 chunks) • Artwork: 0.0 MB • Metadata: 0 rows"` when 0 so `tests/headless_ui.rs:114` stays green, and use `pluralize` for singular (`1 chunk`, `1 row`).

---

## 8. `src/screens/settings_dsp_card.rs`

### 8.1 Accessible Names, Typographic Minus & Focus Rings on 20 EQ Controls (`F-01`, `F-02`, `F-21`)
- **Lines 234–268 (10 frequency bands × 2 `-/+` buttons):**
  - Previously: 10 buttons labeled `"-"` and 10 buttons labeled `"+"` with **zero** `.accessibility_label(...)` and **zero** `.focus_visible(...)`! A screen-reader user heard `"minus button"` ten times with no band frequency or current dB readout.
  - **Fix:**
    ```rust
    focusable_button()
        .test_id(format!("eq-band-{idx}-down"))
        .accessibility_label(format!("Decrease {lbl} band (currently {gain_txt})"))
        .min_w(px(MOBILE_MIN_TOUCH_TARGET))
        .min_h(px(MOBILE_MIN_TOUCH_TARGET))
        .items_center()
        .justify_center()
        .rounded(move || theme().radius)
        .bg(move || theme().card)
        .on_press(move || {
            store::set_eq_band_gain(idx, gain - 1.5);
        })
        .child(
            text("\u{2212}") // typographic minus (−)
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .text_color(move || theme().foreground),
        )
    ```
    And for `+`:
    ```rust
    focusable_button()
        .test_id(format!("eq-band-{idx}-up"))
        .accessibility_label(format!("Increase {lbl} band (currently {gain_txt})"))
        ...
    ```
- **Line 205 (`gain_txt`):** Format negative values using typographic minus `format!("\u{2212}{:.0}dB", gain.abs())` (`F-21`).
- **Line 137 (gated notice prose):** Add `.max_w(px(PROSE_MAX_WIDTH))` (`F-22`).
- **Line 145 (`audio-dsp-preview-controls`):** Remove `.opacity(0.5)` on text container (which drops `muted-foreground` contrast below `2.5:1`!), and instead rely on semantic `muted-foreground` + `.disabled(true)` badge `"Requires PCM"` (`F-06`).

---

## 9. `src/screens/settings_updater_card.rs`

### 9.1 Fluid Progress Bar Width — Fix `320px` Overflow (`F-08`)
- **Lines 281–293 (`UpdatePhase::Downloading`):**
  - Previously: Hardcoded `.w(px(240.0))` + inner `.w(px(240.0 * fraction))`, which overflows a `320px` viewport inside nested padded containers (`p_4` + `p_4` + `p_3` = `88px` horizontal padding -> only `232px` available!).
  - **Fix:** Replace fixed `240.0` with fluid percentage width matching `settings.rs:362-382`:
    ```rust
    view()
        .w_full()
        .h(px(6.0))
        .rounded_full()
        .overflow_hidden()
        .bg(pill_glass_surface)
        .child(
            view()
                .h_full()
                .w(percentage(fraction.clamp(0.0, 1.0)))
                .rounded_full()
                .bg(move || theme().primary),
        )
    ```

### 9.2 Recoverable Updater Error with Retry CTA (`F-11`)
- **Lines 345–348 (`UpdatePhase::Error(err)`):**
  - Previously: Bare red text `format!("Update check failed: {err}")` with no recovery step and no Retry button.
  - **Fix:**
    ```rust
    UpdatePhase::Error(err) => {
        let retry_cb = props.on_check_now.clone();
        view()
            .p_3()
            .gap_2()
            .rounded(radius_card)
            .bg(move || theme().muted)
            .child(
                text(crate::ui::text_format::format_recoverable_error(
                    "Couldn't check for updates",
                    &err,
                    "Check your internet connection and try again.",
                ))
                .text_xs()
                .text_color(move || theme().destructive),
            )
            .child(secondary_button("Retry Check", move || retry_cb()))
            .into_element()
    }
    ```

### 9.3 Focus Rings, Narrow Wrap & Dismiss Button Alignment (`F-01`, `F-08`)
- **Lines 104, 131 (`Stable`/`Beta` channel toggles):** Replace `button()` with `focusable_button()`. Add `.flex_wrap()` on the channel row (`:89`) so `"Update Channel"` + toggles wrap cleanly at `320px`.
- **Lines 366–403 (`updater_top_banner`):**
  - Add `.flex_wrap()` on line 367 and `.rounded(radius_card)` on mobile so the banner doesn't clip at `320px`.
  - Replace `button()` (`:390`) with `focusable_button().items_center().justify_center()`.

---

## 10. U4 Verification Checklist
1. `cargo check --workspace` passes with zero errors and zero unused-import warnings.
2. Confirm ZERO occurrences of `rgb(`, `rgba(`, or `0xFF` across all 9 owned files (including comments).
3. Confirm `grep -rn "scrollbar(false)" src/screens/` returns 0 matches in U4-owned files.
4. Confirm every `input()` in U4-owned files sets `set_text_input_focused` on focus/blur and uses `theme().input` border.
5. Confirm `grep -n "settings_sections" src/screens/library.rs` returns 0 matches and `nav-settings-shortcut` is intact.
