/**
 * @license
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import React, { useCallback, useEffect, useRef, useState } from 'react';
import { Download, Monitor, Moon, RefreshCw, Smartphone, Sun } from 'lucide-react';

interface NshBorderSide {
  width: number;
  color: string | null;
}

interface NshSpan {
  text: string;
  color: string | null;
  fontSize: number;
  fontWeight: number;
  italic: boolean;
  tabularNums: boolean;
  lineHeight: number;
}

interface NshNode {
  id: number;
  kind: 'view' | 'text' | 'scroll' | 'input' | 'image';
  x: number;
  y: number;
  w: number;
  h: number;
  bg: string | null;
  radii: [number, number, number, number];
  borders: [NshBorderSide, NshBorderSide, NshBorderSide, NshBorderSide];
  opacity: number;
  backdropBlurPx: number;
  overflowHidden: boolean;
  text: string | null;
  spans: NshSpan[];
  textAlign: 'left' | 'center' | 'right' | 'justify';
  maxLines: number;
  contentX: number;
  contentY: number;
  contentW: number;
  contentH: number;
  imageUri: string | null;
  pressable: boolean;
  scrollable: boolean;
  scrollHorizontal: boolean;
  scrollContentW: number;
  scrollContentH: number;
  scrollX: number;
  scrollY: number;
  isInput: boolean;
  inputValue: string;
  inputPlaceholder: string;
  inputColor: string | null;
  inputPlaceholderColor: string | null;
  inputFontSize: number;
  testId: string | null;
  a11yLabel: string | null;
  pointerNone: boolean;
  children: NshNode[];
}

interface NshTreeResponse {
  version: number;
  engine: string;
  device: 'android' | 'desktop';
  scheme: 'dark' | 'light';
  width: number;
  height: number;
  apkAvailable: boolean;
  apkSizeBytes: number;
  root: NshNode | null;
}

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

  const baseStyle: React.CSSProperties = {
    position: isRoot ? 'relative' : 'absolute',
    left: isRoot ? 0 : node.x,
    top: isRoot ? 0 : node.y,
    width: node.w,
    height: node.h,
    boxSizing: 'border-box',
    backgroundColor: node.bg ?? 'transparent',
    borderRadius: radiiCss(node.radii),
    borderTop: borderCss(node.borders[0]),
    borderRight: borderCss(node.borders[1]),
    borderBottom: borderCss(node.borders[2]),
    borderLeft: borderCss(node.borders[3]),
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
    return (
      <input
        type="text"
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
          color: node.inputColor ?? '#f4f5f7',
          fontSize: `${node.inputFontSize || 14}px`,
          paddingLeft: Math.max(10, node.contentX),
          paddingRight: 10,
          outline: 'none',
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
    const clampStyle: React.CSSProperties =
      node.maxLines === 1
        ? {
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
          textAlign: node.textAlign,
          lineHeight: firstSpan?.lineHeight ? `${firstSpan.lineHeight}px` : 1.25,
          ...clampStyle,
        }}
      >
        {node.spans.length > 0 ? (
          <span
            style={{
              width: '100%',
              overflow: node.maxLines >= 1 ? 'hidden' : undefined,
              textOverflow: node.maxLines === 1 ? 'ellipsis' : undefined,
              whiteSpace: node.maxLines === 1 ? 'nowrap' : undefined,
            }}
          >
            {node.spans.map((s, i) => (
              <span
                key={i}
                style={{
                  color: s.color ?? 'inherit',
                  fontSize: `${s.fontSize || 14}px`,
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
          cursor: 'pointer',
          textAlign: 'left',
          appearance: 'none',
          transition: 'opacity 110ms ease',
        }}
        className="hover:brightness-110 active:opacity-80 focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-red-500"
      >
        {renderChildren()}
      </button>
    );
  }

  return (
    <div
      data-testid={node.testId ?? undefined}
      aria-label={node.a11yLabel ?? undefined}
      onClick={
        node.pressable
          ? (e) => {
              e.stopPropagation();
              onPress(node.id);
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

export default function App() {
  const [tree, setTree] = useState<NshTreeResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const fetchTree = useCallback(async () => {
    try {
      const res = await fetch('/api/nsh/tree');
      if (!res.ok) {
        const msg = await res.text();
        throw new Error(msg || `HTTP ${res.status}`);
      }
      const data: NshTreeResponse = await res.json();
      setTree(data);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    void fetchTree();
  }, [fetchTree]);

  const sendAction = useCallback(
    async (endpoint: string, payload: Record<string, unknown>) => {
      setBusy(true);
      try {
        const res = await fetch(endpoint, {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(payload),
        });
        if (res.ok) {
          const data: NshTreeResponse = await res.json();
          setTree(data);
          setError(null);
        }
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  const device = tree?.device ?? 'android';
  const scheme = tree?.scheme ?? 'dark';
  const apkMb = tree?.apkSizeBytes
    ? (tree.apkSizeBytes / (1024 * 1024)).toFixed(2)
    : null;

  return (
    <div className="min-h-screen w-full bg-[#07080a] text-[#f4f5f7] flex flex-col">
      {/* Slim Pure-Rust NSH Host Control Bar */}
      <header className="w-full border-b border-[#222631] bg-[#0d0f14] px-4 py-2.5 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <svg
            viewBox="0 0 48 48"
            className="w-7 h-7 shrink-0"
            aria-label="YouTube Music Icon"
          >
            <circle cx="24" cy="24" r="22" fill="#FF0033" />
            <circle
              cx="24"
              cy="24"
              r="13.5"
              fill="none"
              stroke="#FFFFFF"
              strokeWidth="2.2"
            />
            <polygon points="20,17 32,24 20,31" fill="#FFFFFF" />
          </svg>
          <div>
            <div className="text-sm font-semibold leading-tight">
              Raag — Pure Rust NSH Live Host
            </div>
            <div className="text-[11px] text-[#9fa6b2]">
              {tree?.engine ?? 'Connecting to Rust nsh-engine 0.6.1...'}
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2 flex-wrap">
          <div className="flex items-center rounded-lg border border-[#242834] bg-[#13161e] p-0.5">
            <button
              type="button"
              onClick={() => sendAction('/api/nsh/config', { device: 'android' })}
              className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs font-medium transition cursor-pointer ${
                device === 'android'
                  ? 'bg-[#d92638] text-white'
                  : 'text-[#9fa6b2] hover:text-white'
              }`}
            >
              <Smartphone className="w-3.5 h-3.5" />
              Android 390×844
            </button>
            <button
              type="button"
              onClick={() => sendAction('/api/nsh/config', { device: 'desktop' })}
              className={`flex items-center gap-1.5 px-2.5 py-1.5 rounded-md text-xs font-medium transition cursor-pointer ${
                device === 'desktop'
                  ? 'bg-[#d92638] text-white'
                  : 'text-[#9fa6b2] hover:text-white'
              }`}
            >
              <Monitor className="w-3.5 h-3.5" />
              Desktop
            </button>
          </div>

          <button
            type="button"
            aria-label="Toggle Color Scheme"
            onClick={() =>
              sendAction('/api/nsh/config', {
                scheme: scheme === 'dark' ? 'light' : 'dark',
              })
            }
            className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg border border-[#242834] bg-[#13161e] text-xs font-medium text-[#e4e7ee] hover:bg-[#1b202b] transition cursor-pointer"
          >
            {scheme === 'dark' ? (
              <>
                <Sun className="w-3.5 h-3.5" />
                Light
              </>
            ) : (
              <>
                <Moon className="w-3.5 h-3.5" />
                Dark
              </>
            )}
          </button>

          <button
            type="button"
            aria-label="Refresh NSH Frame"
            onClick={() => void fetchTree()}
            className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg border border-[#242834] bg-[#13161e] text-xs text-[#9fa6b2] hover:text-white transition cursor-pointer"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${busy ? 'animate-spin' : ''}`} />
          </button>

          {tree?.apkAvailable && (
            <a
              href="/resonance-arm64-debug.apk"
              download="resonance-arm64-debug.apk"
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-[#d92638] hover:bg-[#e13244] text-white text-xs font-semibold transition"
            >
              <Download className="w-3.5 h-3.5" />
              Download APK {apkMb ? `(${apkMb} MB)` : '(arm64-v8a)'}
            </a>
          )}
        </div>
      </header>

      {/* Live NSH Native View Surface */}
      <main className="flex-1 flex items-center justify-center p-4 md:p-6 overflow-auto">
        {error ? (
          <div className="max-w-md rounded-xl border border-[#2b3040] bg-[#13161e] p-5 text-center space-y-3">
            <div className="text-sm font-semibold text-white">
              Waiting for Pure Rust NSH Engine
            </div>
            <p className="text-xs text-[#9fa6b2] break-words">{error}</p>
            <button
              type="button"
              onClick={() => void fetchTree()}
              className="px-4 py-2 rounded-lg bg-[#d92638] text-white text-xs font-semibold cursor-pointer"
            >
              Retry Connection
            </button>
          </div>
        ) : tree?.root ? (
          <div
            data-testid="nsh-viewport-frame"
            className={
              device === 'android'
                ? 'rounded-[38px] p-2.5 bg-[#171a22] border border-[#2c3140] shadow-[0_24px_80px_rgba(0,0,0,0.65)]'
                : 'rounded-2xl p-1.5 bg-[#171a22] border border-[#2c3140] shadow-[0_24px_80px_rgba(0,0,0,0.65)]'
            }
          >
            <div
              style={{
                width: tree.width,
                height: tree.height,
                position: 'relative',
                overflow: 'hidden',
                borderRadius: device === 'android' ? '28px' : '12px',
                backgroundColor: scheme === 'dark' ? '#0a0b0d' : '#f7f8fa',
              }}
            >
              {/* Android status bar safe-area indicator */}
              {device === 'android' && (
                <div
                  className="pointer-events-none absolute top-0 left-0 right-0 h-6 z-20 flex items-center justify-between px-5 text-[10px] font-semibold"
                  style={{
                    color: scheme === 'dark' ? '#9fa6b2' : '#586070',
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

              {/* Android gesture pill indicator in bottom safe-area */}
              {device === 'android' && (
                <div className="pointer-events-none absolute bottom-1.5 left-0 right-0 z-20 flex justify-center">
                  <div
                    className="w-28 h-1 rounded-full"
                    style={{
                      backgroundColor:
                        scheme === 'dark'
                          ? 'rgba(255,255,255,0.28)'
                          : 'rgba(0,0,0,0.25)',
                    }}
                  />
                </div>
              )}
            </div>
          </div>
        ) : (
          <div className="text-xs text-[#9fa6b2]">
            Booting Pure Rust NSH Engine...
          </div>
        )}
      </main>
    </div>
  );
}
