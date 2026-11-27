# U5: Web Preview Bridge (`src/App.tsx`, `src/index.css`, `index.html`, `server.ts`) & Pure-Rust Serializer (`src/bin/nsh_preview.rs`)

> **Wave:** 2 (Parallel after Wave 1 `U1` passes `cargo check`)
> **Findings Resolved:** `F-01`, `F-02`, `F-03`, `F-06`, `F-08`, `F-11`, `F-12`, `F-14`, `F-15`, `F-18`, `F-19`, `F-21`, `F-22`, `F-23`
> **Exclusive Owned Paths:**
> - `src/App.tsx`
> - `src/previewState.ts`
> - `src/index.css`
> - `index.html`
> - `server.ts`
> - `src/bin/nsh_preview.rs`

> **CRITICAL REPO INVARIANTS:**
> - `src/App.tsx` MUST remain a thin Pure-Rust NSH host/renderer — NEVER re-implement app screens or playback logic in TypeScript!
> - Use `bun` / `bunx` (never `npm` / `npx`).
> - Zero `window.alert` / `window.open`.
> - Dev server remains on port `3000`.

---

## 1. Deep Research Summary of Web Bridge & Serializer Defects

Live Chromium inspection (`playwright-cli` at `1440×900` and `320×700`) plus source audit uncovered **14 specific defects** in the preview host and serializer:

1. **Missing Focus Indicators (`F-01`):** `src/App.tsx:123` sets `outline: 'none'` on `<input>` with zero `:focus-visible` replacement; header buttons (`:425`, `:438`, `:451`, `:474`, `:484`, `:504`) lack `focus-visible:outline-2`.
2. **Label in Name Violation (`F-02`):** `src/App.tsx:453` sets `aria-label="Toggle Color Scheme"` on a button whose visible text is `"Light"` / `"Dark"` (WCAG 2.5.3 failure!).
3. **Pointer-Only Clickable `<div>` (`F-03`):** `src/App.tsx:287-297` renders nested pressable NSH nodes as `<div onClick={...}>` with no `role`, no `tabIndex`, and no `Enter`/`Space` keyboard handler!
4. **Failing Hover & Accent Contrast (`F-06`, `F-14`):** `App.tsx` hardcodes old crimson `#d92638` (`bg-[#d92638]` / `hover:bg-[#e13244]` = `4.43:1` < `4.5:1`) and 15+ raw hex literals instead of semantic CSS variables calibrated to Raag's Warm Saffron-Copper palette (`theme.json`).
5. **Raw JSON Error Exposure (`F-11`):** When `server.ts:117` returns `503 {"error":"Pure Rust NSH Preview Engine is starting up","detail":"fetch failed"}`, `App.tsx:318` passes `await res.text()` verbatim to `<p>{error}</p>`, dumping raw JSON braces to end users!
6. **Fixed `px` Typography, iOS Input Zoom & Sub-12px Text (`F-12`):**
   - `App.tsx:120` uses `${node.inputFontSize || 14}px` (`< 16px` triggers iOS Safari auto-zoom on mobile inputs!).
   - `App.tsx:175` uses `lineHeight: firstSpan?.lineHeight ? `${firstSpan.lineHeight}px` : 1.25` (fixed px line-height + cramped `1.25` ratio instead of unitless `1.4` / scaled `rem`).
   - `App.tsx:193` uses fixed `px` fontSize instead of `rem` (`(s.fontSize / 16)rem`) so browser text zoom respects user preferences.
   - `App.tsx:417` (`text-[11px]`) and `App.tsx:534` (`text-[10px]`) drop below the `12px` floor.
7. **Non-Concentric Radii, Bare `transition` & Missing Scroll Containment (`F-15`, `F-23`):**
   - `App.tsx:517` (`rounded-[38px] p-2.5` = 10px pad + 1px border -> inner radius should be `27px`, not `28px`; Desktop `rounded-2xl` 16px - 7px = `9px`, not `12px`).
   - `App.tsx:428,440,459,478,487` use Tailwind bare `transition` (which animates `all` properties including `box-shadow` and layout). Replace with explicit `transition-colors duration-150`.
   - `App.tsx:240` scroll container lacks `overscrollBehavior: 'contain'`, causing scroll chaining into the outer browser page.
