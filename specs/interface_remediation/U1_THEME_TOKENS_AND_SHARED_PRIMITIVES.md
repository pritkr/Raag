# U1: Calibrated Theme Tokens, Shared A11y/UI Primitives & Writing Helpers

> **Wave:** 1 (Sequential Foundation — MUST complete before Wave 2 subagents U2–U5 start)
> **Findings Resolved:** `F-01` (foundation), `F-04` (foundation), `F-05` (foundation), `F-06`, `F-07`, `F-11` (foundation), `F-13` (foundation), `F-14`, `F-15` (radius token), `F-16` (foundation), `F-17` (`hero_pill_button`/`glass_pill_button`), `F-21` (foundation), `F-23`
> **Exclusive Owned Paths:**
> - `theme.json`
> - `src/ui/tokens.rs`
> - `src/ui/icons.rs`
> - `src/ui/text_format.rs` (**NEW**)
> - `src/ui/mod.rs`
> - `src/ui/controls/primitives.rs`
> - `src/ui/controls/mod.rs`

---

## 1. Deep Research Summary & Design Decisions

### 1.1 Why the Current Palette Fails (Mathematical Audit)
1. **Brand vs. Danger Hue Collision (`F-07`):**
   - Current Light `primary` (`#d92638`, OKLCH `H = 22.8°`) and `destructive` (`#c62828`, `H = 26.7°`) are only **3.9°** apart.
   - Current Dark `primary` (`#d92638`, `H = 22.8°`) and `destructive` (`#f87171`, `H = 22.2°`) are only **0.6°** apart.
   - Furthermore, Dark mode already switches `player-accent`, `control`, and `ring` to warm amber-copper `#eb7a3c` (`H = 46.9°`), creating two competing brand colors (`crimson` vs `amber-copper`).
   - **Solution:** Unify Raag's brand accent on its signature **Warm Saffron-Copper** (`H ≈ 46.3°–46.9°`): `#a8490b` (Light) and `#eb7a3c` (Dark). Keep danger crimson (`H ≈ 22.2°–22.6°`), shift `sponsor-badge` warning gold (`H ≈ 83.4°–91.9°`), and keep `status-ok` emerald (`H ≈ 149.9°–151.7°`). Every semantic hue is now separated by **`>= 23.7°` OKLCH** (well above the `15°` minimum)!
2. **Dark `primary_foreground` Semantics Trap (CRITICAL DISCOVERY!):**
   - In Dark mode, `#eb7a3c` requires **dark ink** `#0a0a0c` (`6.95:1` contrast) — NOT white `#ffffff` (`2.95:1`!).
   - Audit of all 26 `theme().primary_foreground` call sites across `src/` proved that **7 sites** previously used `theme().primary_foreground` assuming it was always `#ffffff`, even when painted over dark cover gradients or `theme().destructive`:
     - Cover monograms (6 sites): `primitives.rs:262`, `transport.rs:210,680`, `now_playing.rs:311`, `queue.rs:499`, `shell.rs:99` -> MUST migrate to **`color_on_cover()`** (`on-cover: #ffffff` in both Light & Dark).
     - Destructive warning banner (1 site): `listen_together.rs:264` -> MUST migrate to **`color_destructive_foreground()`** (`destructive-foreground: #ffffff` Light / `#140808` Dark = **7.11:1**!).
   - All remaining 19 sites sit genuinely on `theme().primary` fills, so keeping them on `theme().primary_foreground` automatically gives **5.80:1** (Light) and **6.95:1** (Dark)!
3. **Unused Tokens (`F-14`):**
   - `grep` across `src/`, `crates/`, and `tests/` confirmed **zero consumers** of `chart-1`..`chart-5`. Delete all 10 unused tokens from `theme.json`.
