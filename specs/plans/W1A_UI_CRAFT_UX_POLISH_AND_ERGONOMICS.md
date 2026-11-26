# W1A — UI Craft, Layout Fixes, Product Honesty & Everyday Music Player Ergonomics

> **Wave:** Wave 1 (Parallel Agent 1A — Runs concurrently with W1B, W1C, W1D, W1E)
> **Exclusive Write Scope:**
> - `src/ui/tokens.rs`, `src/ui/icons.rs`, `src/ui/controls/{mod.rs,cards.rs,media_rows.rs,navigation.rs,primitives.rs,sheets.rs,transport.rs,context_menu.rs,sleep_timer_sheet.rs,speed_sheet.rs}`
> - `src/screens/{home.rs,now_playing.rs,queue.rs,lyrics.rs,library.rs}`
> - `src/store/{ui_slice.rs,mutation_slice.rs,library_slice.rs}`
> - `tests/w1a_ui_craft_and_ergonomics_test.rs` (new)
> **Read-Only / Stitch Hooks (For Wave 1.5 Integration Agent):**
> - Export `pub fn register_global_shortcuts()` (stitched into `src/lib.rs` in Wave 1.5)
> - Export `pub fn desktop_responsive_shell_helpers()` / shell fixes notes (stitched into `src/screens/shell.rs` & `src/screens/settings.rs` in Wave 1.5)
> **Forbidden Edits:** `Cargo.toml`, `crates/contract/**`, `src/live_engine.rs`, `src/store/mod.rs`, `src/screens/{shell,settings,explore,search}.rs`, `src/lib.rs`, `platform/**`.

---

## 1. Ground-Truth Visual & UX Defects Uncovered by Audit

Direct inspection of rendered screenshots (`r2-desktop-home.png`, `r2-mobile-home.png`, `verify-desktop-settings.png`, `verify-mobile.png`) and screen code revealed 11 high-impact craft failures:

| # | Defect | Exact Root Cause | Impact |
| :-: | :--- | :--- | :--- |
| 1 | **Desktop ~70% black dead space** | `.when(is_desktop_layout(), ...)` evaluated once at component build time (`home.rs:633`, `search.rs:245`, `shell.rs`) instead of reactive `dynamic`/`style_when` on `window().width` | Desktop mode renders a ~390px mobile column pinned left |
| 2 | **Clipped mobile bottom nav labels** | Bottom bar lacks sufficient `min_h` + `.pb_safe_area_plus(px(6.0))` and flex shrink protection | Labels ("Home", "Search", "Lyrics", "Queue", "Library") cut off at bottom edge |
| 3 | **Truncated `"Rom..."` mood chips & `< 48dp` target** | Fixed `h(px(36.0))` horizontal scroll container (`home.rs:636`) + constrained chip width | Violates `>= 48dp` touch invariant and clips short words |
| 4 | **Inverted heart icon color bug** | `media_rows.rs:291-297` checks `is_current` instead of `is_liked` for `color_liked()` | Liked tracks show grey heart; unliked playing track shows red outline heart |
| 5 | **Stark grey monogram squares in Queue** | `queue.rs:338-384` never renders `track.artwork_url` | Queue looks broken/placeholder-only even when artwork exists |
| 6 | **Crude Unicode repeat glyphs** | `now_playing.rs:581-585` draws `"↻"`, `"↻A"`, `"↻1"` text instead of SVG icons | Breaks icon consistency with `transport.rs` and `queue.rs` |
| 7 | **Hardcoded `"Opus 251 • Proxy"` badge** | `now_playing.rs:141` hardcodes string regardless of real codec (`itag 140` AAC, local FLAC, etc.) | Misleads audiophile users |
| 8 | **Settings glued to bottom of Library** | `library.rs:1052` permanently renders `settings_sections()` at bottom of Library | Clutters music library with auth cookies & DSP diagnostics |
| 9 | **Engineering jargon in consumer UI** | `"SAPISIDHASH"`, `"local 256 KB chunk cache"`, `"Lock-free SPSC ring buffer"`, `"PO Token: Degraded Fallback"` shown in primary UI/toasts | Intimidating & unpolished for everyday listeners |
| 10 | **Missing core music actions** | No track `...` context menu, no Download button anywhere, no Playlist CRUD UI, no Lyrics auto-scroll, no Sleep Timer, no Playback Speed | Fails daily-driver expectations despite backend already supporting most of them |
| 11 | **Unbound advertised keyboard shortcuts** | `shell.rs:467` advertises `Space · ←/→ · N/P · L · M`, but `src/lib.rs:29-37` only binds `space`, `right`, `left` | Broken power-user affordance |