8. **Missing `<h1>` Landmark & Decorative Icons Exposed to AT (`F-18`):**
   - `App.tsx:414` renders the host title as `<div>` instead of `<h1>`.
   - Lucide icons (`Smartphone`, `Monitor`, `Sun`, `Moon`, `RefreshCw`, `Download`) lack `aria-hidden="true"`, and decorative status bar (`9:41`, `5G 100%`) lacks `aria-hidden="true"`.
9. **Physical CSS Properties (`F-19`):** `App.tsx:121-122` (`paddingLeft`/`paddingRight`) and `:275` (`textAlign: 'left'`) should use logical properties (`paddingInlineStart`, `paddingInlineEnd`, `textAlign: 'start'`).
10. **Missing Tabular Numerals & Typographic Ellipsis (`F-21`, `F-22`):** `App.tsx:418,578` use `...` instead of `…`; `:490` APK MB size lacks `tabular-nums`.
11. **Missing `320px` Reflow Preset & Keyboard/A11y Bridge in `nsh_preview.rs` (`F-01`, `F-03`, `F-08`, `F-18`):**
    - `nsh_preview.rs` only supports `"android"` (`390×844`) and `"desktop"` (`1180×780`). Adding `"compact"` (`320×640`) + optional custom `width`/`height` + `/api/nsh/key` (`Escape` / keyboard shortcuts) + serializing `a11yRole`, `a11yLive`, `isDisabled`, and `isFocused` unlocks full live keyboard & 320px verification in Chromium!

---

## 2. Exact Implementation Plan

### Step 1: Upgrade `src/index.css` with Semantic CSS Variables & Motion/Focus Rules (`F-01`, `F-06`, `F-14`, `F-23`)

Replace `src/index.css` with:

```css
@import "tailwindcss";
@source not "../_refs";
@source not "../target";
@source not "../crates";
@source not "../platform";

:root {
  --host-bg: #09090b;
  --host-surface: #0f0e0d;
  --host-elevated: #181615;
  --host-elevated-hover: #24201d;
  --host-border: #2e2a27;
  --host-border-strong: #3d3733;
  --host-fg: #f5f5f5;
  --host-muted-fg: #a8a29e;
  --host-primary: #eb7a3c;
  --host-primary-hover: #f28c54;
  --host-primary-fg: #0a0a0c;
  --host-ring: #eb7a3c;
  --host-viewport-dark: #09090b;
  --host-viewport-light: #faf9f7;
  --host-status-dark: #a8a29e;
  --host-status-light: #575a61;
  --host-pill-dark: rgba(245, 245, 245, 0.28);
  --host-pill-light: rgba(17, 18, 20, 0.25);
  --host-scrollbar: rgba(168, 162, 158, 0.35);
}

.nsh-scroll {
  scrollbar-width: thin;
  scrollbar-color: var(--host-scrollbar) transparent;
  overscroll-behavior: contain;
}
.nsh-scroll::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
.nsh-scroll::-webkit-scrollbar-thumb {
  background: var(--host-scrollbar);
  border-radius: 9999px;
}
.nsh-scroll::-webkit-scrollbar-track {
  background: transparent;
}

.nsh-input:focus-visible,
.nsh-pressable:focus-visible,
.host-focus:focus-visible {
  outline: 2px solid var(--host-ring);
  outline-offset: 2px;
}

@media (prefers-reduced-motion: reduce) {
  .host-spin {
    animation: none !important;
  }
  .host-transition {
    transition-duration: 0.01ms !important;
  }
}
```