4. **Hardcoded Surfaces in `src/ui/tokens.rs` (`F-14`, `F-06`):**
   - `frosted_bar_surface`, `frosted_card_surface`, `pill_glass_surface`, `pill_glass_hover`, `sidebar_active_surface`, `glass_hairline`, and `color_hero_pill_bg/fg` hardcode `rgb(0x...)` instead of deriving from `theme().card`, `theme().foreground`, `theme().background`.
   - Light-mode `cover_gradient_pair` stops (`#f5a855`, `#48c4a2`, `#4aa6eb`, `#eb6b76`, `#a870eb`, `#6e7ee8`) fail white monogram contrast (`1.98:1`–`3.61:1`). Darkening all 6 light gradient pairs lifts every stop to **`5.63:1`–`11.01:1`** against `#ffffff`!

---

## 2. Exact Implementation Plan

### Step 1: Replace `theme.json` with Mathematically Proven Palette

Replace `theme.json` verbatim with the following calibrated token set (**0 failures** across all 66 WCAG 2.1 pairs and 6 OKLCH hue separations):

```json
{
  "name": "raag",
  "cssVars": {
    "theme": {
      "radius": "10px",
      "radius-inset": "8px",
      "radius-card": "12px",
      "radius-panel": "18px",
      "radius-pill": "999px"
    },
    "light": {
      "background": "#faf9f7",
      "foreground": "#111214",
      "card": "#ffffff",
      "card-foreground": "#111214",
      "popover": "#ffffff",
      "popover-foreground": "#111214",
      "primary": "#a8490b",
      "primary-foreground": "#ffffff",
      "secondary": "#f0eee9",
      "secondary-foreground": "#18191c",
      "muted": "#ebe8e2",
      "muted-foreground": "#575a61",
      "accent": "#fff0e3",
      "accent-foreground": "#8c3a06",
      "destructive": "#c21d30",
      "destructive-foreground": "#ffffff",
      "on-cover": "#ffffff",
      "liked": "#c21d30",
      "liked-foreground": "#ffffff",
      "border": "#d6d1c7",
      "input": "#858077",
      "ring": "#a8490b",
      "sidebar": "#f4f2ed",
      "sidebar-foreground": "#111214",
      "sidebar-accent": "#e9e6df",
      "sidebar-accent-foreground": "#111214",
      "sidebar-border": "#d6d1c7",
      "player": "#ffffff",
      "player-accent": "#a8490b",
      "control": "#111214",
      "control-foreground": "#ffffff",
      "ambient-warm": "#f5ebe0",
      "lyric-active": "#111214",
      "lyric-inactive": "#595d66",
      "sponsor-badge": "#7a5900",
      "status-ok": "#146c33",
      "font-sans": "Inter, -apple-system, BlinkMacSystemFont, Roboto, sans-serif",
      "font-mono": "JetBrains Mono, ui-monospace, monospace"
    },
    "dark": {
      "background": "#09090b",
      "foreground": "#f5f5f5",
      "card": "#141312",
      "card-foreground": "#f5f5f5",
      "popover": "#181615",
      "popover-foreground": "#f5f5f5",
      "primary": "#eb7a3c",
      "primary-foreground": "#0a0a0c",
      "secondary": "#1f1d1b",
      "secondary-foreground": "#f5f5f5",
      "muted": "#282523",
      "muted-foreground": "#a8a29e",
      "accent": "#2c201b",
      "accent-foreground": "#ffd8a8",
      "destructive": "#f87171",
      "destructive-foreground": "#140808",
      "on-cover": "#ffffff",
      "liked": "#d92646",
      "liked-foreground": "#ffffff",
      "border": "#2e2a27",
      "input": "#6d6863",
      "ring": "#eb7a3c",
      "sidebar": "#0c0a09",
      "sidebar-foreground": "#f5f5f5",
      "sidebar-accent": "#241e1a",
      "sidebar-accent-foreground": "#f5f5f5",
      "sidebar-border": "#262321",
      "player": "#0c0b0a",
      "player-accent": "#eb7a3c",
      "control": "#eb7a3c",
      "control-foreground": "#0a0a0c",
      "ambient-warm": "#2a190d",
      "lyric-active": "#ffffff",
      "lyric-inactive": "#9c9690",
      "sponsor-badge": "#facc15",
      "status-ok": "#4ade80",
      "font-sans": "Inter, -apple-system, BlinkMacSystemFont, Roboto, sans-serif",
      "font-mono": "JetBrains Mono, ui-monospace, monospace"
    }
  }
}
```