---

## 2. Design Taste & NSH `=0.7.0` Architecture Constitution

Grounded in `.agents/skills/nsh-design-guides/references/design-guides.md` and `rust-ui.md`:

1. **Zero-Re-render Reactive Layout (`dynamic` + `window()`):**
   - **Rule:** Never branch structural desktop/mobile layouts with build-time `.when(is_desktop_layout(), ...)`.
   - Define `pub fn breakpoint_is_desktop() -> bool { window().width.0 >= 820.0 }` and `pub fn grid_columns() -> u32` (`>= 1280 => 5`, `>= 960 => 4`, `>= 640 => 3`, else `2`).
   - Wrap breakpoint-switching containers in `dynamic(move || { let cols = grid_columns(); untracked(|| build_grid(cols).key(format!("grid-{cols}"))) })` and give every flex text/pane child `.flex_1().min_w(px(0.0))` so Taffy stretches cleanly to 100% viewport width on Desktop without dead voids.
2. **Concentric Radii & Alignment Spines:**
   - Outer cards: `radius_card` (`12px`), inner artwork: `max(0, 12 - 4) = 8px` (`radius_inset`), pills/chips: `radius_pill` (`999px`) ONLY for true filter chips/status pills, never accidental capsules on tooltips or rows.
   - Repeated track rows share fixed column lanes: `[48px Artwork] [12px gap] [Flex-1 Title + Artist] [44px Like] [44px Download Status] [48px Tabular Duration] [44px Overflow ...]`.
3. **Platform-Adaptive Touch vs. Pointer Ergonomics:**
   - **Mobile:** Every pressable control has `min_h(px(48.0))` and `min_w(px(48.0))` (or `.hit_slop(EdgeInsets::all(px(8.0)))`), body text `>= 15–16px`, context actions open as native bottom `sheet()`.
   - **Desktop:** Compact hover-responsive rows (`44px`), visible focus rings (`focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))`), tooltips on icon-only controls, right-click `context_menu()` + `...` anchored `menu()`.
4. **Consumer-Friendly Copy Map (De-Jargoning):**

| Old Engineering String | New Consumer Copy | Where Diagnostics Move |
| :--- | :--- | :--- |
| `"Opus 251 • Proxy"` | Dynamic `CodecBadgeInfo::label()` (e.g. `"High · Opus 160 kbps"`, `"Lossless · FLAC"`, `"AAC 256 kbps"`, `"Offline"`) | Technical `itag`/client in **Now Playing → Audio Info** popover |
| `"PO Token: Degraded Fallback"` | `"Playback Mode: Standard"` (or hidden unless error) | **Settings → Advanced & Diagnostics** |
| `"SAPISIDHASH Cookie Auth"` | `"YouTube Music Account"` | **Settings → Advanced & Diagnostics** |
| `"local 256 KB chunk cache"` | `"Playback Cache"` | **Settings → Storage & Cache** |
| `"Listen Together (Shared Radio)"` | `"Radio Mix"` *(upgraded back to `"Listen Together"` only when W2D ships real rooms)* | — |

---

## 3. Detailed Feature & Component Architecture

### 3.1 Fix Core Visual & Component Bugs (`src/ui/controls/*`, `src/screens/{home,now_playing,queue,library}.rs`)

1. **Heart Color Fix (`src/ui/controls/media_rows.rs`):**
   - Replace `if is_current { color_liked() } else { theme().muted_foreground }` with:
     ```rust
     let is_liked = move || store::is_track_liked(&track_id);
     icon(
         if is_liked() { IconKind::HeartFilled } else { IconKind::HeartOutline },
         18.0,
         move || if is_liked() { color_liked() } else { theme().muted_foreground },
     )
     ```
   - Track row playing highlight uses `theme().primary` on the title text + subtle animated equalizer/playing indicator over the artwork thumbnail—never hijacking the heart color.