*(Contrast proof on Host Controls: `--host-primary-fg` `#0a0a0c` on `--host-primary` `#eb7a3c` = **6.95:1**, on `--host-primary-hover` `#f28c54` = **8.09:1**! `--host-fg` `#f5f5f5` on `--host-surface` `#0f0e0d` = **17.64:1**; `--host-muted-fg` `#a8a29e` on `--host-surface` `#0f0e0d` = **7.51:1**! Zero contrast failures anywhere in the host chrome!)*

---

### Step 2: Update `src/bin/nsh_preview.rs` (320px Preset, A11y Fields, Key Dispatch) (`F-03`, `F-08`, `F-18`)

1. **Extend `SerializedNode` (`:141`) and internal `NodeData`:**
   Add optional accessibility fields serialized from `MountItem::ViewProps`:
   ```rust
   a11y_role: Option<String>,
   a11y_live: Option<String>,
   disabled: bool,
   ```
   In `MountItem::ViewProps { id, props }` (`:436-444`), inspect `props.accessibility` fields available in `nsh_core =0.7.0` (check exact field names via `cargo check --bin nsh_preview`; e.g., `props.accessibility.label`, `props.accessibility.role`, `props.accessibility.live_region`, `props.accessibility.disabled` — map any present field into `SerializedNode` and leave unsupported fields `None` without inventing APIs).
2. **Extend `ConfigReq` (`:617`) and `make_environment` (`:634`):**
   ```rust
   #[derive(Deserialize)]
   struct ConfigReq {
       device: Option<String>,
       scheme: Option<String>,
       width: Option<f32>,
       height: Option<f32>,
   }
   ```
   Support `"compact"` (`320.0 × 640.0`, `Os::Android`) alongside `"android"` (`390.0 × 844.0`, `Os::Android`) and `"desktop"` (`1180.0 × 780.0`, `Os::Linux`). If `cfg.width` / `cfg.height` are provided, clamp to `[300.0..=2560.0]` and `[480.0..=1600.0]` and override viewport size!