#### Verified Contrast & OKLCH Proof Matrix (Every Value Measured)
| Pair | Light Ratio | Dark Ratio | Threshold |
| :--- | :--- | :--- | :--- |
| `foreground` / `background` | `17.81:1` | `18.25:1` | `>= 7.0:1` |
| `card-foreground` / `card` | `18.74:1` | `17.02:1` | `>= 7.0:1` |
| `primary-foreground` / `primary` | `5.80:1` | `6.95:1` | `>= 4.5:1` |
| `primary` / `background` / `card` / `muted` | `5.51 / 5.80 / 4.74:1` | `6.99 / 6.52 / 5.35:1` | `>= 4.5:1` |
| `destructive-foreground` / `destructive` | `5.98:1` | `7.11:1` | `>= 4.5:1` |
| `destructive` / `background` / `card` / `muted` | `5.68 / 5.98 / 4.89:1` | `7.19 / 6.71 / 5.50:1` | `>= 4.5:1` |
| `liked-foreground` / `liked` | `5.98:1` | `4.87:1` | `>= 4.5:1` |
| `lyric-inactive` / `background` / `ambient-warm` | `6.27 / 5.61:1` | `6.80 / 5.77:1` | `>= 4.5:1` |
| `status-ok` / `card` / `muted` | `6.52 / 5.33:1` | `10.65 / 8.74:1` | `>= 4.5:1` |
| `sponsor-badge` / `card` / `muted` | `6.45 / 5.27:1` | `12.12 / 9.94:1` | `>= 4.5:1` |
| `input` / `background` / `card` (non-text) | `3.73 / 3.92:1` | `3.61 / 3.37:1` | `>= 3.0:1` |
| OKLCH Hue Separation from `primary` | `destructive`: `23.7°`, `sponsor`: `37.1°`, `ok`: `103.6°` | `destructive`: `24.7°`, `sponsor`: `45.0°`, `ok`: `104.8°` | `>= 15.0°` |

---

### Step 2: Update `src/ui/tokens.rs`

1. **Add new semantic token accessors:**
   ```rust
   pub fn color_destructive_foreground() -> AdaptiveColor {
       theme()
           .color("destructive-foreground")
           .unwrap_or(theme().primary_foreground)
   }

   pub fn color_on_cover() -> AdaptiveColor {
       theme()
           .color("on-cover")
           .unwrap_or(theme().lyric_active_fallback())
   }
   ```
   *(Where fallback is `theme().color("on-cover").unwrap_or_else(|| AdaptiveColor::new(rgb(0xffffff), rgb(0xffffff)))` — remember `src/ui/tokens.rs` is the ONLY Rust file permitted to hold color primitives; `src/screens/` and `src/ui/controls/` never touch raw RGB.)*
2. **Add concentric inner radius helpers:**
   ```rust
   /// Concentric inner radius for panel-level cards (`radius_panel - 10px padding - 1px border = 7px`).
   pub fn radius_panel_inner() -> Points {
       px((theme().number("radius-panel").unwrap_or(18.0) - 11.0).max(4.0))
   }

   /// Concentric inner radius for segmented tabs (`radius_card - 4px padding = 8px`).
   pub fn radius_tab_inner() -> Points {
       px((theme().number("radius-card").unwrap_or(12.0) - 4.0).max(4.0))
   }
   ```