2. **Real Artwork in Queue (`src/screens/queue.rs`):**
   - Replace the monogram-only box with shared `artwork_box(track.artwork_url.clone(), track.fallback_monogram(), 44.0)` (zero blur inside `virtual_list`, native rounded clip, graceful monogram fallback only when `artwork_url.is_none()` or load fails).
3. **SVG Repeat & Shuffle Icons (`src/screens/now_playing.rs`):**
   - Replace `"↻"`, `"↻A"`, `"↻1"` text labels with `IconKind::Repeat` (muted when `None`, `theme().primary` when `All`) and `IconKind::RepeatOne` (`theme().primary` with active dot indicator underneath when `One`), matching `transport.rs`.
4. **Dynamic Truthful Codec Badge (`src/screens/now_playing.rs`):**
   - Bind to `CodecBadgeInfo` from `PlaybackSlice` (`codec`, `bitrate_kbps`, `is_offline`, `is_local`, `is_lossless`), showing e.g. `"Opus · 160 kbps"`, `"AAC · 256 kbps"`, `"Offline · M4A"`, or `"Local · FLAC"`. Hide the badge cleanly when idle/fixture has no stream metadata.
5. **Mood Chips & Mobile Bottom Bar (`src/screens/home.rs`, `src/ui/controls/navigation.rs`):**
   - Remove fixed `h(px(36.0))` on the mood chip bar; give each chip `.min_h(px(48.0))` on mobile (`.min_h(px(36.0))` on desktop), `.px_4()`, `.flex_shrink_0()`, and `number_of_lines(1)` with natural width (no truncation!).
   - Provide a reusable `mobile_bottom_nav_bar(...)` in `src/ui/controls/navigation.rs` with `.flex_shrink_0()`, `.min_h(px(62.0))`, `.pt_1p5()`, `.pb_safe_area_plus(px(6.0))`, and `min_w(px(48.0))` per item so labels are never clipped at `390×844` or smaller viewports.
6. **Clean Library Separation (`src/screens/library.rs`):**
   - Remove `settings_sections()` from `library.rs` completely. Library displays exclusively user music: **Liked Songs, Playlists, Pinned Offline, Downloads, Albums, Artists, Local Files (hook for W2C), and History**. Add a clean header gear icon on mobile (`nav-settings-shortcut`) linking to `AppTab::Settings`.

---

### 3.2 Universal Track Context Menu (`src/ui/controls/context_menu.rs`)

Build ONE canonical `track_context_menu(track: Track, context: TrackMenuContext)` component using `nsh_base::menu()` / `nsh_base::context_menu()` (desktop popover + right-click) and `nsh_base::sheet()` (mobile bottom sheet triggered by `...` button or long-press):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackMenuContext {
    General,
    Playlist { playlist_id: String, set_video_id: Option<String>, index: usize },
    Queue { queue_index: usize },
    Downloaded,
}
```

**Action Hierarchy (Ordered by User Frequency):**
1. **Header Preview** (mobile sheet): `48px` artwork + Title + Artist + Duration.
2. **Queue Actions:** `Play Next`, `Add to Queue`, `Start Radio` (fetches `autoplay_next` seeded by this track).
3. **Library & Playlist Actions:** `Like / Remove Like`, `Add to Playlist...` (opens Playlist Picker Sheet), *(if in editable Playlist)* `Remove from Playlist`, *(if in Queue)* `Remove from Queue`.
4. **Offline Action:** Dynamic state button:
   - Not downloaded → `Download for Offline`
   - Downloading → `Pause Download (XX%)`
   - Paused/Failed → `Resume Download`
   - Completed → `Remove Download` (destructive color)
5. **Navigation Actions:** `Go to Artist` (uses `artist.id` → opens Artist screen in W2B, falls back to artist search if ID absent), `Go to Album` (uses `album.id`), `View Lyrics`.
6. **Utility:** `Copy Track Link` (`https://music.youtube.com/watch?v={id}`, copies via clipboard + toast `"Link copied"`).

---

### 3.3 Download Action Surface & Playlist CRUD UI (`src/ui/controls/sheets.rs`, `src/store/mutation_slice.rs`)

