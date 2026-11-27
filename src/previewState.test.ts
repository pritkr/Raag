/**
 * @license
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  formatHostError,
  INITIAL_PREVIEW_STATE,
  previewReducer,
  type NshTreeResponse,
} from './previewState';

function makeTree(version: number): NshTreeResponse {
  return {
    version,
    engine: 'nsh-0.7.0',
    device: 'android',
    scheme: 'dark',
    width: 390,
    height: 844,
    apkAvailable: false,
    apkSizeBytes: 0,
    root: null,
  };
}

test('formatHostError converts 503 JSON payload into friendly recoverable copy', () => {
  const raw = JSON.stringify({
    error: 'Pure Rust NSH Preview Engine is starting up',
    detail: 'fetch failed',
  });
  const msg = formatHostError(503, raw);
  assert.ok(msg.includes('starting up'), `expected startup guidance, got: ${msg}`);
  assert.ok(msg.includes('Retry Connection'), `expected retry CTA reference, got: ${msg}`);
  assert.ok(!msg.includes('{'), `must not leak raw JSON braces: ${msg}`);
  assert.ok(!msg.includes('}'), `must not leak raw JSON braces: ${msg}`);
  assert.ok(!msg.includes('fetch failed'), `must not leak raw fetch error: ${msg}`);
});

test('formatHostError extracts clean error message from non-503 JSON payloads', () => {
  const msg = formatHostError(500, '{"error":"Engine layout pass crashed..."}');
  assert.equal(msg, 'Engine layout pass crashed. Select Retry Connection to try again.');
});

test('formatHostError strips raw HTML error bodies to clean HTTP status copy', () => {
  const msg = formatHostError(502, '<html><body><h1>502 Bad Gateway</h1></body></html>');
  assert.ok(msg.includes('HTTP 502'));
  assert.ok(msg.includes('Retry Connection'));
  assert.ok(!msg.includes('<html'));
});

test('formatHostError handles network unreachable status 0 cleanly', () => {
  const msg = formatHostError(0, 'TypeError: Failed to fetch');
  assert.ok(msg.includes('Could not reach'));
  assert.ok(msg.includes('Retry Connection'));
  assert.ok(!msg.includes('TypeError'));
});

test('previewReducer fences out-of-order tree responses and stale failures', () => {
  let state = INITIAL_PREVIEW_STATE;
  state = previewReducer(state, { type: 'request_started' });
  assert.equal(state.inFlightCount, 1);

  // Newer response arrives first (seq 2, version 10)
  state = previewReducer(state, {
    type: 'tree_received',
    seq: 2,
    tree: makeTree(10),
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.tree?.version, 10);
  assert.equal(state.lastAppliedSeq, 2);

  // Stale older response arrives late (seq 1, version 8) -> ignored
  state = previewReducer(state, { type: 'request_started' });
  state = previewReducer(state, {
    type: 'tree_received',
    seq: 1,
    tree: makeTree(8),
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.tree?.version, 10);

  // Stale request failure (seq 1 < lastAppliedSeq 2) -> ignored
  state = previewReducer(state, { type: 'request_started' });
  state = previewReducer(state, {
    type: 'request_failed',
    seq: 1,
    message: 'stale error',
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.error, null);

  // Newer failure (seq 3 >= lastAppliedSeq 2) -> transitions to error
  state = previewReducer(state, { type: 'request_started' });
  state = previewReducer(state, {
    type: 'request_failed',
    seq: 3,
    message: 'Engine unreachable',
  });
  assert.equal(state.status, 'error');
  assert.equal(state.error, 'Engine unreachable');
  assert.equal(state.lastAppliedSeq, 3);
});
