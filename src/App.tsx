/**
 * @license
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import React, { useCallback, useEffect, useReducer, useRef, useState } from 'react';
import { Download, Monitor, Moon, RefreshCw, Smartphone, Sun } from 'lucide-react';
import {
  INITIAL_PREVIEW_STATE,
  formatHostError,
  previewReducer,
  type NshBorderSide,
  type NshNode,
  type NshTreeResponse,
  type PreviewDevice,
} from './previewState';

function borderCss(side: NshBorderSide): string | undefined {
  if (side.width > 0 && side.color) {
    return `${side.width}px solid ${side.color}`;
  }
  return undefined;
}

function radiiCss(r: [number, number, number, number]): string | undefined {
  if (r[0] === 0 && r[1] === 0 && r[2] === 0 && r[3] === 0) return undefined;
  return `${r[0]}px ${r[1]}px ${r[2]}px ${r[3]}px`;
}

function collectText(node: NshNode): string {
  if (node.text) return node.text;
  return node.children.map(collectText).filter(Boolean).join(' ').trim();
}

function logicalTextAlign(
  align: NshNode['textAlign'],
): React.CSSProperties['textAlign'] {
  if (align === 'right') return 'end';
  if (align === 'center') return 'center';
  if (align === 'justify') return 'justify';
  return 'start';
}

interface NodeViewProps {
  node: NshNode;
  isRoot?: boolean;
  insideButton?: boolean;
  onPress: (id: number) => void;
  onInput: (id: number, text: string) => void;
  onSubmit: (id: number, text: string) => void;
  onScroll: (id: number, x: number, y: number) => void;
}

function NshNodeView({
  node,
  isRoot = false,
  insideButton = false,
  onPress,
  onInput,
  onSubmit,
  onScroll,
}: NodeViewProps) {
  const [draft, setDraft] = useState(node.inputValue);
  const scrollTimer = useRef<number | null>(null);

  useEffect(() => {
    setDraft(node.inputValue);
  }, [node.inputValue]);

  useEffect(() => {
    return () => {
      if (scrollTimer.current !== null) {
        window.clearTimeout(scrollTimer.current);
      }
    };
  }, []);

  const baseStyle: React.CSSProperties = {
    position: isRoot ? 'relative' : 'absolute',
    insetInlineStart: isRoot ? 0 : node.x,
    top: isRoot ? 0 : node.y,
    width: node.w,
    height: node.h,
    boxSizing: 'border-box',
    backgroundColor: node.bg ?? 'transparent',
    borderRadius: radiiCss(node.radii),
    borderTop: borderCss(node.borders[0]),
    borderInlineEnd: borderCss(node.borders[1]),
    borderBottom: borderCss(node.borders[2]),
    borderInlineStart: borderCss(node.borders[3]),
    opacity: node.opacity,
    backdropFilter:
      node.backdropBlurPx > 0
        ? `blur(${node.backdropBlurPx}px) saturate(1.4)`
        : undefined,
    WebkitBackdropFilter:
      node.backdropBlurPx > 0
        ? `blur(${node.backdropBlurPx}px) saturate(1.4)`
        : undefined,
    overflow: node.scrollable
      ? undefined
      : node.overflowHidden
        ? 'hidden'
        : 'visible',
    pointerEvents: node.pointerNone ? 'none' : undefined,
    fontFamily:
      'Inter, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
  };

  if (node.isInput) {
    const inputFontRem = Math.max(1, (node.inputFontSize || 16) / 16);
    return (
      <input
        type="text"
        className="nsh-input"
        value={draft}
        placeholder={node.inputPlaceholder}
        aria-label={node.a11yLabel ?? node.inputPlaceholder ?? 'Text input'}
        data-testid={node.testId ?? undefined}
        onChange={(e) => {
          const val = e.target.value;
          setDraft(val);
          onInput(node.id, val);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            onSubmit(node.id, draft);
          }
        }}
        style={{
          ...baseStyle,
          color: node.inputColor ?? 'var(--host-fg)',
          fontSize: `${inputFontRem}rem`,
          paddingInlineStart: Math.max(10, node.contentX),
          paddingInlineEnd: 10,
        }}
      />
    );
  }

  if (node.kind === 'image' && node.imageUri) {
    return (
      <img
        src={node.imageUri}
        alt={node.a11yLabel ?? ''}
        data-testid={node.testId ?? undefined}
        style={{ ...baseStyle, objectFit: 'cover', display: 'block' }}
      />
    );
  }

  if (node.kind === 'text') {
    const firstSpan = node.spans[0];
    const baseFontPx = firstSpan?.fontSize || 14;
    const computedLineHeight = firstSpan?.lineHeight
      ? Number(Math.max(1.3, firstSpan.lineHeight / baseFontPx).toFixed(3))
      : 1.4;
    const rawText = (node.text ?? '').trim();
    const isShortControlLabel =
      insideButton &&
      node.maxLines === 1 &&
      rawText.length > 0 &&
      rawText.length <= 16 &&
      node.w > 0 &&
      node.w <= 112 &&
      !rawText.includes('•');
    const clampStyle: React.CSSProperties =
      node.maxLines === 1
        ? isShortControlLabel
          ? {
              whiteSpace: 'nowrap',
              overflow: 'visible',
            }
          : {
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }
        : node.maxLines > 1
          ? {
              display: '-webkit-box',
              WebkitLineClamp: node.maxLines,
              WebkitBoxOrient: 'vertical',
              overflow: 'hidden',
            }
          : {
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
            };

    return (
      <div
        data-testid={node.testId ?? undefined}
        style={{
          ...baseStyle,
          display: node.maxLines > 1 ? '-webkit-box' : 'flex',
          alignItems: node.maxLines > 1 ? undefined : 'center',
          justifyContent:
            node.textAlign === 'center'
              ? 'center'
              : node.textAlign === 'right'
                ? 'flex-end'
                : 'flex-start',
          textAlign: logicalTextAlign(node.textAlign),
          lineHeight: computedLineHeight,
          ...clampStyle,
        }}
      >
        {node.spans.length > 0 ? (
          <span
            style={{
              width: isShortControlLabel ? 'auto' : '100%',
              overflow:
                node.maxLines >= 1 && !isShortControlLabel ? 'hidden' : 'visible',
              textOverflow:
                node.maxLines === 1 && !isShortControlLabel ? 'ellipsis' : undefined,
              whiteSpace: node.maxLines === 1 ? 'nowrap' : undefined,
            }}
          >
            {node.spans.map((s, i) => (
              <span
                key={i}
                style={{
                  color: s.color ?? 'inherit',
                  fontSize: `${(s.fontSize || 14) / 16}rem`,
                  fontWeight: s.fontWeight || 400,
                  fontStyle: s.italic ? 'italic' : 'normal',
                  fontVariantNumeric: s.tabularNums ? 'tabular-nums' : undefined,
                }}
              >
                {s.text}
              </span>
            ))}
          </span>
        ) : (
          <span>{node.text}</span>
        )}
      </div>
    );
  }

  const renderChildren = () =>
    node.children.map((child) => (
      <NshNodeView
        key={child.id}
        node={child}
        insideButton={insideButton || node.pressable}
        onPress={onPress}
        onInput={onInput}
        onSubmit={onSubmit}
        onScroll={onScroll}
      />
    ));

  if (node.scrollable) {
    return (
      <div
        data-testid={node.testId ?? undefined}
        className="nsh-scroll"
        onScroll={(e) => {
          const el = e.currentTarget;
          const sx = el.scrollLeft;
          const sy = el.scrollTop;
          if (scrollTimer.current !== null) {
            window.clearTimeout(scrollTimer.current);
          }
          scrollTimer.current = window.setTimeout(() => {
            onScroll(node.id, sx, sy);
          }, 90);
        }}
        style={{
          ...baseStyle,
          overflowX: node.scrollHorizontal ? 'auto' : 'hidden',
          overflowY: node.scrollHorizontal ? 'hidden' : 'auto',
          overscrollBehavior: 'contain',
          WebkitOverflowScrolling: 'touch',
        }}
      >
        <div
          style={{
            position: 'relative',
            width: Math.max(node.w, node.scrollContentW),
            height: Math.max(node.h, node.scrollContentH),
          }}
        >
          {renderChildren()}
        </div>
      </div>
    );
  }

  if (node.pressable && !insideButton) {
    const label = node.a11yLabel || collectText(node) || `Action ${node.id}`;
    return (
      <button
        type="button"
        disabled={node.disabled ?? false}
        aria-label={label}
        data-testid={node.testId ?? undefined}
        onClick={(e) => {
          e.stopPropagation();
          onPress(node.id);
        }}
        style={{
          ...baseStyle,
          padding: 0,
          margin: 0,
          cursor: node.disabled ? 'not-allowed' : 'pointer',
          textAlign: 'start',
          appearance: 'none',
          transitionProperty: 'opacity',
          transitionDuration: '120ms',
          transitionTimingFunction: 'ease',
        }}
        className="nsh-pressable host-transition hover:brightness-110 active:opacity-80"
      >
        {renderChildren()}
      </button>
    );
  }

  const isStatusRegion = node.testId === 'global-status-region';
  const livePoliteness =
    isStatusRegion
      ? 'polite'
      : node.a11yLive === 'polite' || node.a11yLive === 'assertive'
        ? node.a11yLive
        : undefined;

  return (
    <div
      data-testid={node.testId ?? undefined}
      role={
        node.pressable
          ? 'button'
          : isStatusRegion
            ? 'status'
            : (node.a11yRole ?? undefined)
      }
      tabIndex={node.pressable ? 0 : undefined}
      aria-label={
        node.pressable
          ? (node.a11yLabel || collectText(node) || `Action ${node.id}`)
          : (node.a11yLabel ?? undefined)
      }
      aria-live={livePoliteness}
      aria-atomic={isStatusRegion ? true : undefined}
      className={node.pressable ? 'nsh-pressable' : undefined}
      onClick={
        node.pressable
          ? (e) => {
              e.stopPropagation();
              onPress(node.id);
            }
          : undefined
      }
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
      style={{
        ...baseStyle,
        cursor: node.pressable ? 'pointer' : undefined,
      }}
    >
      {renderChildren()}
    </div>
  );
}

const DEVICE_PRESETS: Array<{
  id: PreviewDevice;
  label: string;
  isDesktop: boolean;
}> = [
  { id: 'compact', label: 'Compact 320×640', isDesktop: false },
  { id: 'android', label: 'Android 390×844', isDesktop: false },
  { id: 'desktop', label: 'Desktop', isDesktop: true },
];

export default function App() {
  const [preview, dispatch] = useReducer(previewReducer, INITIAL_PREVIEW_STATE);
  const reqSeqRef = useRef(0);

  const fetchTree = useCallback(async () => {
    const seq = ++reqSeqRef.current;
    dispatch({ type: 'request_started' });
    try {
      const res = await fetch('/api/nsh/tree');
      if (!res.ok) {
        const msg = await res.text();
        dispatch({
          type: 'request_failed',
          seq,
          message: formatHostError(res.status, msg),
        });
        return;
      }
      const data: NshTreeResponse = await res.json();
      dispatch({ type: 'tree_received', seq, tree: data });
    } catch (e) {
      dispatch({
        type: 'request_failed',
        seq,
        message: formatHostError(0, e instanceof Error ? e.message : String(e)),
      });
    }
  }, []);

  useEffect(() => {
    void fetchTree();
  }, [fetchTree]);

  useEffect(() => {
    if (preview.status !== 'error') return;
    const timer = window.setTimeout(() => {
      void fetchTree();
    }, 2000);
    return () => window.clearTimeout(timer);
  }, [preview.status, preview.error, fetchTree]);

  const sendAction = useCallback(
    async (endpoint: string, payload: Record<string, unknown>) => {
      const seq = ++reqSeqRef.current;
      dispatch({ type: 'request_started' });
      try {
        const res = await fetch(endpoint, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(payload),
        });
        if (res.ok) {
          const data: NshTreeResponse = await res.json();
          dispatch({ type: 'tree_received', seq, tree: data });
        } else {
          dispatch({ type: 'request_settled' });
        }
      } catch {
        dispatch({ type: 'request_settled' });
      }
    },
    [],
  );

  const tree = preview.tree;
  const error = preview.error;
  const busy = preview.inFlightCount > 0;
  const device: PreviewDevice = tree?.device ?? 'android';
  const isPhone = device === 'android' || device === 'compact';
  const scheme = tree?.scheme ?? 'dark';
  const apkMb = tree?.apkSizeBytes
    ? (tree.apkSizeBytes / (1024 * 1024)).toFixed(2)
    : null;
  const nextThemeLabel = scheme === 'dark' ? 'Light theme' : 'Dark theme';

  return (
    <div className="min-h-screen w-full bg-[var(--host-bg)] text-[var(--host-fg)] flex flex-col">
      {/* Slim Pure-Rust NSH Host Control Bar */}
      <header className="w-full border-b border-[var(--host-border)] bg-[var(--host-surface)] px-4 py-2.5 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3 min-w-0">
          <svg
            viewBox="0 0 48 48"
            className="w-7 h-7 shrink-0"
            aria-hidden="true"
            focusable="false"
          >
            <defs>
              <linearGradient id="raag-grad" x1="0%" y1="0%" x2="100%" y2="100%">
                <stop offset="0%" stopColor="#a8490b" />
                <stop offset="100%" stopColor="#eb7a3c" />
              </linearGradient>
            </defs>
            <rect x="2" y="2" width="44" height="44" rx="13" fill="url(#raag-grad)" />
            <path
              d="M13 30c0-6.1 4.9-11 11-11s11 4.9 11 11"
              fill="none"
              stroke="#FFFFFF"
              strokeWidth="2.6"
              strokeLinecap="round"
              opacity="0.45"
            />
            <path
              d="M18 30c0-3.3 2.7-6 6-6s6 2.7 6 6"
              fill="none"
              stroke="#FFFFFF"
              strokeWidth="2.8"
              strokeLinecap="round"
              opacity="0.8"
            />
            <rect x="22" y="13" width="4" height="19" rx="2" fill="#FFFFFF" />
            <rect x="15" y="19" width="3.5" height="13" rx="1.75" fill="#FFFFFF" opacity="0.9" />
            <rect x="29.5" y="17" width="3.5" height="15" rx="1.75" fill="#FFFFFF" opacity="0.9" />
          </svg>
          <div className="min-w-0">
            <h1 className="text-sm font-semibold leading-snug text-[var(--host-fg)]">
              Raag — Pure Rust NSH Live Host
            </h1>
            <div className="text-xs text-[var(--host-muted-fg)]">
              {tree?.engine ?? 'Connecting to Rust nsh-engine 0.7.0…'}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          <div
            role="group"
            aria-label="Viewport preset"
            className="flex items-center flex-wrap rounded-lg border border-[var(--host-border)] bg-[var(--host-elevated)] p-0.5"
          >
            {DEVICE_PRESETS.map((preset) => {
              const active = device === preset.id;
              return (
                <button
                  key={preset.id}
                  type="button"
                  aria-pressed={active}
                  onClick={() => sendAction('/api/nsh/config', { device: preset.id })}
                  className={`flex items-center gap-1.5 px-2.5 py-2 rounded-md text-xs font-medium tabular-nums host-focus host-transition transition-colors duration-150 cursor-pointer ${
                    active
                      ? 'bg-[var(--host-primary)] text-[var(--host-primary-fg)] font-semibold'
                      : 'text-[var(--host-muted-fg)] hover:text-[var(--host-fg)] hover:bg-[var(--host-elevated-hover)]'
                  }`}
                >
                  {preset.isDesktop ? (
                    <Monitor className="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
                  ) : (
                    <Smartphone className="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
                  )}
                  {preset.label}
                </button>
              );
            })}
          </div>

          <button
            type="button"
            aria-label={`Switch to ${nextThemeLabel}`}
            onClick={() =>
              sendAction('/api/nsh/config', {
                scheme: scheme === 'dark' ? 'light' : 'dark',
              })
            }
            className="flex items-center gap-1.5 px-3 py-2 rounded-lg border border-[var(--host-border)] bg-[var(--host-elevated)] text-xs font-medium text-[var(--host-fg)] hover:bg-[var(--host-elevated-hover)] host-focus host-transition transition-colors duration-150 cursor-pointer"
          >
            {scheme === 'dark' ? (
              <Sun className="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
            ) : (
              <Moon className="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
            )}
            {nextThemeLabel}
          </button>

          <button
            type="button"
            aria-label="Refresh preview frame"
            onClick={() => void fetchTree()}
            className="flex items-center justify-center p-2 rounded-lg border border-[var(--host-border)] bg-[var(--host-elevated)] text-xs text-[var(--host-muted-fg)] hover:text-[var(--host-fg)] hover:bg-[var(--host-elevated-hover)] host-focus host-transition transition-colors duration-150 cursor-pointer"
          >
            <RefreshCw
              className={`w-3.5 h-3.5 ${busy ? 'animate-spin host-spin' : ''}`}
              aria-hidden="true"
            />
          </button>

          {tree?.apkAvailable && (
            <a
              href="/raag-arm64-debug.apk"
              download="raag-arm64-debug.apk"
              className="flex items-center gap-1.5 px-3 py-2 rounded-lg bg-[var(--host-primary)] hover:bg-[var(--host-primary-hover)] text-[var(--host-primary-fg)] text-xs font-semibold tabular-nums host-focus host-transition transition-colors duration-150"
            >
              <Download className="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
              Download APK {apkMb ? `(${apkMb} MB)` : '(arm64-v8a)'}
            </a>
          )}
        </div>
      </header>

      {/* Live NSH Native View Surface */}
      <main className="flex-1 flex items-center justify-center p-4 md:p-6 overflow-auto">
        {error ? (
          <div
            role="status"
            aria-live="polite"
            className="max-w-md w-full rounded-xl border border-[var(--host-border-strong)] bg-[var(--host-elevated)] p-5 text-center space-y-3"
          >
            <h2 className="text-sm font-semibold text-[var(--host-fg)]">
              Waiting for Pure Rust NSH Engine
            </h2>
            <p className="text-xs leading-relaxed text-[var(--host-muted-fg)] break-words">
              {error}
            </p>
            <button
              type="button"
              onClick={() => void fetchTree()}
              className="px-4 py-2 rounded-lg bg-[var(--host-primary)] hover:bg-[var(--host-primary-hover)] text-[var(--host-primary-fg)] text-xs font-semibold host-focus host-transition transition-colors duration-150 cursor-pointer"
            >
              Retry Connection
            </button>
          </div>
        ) : tree?.root ? (
          <div
            data-testid="nsh-viewport-frame"
            className={
              isPhone
                ? 'rounded-[38px] p-2.5 bg-[var(--host-elevated)] border border-[var(--host-border-strong)] shadow-[0_24px_80px_rgba(0,0,0,0.65)]'
                : 'rounded-2xl p-1.5 bg-[var(--host-elevated)] border border-[var(--host-border-strong)] shadow-[0_24px_80px_rgba(0,0,0,0.65)]'
            }
          >
            <div
              style={{
                width: tree.width,
                height: tree.height,
                position: 'relative',
                overflow: 'hidden',
                borderRadius: isPhone ? '27px' : '9px',
                backgroundColor:
                  scheme === 'dark'
                    ? 'var(--host-viewport-dark)'
                    : 'var(--host-viewport-light)',
              }}
            >
              {/* Decorative Android status bar safe-area indicator */}
              {isPhone && (
                <div
                  aria-hidden="true"
                  className="pointer-events-none absolute top-0 left-0 right-0 h-6 z-20 flex items-center justify-between px-5 text-xs font-semibold tabular-nums"
                  style={{
                    color:
                      scheme === 'dark'
                        ? 'var(--host-status-dark)'
                        : 'var(--host-status-light)',
                  }}
                >
                  <span>9:41</span>
                  <div className="w-16 h-3.5 rounded-full bg-black/50" />
                  <span>5G 100%</span>
                </div>
              )}

              <NshNodeView
                node={tree.root}
                isRoot
                onPress={(id) => void sendAction('/api/nsh/press', { id })}
                onInput={(id, text) =>
                  void sendAction('/api/nsh/input', { id, text })
                }
                onSubmit={(id, text) =>
                  void sendAction('/api/nsh/submit', { id, text })
                }
                onScroll={(id, x, y) =>
                  void sendAction('/api/nsh/scroll', { id, x, y })
                }
              />

              {/* Decorative Android gesture pill indicator in bottom safe-area */}
              {isPhone && (
                <div
                  aria-hidden="true"
                  className="pointer-events-none absolute bottom-1.5 left-0 right-0 z-20 flex justify-center"
                >
                  <div
                    className="w-28 h-1 rounded-full"
                    style={{
                      backgroundColor:
                        scheme === 'dark'
                          ? 'var(--host-pill-dark)'
                          : 'var(--host-pill-light)',
                    }}
                  />
                </div>
              )}
            </div>
          </div>
        ) : (
          <div
            role="status"
            aria-live="polite"
            className="text-xs text-[var(--host-muted-fg)]"
          >
            Booting Pure Rust NSH Engine…
          </div>
        )}
      </main>
    </div>
  );
}