1. **Download Action Surface:**
   - Every track row (`media_rows.rs`), Now Playing action bar (`now_playing.rs`), and Collection detail header (`home.rs` / `library.rs`) displays a compact, accessible download status control (`download_status_button(track)`):
     - `None`: subtle outline download icon (`IconKind::Download`)
     - `Queued / Downloading`: circular/percentage progress badge (`IconKind::Pause` on tap)
     - `Completed`: filled accent check badge (`IconKind::CheckCircle`)
     - `Failed`: warning retry icon
   - Dispatches `store::enqueue_download(track)` / `pause` / `resume` / `delete` (connecting the already-built `DownloadManager` to actual user taps!).
2. **Playlist Picker & Creator Sheet (`playlist_picker_sheet()`):**
   - Triggered by `"Add to Playlist..."` from any Track Context Menu or Now Playing.
   - Top row: `"+ New Playlist"` inline form (`title` input + `Private / Unlisted / Public` segmented picker + `Create & Add` primary button).
   - Scrollable list of user's owned playlists with track counts and instant add feedback.
   - **Optimistic Mutations with Generation-Guarded Rollback (`src/store/mutation_slice.rs`):**
     - `AddVideo`, `RemoveVideo`, `MoveVideoBefore` update local `LibrarySlice` / active playlist view immediately under a monotonic `mutation_gen: u64`.
     - If `live_engine` reports a backend failure for that `mutation_gen`, revert to the saved pre-mutation snapshot and display a clear toast (`"Couldn't update playlist — restored"`).

---

### 3.4 Synced Lyrics Auto-Scroll with User-Override Hysteresis (`src/screens/lyrics.rs`)