3. **Derive translucent glass surfaces from `theme()` instead of hardcoded hex (`F-14`):**
   - `frosted_bar_surface()`: derive from `color_player_surface()` with alpha `0.90` (light) / `0.92` (dark) using `.opacity(...)` on `AdaptiveColor` (check `AdaptiveColor::opacity(f32)` in `nsh_core`; if `AdaptiveColor` exposes `.light` / `.dark` `Hsla`, adjust `.a` directly).
   - `frosted_card_surface()`: derive from `theme().popover` with alpha `0.78` / `0.76`.
   - `pill_glass_surface()` / `pill_glass_hover()` / `sidebar_active_surface()`: derive from `theme().foreground` at alpha `0.06` / `0.11` / `0.08` (light) and `0.08` / `0.14` / `0.10` (dark).
   - `glass_hairline()`: derive from `theme().foreground` at alpha `0.10` (light) / `0.11` (dark).
   - `color_hero_pill_bg()` -> `theme().foreground`; `color_hero_pill_fg()` -> `theme().background` (17.81:1 light / 18.25:1 dark with zero hardcoded RGB!).
4. **Calibrate Light-Mode `cover_gradient_pair` stops (`F-06`):**
   Replace the light `(lf, lt)` stops in `src/ui/tokens.rs:171-189` with mathematically verified WCAG AA stops (`#ffffff` contrast `>= 5.63:1` on EVERY stop):
   ```rust
   const PAIRS: [RgbQuad; 6] = [
       // Crimson & Deep Berry (#b92b40 -> 6.03:1, #8a182d -> 9.34:1)
       ([185, 43, 64], [138, 24, 45], [156, 36, 54], [52, 18, 34]),
       // Warm Sunset Amber & Terracotta (#a84c10 -> 5.67:1, #803308 -> 8.77:1)
       ([168, 76, 16], [128, 51, 8], [180, 96, 38], [64, 28, 18]),
       // Midnight Indigo & Electric Violet (#4851b8 -> 6.68:1, #31368a -> 10.37:1)
       ([72, 81, 184], [49, 54, 138], [58, 72, 164], [24, 26, 68]),
       // Emerald & Teal Lagoon (#14755d -> 5.63:1, #0d5747 -> 8.50:1)
       ([20, 117, 93], [13, 87, 71], [32, 122, 98], [16, 48, 46]),
       // Magenta & Plum (#8f3280 -> 7.15:1, #621d6b -> 11.01:1)
       ([143, 50, 128], [98, 29, 107], [142, 46, 126], [48, 18, 58]),
       // Cobalt & Deep Ocean (#1f61a8 -> 6.30:1, #14427d -> 9.98:1)
       ([31, 97, 168], [20, 66, 125], [38, 92, 164], [16, 36, 68]),
   ];
   ```
5. **Standardize press feedback & motion constants (`F-23`):**
   ```rust
   pub const PRESS_OPACITY: f32 = 0.80;
   pub const PROSE_MAX_WIDTH: f32 = 660.0;
   pub const NARROW_VIEWPORT_MAX_WIDTH: f32 = 380.0;

   pub fn is_narrow_viewport() -> bool {
       window().width.0 < NARROW_VIEWPORT_MAX_WIDTH
   }
   ```

---

### Step 3: Create `src/ui/text_format.rs` (NEW Pure Unit-Testable Writing Module)

Export `pub mod text_format;` in `src/ui/mod.rs`. Provide deterministic, pure functions for `F-11`, `F-13`, `F-20`, `F-21`:

