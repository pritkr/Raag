/**
 * @license
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

export interface NshBorderSide {
  width: number;
  color: string | null;
}

export interface NshSpan {
  text: string;
  color: string | null;
  fontSize: number;
  fontWeight: number;
  italic: boolean;
  tabularNums: boolean;
  lineHeight: number;
}

export interface NshNode {
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
  a11yRole?: string | null;
  a11yLive?: string | null;
  disabled?: boolean;
  pointerNone: boolean;
  children: NshNode[];
}

export type PreviewDevice = 'compact' | 'android' | 'desktop';

export interface NshTreeResponse {
  version: number;
  engine: string;
  device: PreviewDevice;
  scheme: 'dark' | 'light';
  width: number;
  height: number;
  apkAvailable: boolean;
  apkSizeBytes: number;
  root: NshNode | null;
}

/**
 * Formats HTTP or network errors from the NSH preview proxy into clean,
 * recoverable human-readable copy without exposing raw JSON or HTML payloads.
 */
export function formatHostError(status: number, rawBody: string): string {
  const trimmed = rawBody.trim();
  if (trimmed.startsWith('{')) {
    try {
      const parsed = JSON.parse(trimmed) as { error?: string; detail?: string };
      if (status === 503) {
        return 'The Pure Rust NSH preview engine is still starting up. Wait a moment or select Retry Connection.';
      }
      if (parsed.error && typeof parsed.error === 'string') {
        const clean = parsed.error.trim().replace(/\.+$/, '');
        return `${clean}. Select Retry Connection to try again.`;
      }
    } catch {
      // Fall through to status-based messaging
    }
  }
  if (status === 503) {
    return 'The Pure Rust NSH preview engine is still starting up. Wait a moment or select Retry Connection.';
  }
  if (status >= 400) {
    return `Preview engine returned HTTP ${status}. Check that nsh_preview is running and select Retry Connection.`;
  }
  return 'Could not reach the Pure Rust NSH preview engine. Wait a moment or select Retry Connection.';
}

/**
 * Explicit finite state machine for the Pure-Rust NSH web preview host.
 * Replaces the implicit `(tree, error, busy)` product state and adds monotonic
 * request sequence fencing so out-of-order HTTP responses never clobber newer UI state.
 */
export type PreviewState =
  | {
      status: 'booting';
      tree: null;
      error: null;
      inFlightCount: number;
      lastAppliedSeq: number;
    }
  | {
      status: 'ready';
      tree: NshTreeResponse;
      error: null;
      inFlightCount: number;
      lastAppliedSeq: number;
    }
  | {
      status: 'error';
      tree: null;
      error: string;
      inFlightCount: number;
      lastAppliedSeq: number;
    };

export type PreviewAction =
  | { type: 'request_started' }
  | { type: 'tree_received'; seq: number; tree: NshTreeResponse }
  | { type: 'request_failed'; seq: number; message: string }
  | { type: 'request_settled' };

export const INITIAL_PREVIEW_STATE: PreviewState = {
  status: 'booting',
  tree: null,
  error: null,
  inFlightCount: 0,
  lastAppliedSeq: 0,
};

export function previewReducer(
  state: PreviewState,
  action: PreviewAction,
): PreviewState {
  switch (action.type) {
    case 'request_started':
      return {
        ...state,
        inFlightCount: state.inFlightCount + 1,
      };

    case 'tree_received': {
      const nextInFlight = Math.max(0, state.inFlightCount - 1);
      const isNewer =
        state.tree === null ||
        action.tree.version > state.tree.version ||
        (action.tree.version === state.tree.version && action.seq >= state.lastAppliedSeq);
      if (!isNewer) {
        return {
          ...state,
          inFlightCount: nextInFlight,
        };
      }
      return {
        status: 'ready',
        tree: action.tree,
        error: null,
        inFlightCount: nextInFlight,
        lastAppliedSeq: Math.max(state.lastAppliedSeq, action.seq),
      };
    }

    case 'request_failed': {
      const nextInFlight = Math.max(0, state.inFlightCount - 1);
      if (action.seq < state.lastAppliedSeq) {
        return {
          ...state,
          inFlightCount: nextInFlight,
        };
      }
      return {
        status: 'error',
        tree: null,
        error: action.message,
        inFlightCount: nextInFlight,
        lastAppliedSeq: action.seq,
      };
    }

    case 'request_settled':
      return {
        ...state,
        inFlightCount: Math.max(0, state.inFlightCount - 1),
      };
  }
}