**State Machine:**
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyricsScrollMode {
    /// Automatically centers active line at 38% viewport height on line change.
    AutoFollow,
    /// User manually scrolled; suppress auto-scroll until `resume_at_unix_ms` or tap "Sync".
    UserBrowsing { resume_at_unix_ms: u64 },
}
```

**UX & Implementation Details:**
- Bind `virtual_list` with `let list_ref = element_ref();` and `.estimated_item_size(px(56.0))`.
- Maintain `last_scrolled_line: Stored<Option<usize>>` and `scroll_mode: Signal<LyricsScrollMode>`.
- An `effect` watches `active_line_index` (computed from `LyricsDocument::active_line_index(position_ms)`):
  - When `active_line` changes AND `scroll_mode == AutoFollow`: call `list_ref.scroll_to_index(idx, percentage(0.38), px(0.0), !reduced_motion)`.
- On `.on_scroll(...)` driven by user drag/wheel (differentiated from programmatic scroll via a `350 ms` programmatic-scroll guard timestamp): transition to `UserBrowsing { resume_at: now + 5000 }`.
- While in `UserBrowsing`, render a sleek floating `"Sync to current line"` pill at the bottom of the lyrics pane; tapping it immediately restores `AutoFollow` and scrolls to the active line.
- **Word-Level Karaoke Support:** When `line.words` is non-empty (`LyricSyncKind::Word`), render per-word opacity/color progression (`position_ms >= word.start_ms`); when `LyricSyncKind::Line`, emphasize active line (`font_weight::BOLD`, larger scale, `theme().foreground`), gently dim past lines (`opacity 0.55`), and keep upcoming lines readable (`opacity 0.72`).
- Tapping any synced line seeks playback directly to `line.start_ms` and re-engages `AutoFollow`.

---

### 3.5 Sleep Timer & Playback Speed / Pitch Controls (`src/ui/controls/{sleep_timer_sheet,speed_sheet}.rs`)

1. **Sleep Timer (`SleepTimerState` in `crates/contract/src/audio_ext.rs`):**
   - Modes: `Off`, `Countdown { ends_at_unix_ms: u64 }`, `EndOfCurrentTrack`.
   - Presets sheet: `Off`, `15 min`, `30 min`, `45 min`, `60 min`, `End of track`, plus `+5 min` extend button when active.
   - **Gentle 3-Second Fade-Out:** During the final 3,000 ms of a countdown, linearly attenuate output volume from `saved_volume -> 0.0`, pause playback at `0`, and immediately restore `saved_volume` so the user's next play session isn't muted!
   - Visual indicator: Compact moon icon + remaining `MM:SS` (or `"End of track"`) badge in Now Playing header and desktop player bar.
2. **Playback Speed & Pitch Control (`PlaybackSpeedConfig`):**
   - Range `0.50×` to `2.00×` in `0.05×` steps with one-tap chips (`0.75×`, `1.0×`, `1.25×`, `1.5×`, `2.0×`) + `"Reset to 1.0×"`.
   - `"Preserve Pitch"` toggle (default `true`, persisted in `SettingsV1`; honored by W2A's WSOLA time-stretcher and Android ExoPlayer `PlaybackParameters`).
   - Speed badge (`1.25×`) shown next to transport controls whenever `speed != 1.0`.

---

### 3.6 Queue Ergonomics & Full Global Keyboard Shortcut Map

1. **Queue Screen (`src/screens/queue.rs`):**
   - Three clear visual regions:
     - **Now Playing** (pinned top card with real artwork & live progress hairline)
     - **Up Next** (user-queued / album tracks with reorder `↑`/`↓` accessible controls + remove button + `"Clear Up Next"` + `"Save as Playlist"`)
     - **Autoplay Radio** (visually separated section header + toggle switch so listeners always understand why tracks keep playing)
2. **Global Keyboard Shortcut Manager (`pub fn register_global_shortcuts()` in `src/ui/controls/navigation.rs`):**
   - Guard: Track `text_input_focused: Signal<bool>` (updated via `on_focus`/`on_blur` on search/form inputs). Single-letter and arrow shortcuts (`space`, `n`, `p`, `l`, `m`, `s`, `r`, `left`, `right`, `up`, `down`, `/`) are **no-ops when an input has focus**; modifier shortcuts (`mod+k`, `escape`) always work!
   - Full verified binding table (using valid NSH `on_key` strings):
     - `space` → Toggle Play / Pause
     - `left` / `right` → Seek `-5s` / `+5s`
     - `shift+left` / `shift+right` → Seek `-15s` / `+15s`
     - `n` / `p` → Next / Previous Track
     - `l` → Toggle Like on current track
     - `m` → Toggle Mute
     - `s` → Toggle Shuffle (real Fisher–Yates!)
     - `r` → Cycle Repeat Mode (`Off → All → One`)
     - `up` / `down` → Volume `+5%` / `-5%`
     - `mod+k` and `/` → Focus Search tab & search input
     - `escape` → Close active sheet, context menu, or Now Playing overlay

---

## 4. Deterministic Offline Verification (`tests/w1a_ui_craft_and_ergonomics_test.rs`)

1. `heart_icon_color_follows_is_liked_not_is_current` — headless UI test verifying unliked playing track does NOT paint liked color, and liked non-playing track DOES paint liked color.
2. `mobile_and_desktop_layouts_have_zero_clipping_and_48dp_targets` — `nsh_testing::Headless` layout check at `390×844` (mobile) and `1440×900` (desktop) confirming full desktop width utilization, unclipped bottom nav, and `>= 48dp` mood chips.
3. `track_context_menu_exposes_download_playlist_and_queue_actions` — opens `...` menu in headless test, verifies Play Next, Add to Playlist, Download, and Go to Artist callbacks fire cleanly.
4. `lyrics_auto_scroll_pauses_on_user_scroll_and_resumes_on_sync` — unit test of `LyricsScrollMode` hysteresis transitions & timestamp expiry.
5. `sleep_timer_fades_volume_pauses_and_restores_saved_volume` — deterministic clock test verifying 3-second fade curve, pause trigger, End-of-Track trigger, and volume restoration.
6. `keyboard_shortcuts_ignore_letter_keys_while_input_focused` — verifies `space`/`n`/`l` fire when unfocused and are suppressed when `text_input_focused == true`.
7. `zero_raw_color_literals_and_no_settings_in_library` — static scan + headless check asserting Library tab no longer renders Settings diagnostics.

```bash
cargo test --test w1a_ui_craft_and_ergonomics_test -- --nocapture
cargo test --test headless_ui --test phase5_polish_and_ergonomics
```