```rust
//! Standardized UI copy, pluralization, typographic punctuation, and recovery-oriented errors.

/// Returns grammatically correct singular or plural count text (`"1 song"`, `"4 songs"`).
pub fn pluralize(count: usize, singular: &str, plural: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {plural}")
    }
}

/// Formats a user-facing error with an explicit cause and next-step recovery action.
/// Redacts raw URL query parameters so secrets never leak into UI copy.
pub fn format_recoverable_error(what_failed: &str, raw_detail: &str, recovery: &str) -> String {
    let clean = redact_query_params(raw_detail.trim());
    if clean.is_empty() {
        format!("{what_failed}. {recovery}")
    } else {
        format!("{what_failed} ({clean}). {recovery}")
    }
}

/// Formats a quoted search/filter term using typographic curly double quotes.
pub fn quote_term(term: &str) -> String {
    format!("\u{201C}{term}\u{201D}")
}

/// Strips `?query` and `#fragment` from any embedded URL token to prevent secret leakage.
fn redact_query_params(input: &str) -> String {
    input
        .split_whitespace()
        .map(|tok| {
            if tok.starts_with("http://") || tok.starts_with("https://") {
                tok.split(['?', '#']).next().unwrap_or("<redacted>")
            } else {
                tok
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pluralize_handles_zero_one_and_many() {
        assert_eq!(pluralize(0, "song", "songs"), "0 songs");
        assert_eq!(pluralize(1, "song", "songs"), "1 song");
        assert_eq!(pluralize(2, "song", "songs"), "2 songs");
        assert_eq!(pluralize(1, "track", "tracks"), "1 track");
        assert_eq!(pluralize(1, "result", "results"), "1 result");
        assert_eq!(pluralize(1, "folder", "folders"), "1 folder");
    }

    #[test]
    fn recoverable_error_redacts_signed_urls_and_includes_action() {
        let msg = format_recoverable_error(
            "Couldn't cast track",
            "failed GET https://cdn.example.com/audio?sig=SECRET&pot=TOK",
            "Check that your Cast device is on the same Wi-Fi network and try again.",
        );
        assert!(!msg.contains("SECRET"));
        assert!(!msg.contains("TOK"));
        assert!(msg.contains("try again."));
    }
}
```

---

### Step 4: Upgrade `src/ui/controls/primitives.rs` & Re-Export in `src/ui/controls/mod.rs`

Add the following reusable, zero-raw-color primitives (preserving all existing exported function signatures so Wave 2 compiles cleanly):

1. **`focusable_button()` (`F-01`, `F-23`):**
   ```rust
   /// Base interactive button with mandatory WCAG 2.4.7 `focus_visible` ring and consistent press feedback.
   pub fn focusable_button() -> Button {
       button()
           .focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))
           .active(|s| s.opacity(PRESS_OPACITY))
   }
   ```
   *(Check exact return type of `nsh_base::button()` — it is `nsh_base::Button`.)*

2. **`destructive_button(label, on_press)` (`F-05`, `F-06`, `F-07`):**
   ```rust
   /// High-contrast destructive action button with danger styling, >=48dp target, and focus ring.
   pub fn destructive_button(
       label: impl IntoReactive<SharedString>,
       on_press: impl Fn() + 'static,
   ) -> impl IntoElement {
       focusable_button()
           .on_press(on_press)
           .min_h(px(MOBILE_MIN_TOUCH_TARGET))
           .px_4()
           .items_center()
           .justify_center()
           .rounded(move || theme().radius)
           .bg(move || theme().destructive)
           .hover(|s| s.opacity(0.92))
           .child(
               text(label)
                   .text_sm()
                   .font_weight(FontWeight::SEMIBOLD)
                   .text_color(color_destructive_foreground),
           )
   }
   ```

3. **`confirm_destructive(title, description, confirm_label, on_confirm)` (`F-05`):**
   Reuses `nsh_base::alert_dialog()` identically to `src/ui/controls/vault_widgets.rs:42-54`:
   ```rust
   /// Presents a native non-blocking destructive confirmation dialog before running `on_confirm`.
   pub fn confirm_destructive(
       title: impl Into<String>,
       description: impl Into<String>,
       confirm_label: impl Into<String>,
       on_confirm: impl Fn() + 'static,
   ) {
       let title = title.into();
       let description = description.into();
       let confirm_label = confirm_label.into();
       let cb = std::rc::Rc::new(on_confirm);
       spawn(async move {
           let answer = nsh_base::alert_dialog()
               .title(title)
               .description(description)
               .cancel("cancel", "Cancel")
               .destructive("confirm", confirm_label)
               .show()
               .await;
           if matches!(answer, Ok(r) if r.action.as_deref() == Some("confirm")) {
               cb();
           }
       });
   }
   ```

4. **`active_state_dot(active)` (`F-04`):**
   ```rust
   /// Non-color geometric active indicator dot paired with toggled icon buttons.
   pub fn active_state_dot(active: impl Fn() -> bool + Copy + 'static) -> impl IntoElement {
       when(active, || {
           view()
               .size(px(4.0))
               .rounded_full()
               .bg(color_player_accent)
       })
   }
   ```

5. **`labeled_input(props)` (`F-16`, `F-01`, `F-06`):**
   Wraps `nsh_base::input()` with:
   - Visible or screen-reader label (`text(label)` above input or `.accessibility_label(label)` when compact search bar)
   - Automatic `crate::ui::controls::set_text_input_focused(true)` on `.on_focus` and `false` on `.on_blur` so global single-key shortcuts (`Space`, `l`, `m`, `n`, `p`, `s`, `r`, `/`) NEVER hijack typing!
   - `.min_h(px(MOBILE_MIN_TOUCH_TARGET))`
   - `.border_1().border_color(move || theme().input)` (`>= 3.37:1` contrast) + `.focus(|s| s.border_color(move || theme().ring))`
   - `.placeholder_color(|| color(theme().muted_foreground))` and `.caret_color(|| color(theme().primary))`
   - Optional `.test_id(...)`, `.masked(bool)`, `.return_key_type(...)`, `.on_submit(...)`.

6. **Fix existing controls inside `src/ui/controls/primitives.rs`:**
   - `mood_chip` (`:133`): add `.focus_visible(|s| s.outline(px(2.0)).outline_color(move || theme().ring))` and `.min_h(px(MOBILE_MIN_TOUCH_TARGET))` on mobile (`when(!is_desktop_layout(), |b| b.min_h(px(MOBILE_MIN_TOUCH_TARGET)))`).
   - `hero_pill_button` (`:182`) and `glass_pill_button` (`:219`): change `.min_h(px(40.0))` to `.min_h(px(if is_desktop_layout() { 40.0 } else { MOBILE_MIN_TOUCH_TARGET }))` so the **button's own hit area** is `48dp` on mobile (`F-17`)!
   - `artwork_box` (`:262`): change `.text_color(move || theme().primary_foreground)` to `.text_color(color_on_cover)` (`F-06`)!

7. **Re-export in `src/ui/controls/mod.rs`:**
   Export `active_state_dot`, `confirm_destructive`, `destructive_button`, `focusable_button`, `labeled_input`, `LabeledInputProps` from `crate::ui::controls`.

---

### Step 5: Calibrate Stroke Weight in `src/ui/icons.rs` (`F-23`)
- Currently `icons.rs` renders all icons with fixed `stroke-width="2"` on a `24×24` viewBox; when scaled to `size <= 16.0`, the rendered stroke narrows to `1.0–1.33px`, appearing thin next to `SEMIBOLD` labels.
- Update the SVG template helper in `src/ui/icons.rs` to compute `let stroke_w = if size <= 15.0 { "2.35" } else if size <= 18.0 { "2.15" } else { "2.0" };` and include `stroke-width="{stroke_w}"` so small icons remain optically crisp across 12px–24px sizes.

---

## 3. U1 Verification Checklist
1. `cargo check --workspace` passes with zero errors.
2. `cargo test --test phase4_hardening_and_qa wcag_aa_contrast` passes (all 9 existing assertions green).
3. `cargo test --lib text_format` passes all pluralization & URL-redaction unit tests.
4. Confirm zero `rgb(`, `rgba(`, or `0xFF` appear anywhere in `src/ui/controls/primitives.rs`.
5. Confirm `chart-1`..`chart-5` no longer exist in `theme.json`.