3. **Add `POST /api/nsh/key` endpoint (`F-03`):**
   ```rust
   #[derive(Deserialize)]
   struct KeyReq {
       key: String,
   }
   ```
   Dispatch `EventKind::KeyDown` with `nsh_core::KeyEvent` (matching `nsh_core`'s exact `KeyEvent` constructor used by `nsh_testing::Headless::key(...)`) on `ROOT_ID` so pressing `Escape` or player shortcuts (`Space`, `l`, `m`, `n`, `p`, `/`) in the web host drives the REAL Rust NSH keyboard handler!
4. **Update `device` field in `NshTreeResponse` / `src/previewState.ts`:**
   Update `device: 'compact' | 'android' | 'desktop'` and add `a11yRole?: string | null; a11yLive?: string | null; disabled?: boolean;` to `NshNode` in `src/previewState.ts`.

---

### Step 3: Upgrade `src/App.tsx` (`F-01`, `F-02`, `F-03`, `F-06`, `F-11`, `F-12`, `F-14`, `F-15`, `F-18`, `F-19`, `F-21`, `F-22`, `F-23`)

Apply these exact fixes in `src/App.tsx`:

1. **Friendly Error Parsing (`F-11`) — replace lines 317–329:**
   Add a pure helper `formatHostError(status: number, rawText: string): string` (export from `src/previewState.ts` so `node:test` can unit-test it!):
   ```ts
   export function formatHostError(status: number, rawBody: string): string {
     try {
       const parsed = JSON.parse(rawBody) as { error?: string; detail?: string };
       if (status === 503) {
         return 'The Pure Rust NSH preview engine is still starting up. Wait a moment or select Retry Connection.';
       }
       if (parsed.error) {
         return `${parsed.error}. Select Retry Connection to try again.`;
       }
     } catch {
       // Fall through to plain-text handling
     }
     if (status === 503) {
       return 'The Pure Rust NSH preview engine is still starting up. Wait a moment or select Retry Connection.';
     }
     return `Preview engine returned HTTP ${status}. Check that nsh_preview is built and select Retry Connection.`;
   }
   ```
   Use `formatHostError(res.status, msg)` in `fetchTree()` so raw JSON (`{"error":...,"detail":"fetch failed"}`) NEVER reaches the UI!

2. **Accessible `<input>` (`F-01`, `F-12`, `F-19`) — lines 99–127:**
   - Remove `outline: 'none'` and add `className="nsh-input"` (which provides `:focus-visible` `2px solid var(--host-ring)`).
   - Change `fontSize` to `max(1rem, ${(node.inputFontSize || 16) / 16}rem)` (guarantees `>= 16px` effective size so iOS Safari never auto-zooms on focus!).
   - Replace `paddingLeft` / `paddingRight` with logical `paddingInlineStart: Math.max(10, node.contentX)` and `paddingInlineEnd: 10`.
   - Use `color: node.inputColor ?? 'var(--host-fg)'`.

3. **Scalable `rem` Text & Unitless Line Height (`F-12`, `F-19`) — lines 140–208:**
   - Convert span `fontSize` from `${s.fontSize || 14}px` to `${(s.fontSize || 14) / 16}rem`.
   - Convert `lineHeight`:
     ```ts
     const baseFontPx = firstSpan?.fontSize || 14;
     const computedLineHeight = firstSpan?.lineHeight
       ? Number((firstSpan.lineHeight / baseFontPx).toFixed(3))
       : 1.4;
     ```
     (Unitless ratio `>= 1.35`, scaling cleanly when users zoom text to 200%!)
   - Map `node.textAlign`: `'left' -> 'start'`, `'right' -> 'end'`, `'center' -> 'center'`, `'justify' -> 'justify'` (`F-19`).

4. **Scroll Containment (`F-15`) — lines 223–257:**
   - Add `overscrollBehavior: 'contain'` to the scrollable container style (also reinforced by `.nsh-scroll`).

5. **Eliminate Pointer-Only Clickable `<div>` & Fix Button Focus/Logical Props (`F-01`, `F-03`, `F-19`) — lines 259–305:**
   - On `<button>` (`:262-282`):
     - Replace `textAlign: 'left'` with `textAlign: 'start'`.
     - Replace `focus-visible:outline-red-500` with `nsh-pressable` (uses `var(--host-ring)` Saffron-Copper!).
     - Pass `disabled={node.disabled ?? false}`.
   - On nested pressable fallback (`:286-305`):
     - **CRITICAL (`F-03`):** If `node.pressable` is true even when `insideButton` is true, DO NOT emit a pointer-only `<div>`! Add:
       ```tsx
       role={node.pressable ? 'button' : (node.a11yRole ?? undefined)}
       tabIndex={node.pressable ? 0 : undefined}
       aria-live={node.a11yLive === 'polite' || node.a11yLive === 'assertive' ? node.a11yLive : undefined}
       className={node.pressable ? 'nsh-pressable' : undefined}
       onKeyDown={
         node.pressable
           ? (e) => {
               if (e.key === 'Enter' || e.key === ' ') {
                 e.preventDefault();
                 e.stopPropagation();
                 onPress(node.id);
               }
             }
           : undefined
       }
       ```
   - Wire global `window.addEventListener('keydown', ...)` inside `App()` so pressing `Escape` sends `POST /api/nsh/key` with `{ key: 'Escape' }` when a preview tree is mounted!

6. **Fix Host Header Landmarks, Label in Name, Touch Targets, Contrast, Radii & Icons (`F-01`, `F-02`, `F-06`, `F-12`, `F-14`, `F-15`, `F-18`, `F-21`, `F-22`, `F-23`):**
   - **Line 381 (Logo SVG):** Mark `aria-hidden="true"` (adjacent `<h1>` already names the app) and update gradient stops to Raag's Saffron-Copper (`#a8490b` -> `#eb7a3c`).
   - **Line 414:** Change `<div className="text-sm font-semibold leading-tight">` to `<h1 className="text-sm font-semibold leading-snug">` (`F-18`).
   - **Line 417:** Change `text-[11px]` to `text-xs` (`12px` floor, `F-12`) and replace `...` with `…` (`F-21`).
   - **Lines 424–449 (Viewport Switcher):** Add a third `320px` preset button (`320px` / `Android 390×844` / `Desktop`), set `aria-pressed={device === ...}`, use `bg-[var(--host-primary)] text-[var(--host-primary-fg)]` when active (`6.95:1` contrast!), add `host-focus host-transition transition-colors duration-150`, and set `aria-hidden="true"` on `<Smartphone>` / `<Monitor>` icons (`F-01`, `F-06`, `F-18`, `F-23`).
   - **Lines 451–472 (Theme Toggle — WCAG 2.5.3 Label in Name Fix `F-02`):**
     - **DELETE** mismatched `aria-label="Toggle Color Scheme"`.
     - Instead let the visible text serve as the accessible name (or set `aria-label={scheme === 'dark' ? 'Switch to Light theme' : 'Switch to Dark theme'}` paired with visible text `{scheme === 'dark' ? 'Light theme' : 'Dark theme'}` so the visible label is a strict substring of the accessible name!).
     - Mark `<Sun>` / `<Moon>` with `aria-hidden="true"`.
   - **Lines 474–481 (Refresh button):** Keep `aria-label="Refresh preview frame"`, add `host-focus host-transition transition-colors duration-150`, mark `<RefreshCw className={... host-spin} aria-hidden="true" />`.
   - **Lines 483–492 (Download APK):** Replace `bg-[#d92638] hover:bg-[#e13244] text-white` with `bg-[var(--host-primary)] hover:bg-[var(--host-primary-hover)] text-[var(--host-primary-fg)] tabular-nums host-focus host-transition transition-colors duration-150` (`6.95:1` / `8.09:1` contrast, `F-06`, `F-22`), and mark `<Download aria-hidden="true" />`.
   - **Lines 516–530 (Concentric Frame Radii `F-15`):**
     - Android/Compact frame: outer `rounded-[38px] p-2.5 border` (10px + 1px = 11px) -> inner `borderRadius: '27px'`.
     - Desktop frame: outer `rounded-2xl` (`16px`) `p-1.5 border` (6px + 1px = 7px) -> inner `borderRadius: '9px'`.
     - Viewport backgrounds: `var(--host-viewport-dark)` / `var(--host-viewport-light)`.
   - **Lines 532–543 & 561–573 (Decorative Android status bar & gesture bar):** Add `aria-hidden="true"` and bump `text-[10px]` to `text-xs` (`12px`) with `tabular-nums` and `var(--host-status-dark)` / `var(--host-status-light)` (`F-12`, `F-18`).
   - **Line 578:** Replace `Booting Pure Rust NSH Engine...` with `Booting Pure Rust NSH Engine…` (`F-21`).

---

### Step 4: Verify `server.ts` & `index.html`

- **`server.ts:116-121`:** Keep structured `503` JSON `{ error: 'Pure Rust NSH Preview Engine is starting up', detail }` for API callers while ensuring `detail` never leaks stack traces or paths, and add graceful auto-build hint in logs if binary is missing.
- **`index.html`:** Verify `<meta name="theme-color" content="#09090b" />` and `<html lang="en">` remain valid (already clean!).

---

## 3. U5 Verification Checklist
1. `bun run lint` (`tsc --noEmit`) passes with zero errors.
2. `cargo check --bin nsh_preview` passes with zero errors.
3. Unit tests (`node --test`) verify `formatHostError(503, '{"error":"...","detail":"fetch failed"}')` returns clean, human-readable text with zero JSON braces.
4. `grep -nE "#d92638|#e13244|outline: 'none'|Toggle Color Scheme|text-\[10px\]|text-\[11px\]" src/App.tsx` returns **0 matches**.
5. `grep -nE "paddingLeft|paddingRight|textAlign: 'left'" src/App.tsx` returns **0 matches**.
