import test from 'node:test';
import assert from 'node:assert/strict';
import {
  INITIAL_PREVIEW_STATE,
  previewReducer,
  type NshTreeResponse,
} from './previewState';

function makeTree(version: number, device: 'android' | 'desktop' = 'android'): NshTreeResponse {
  return {
    version,
    engine: 'nsh-engine 0.7.0',
    device,
    scheme: 'dark',
    width: device === 'android' ? 390 : 1280,
    height: device === 'android' ? 844 : 800,
    apkAvailable: false,
    apkSizeBytes: 0,
    root: null,
  };
}

test('boots cleanly from booting -> ready and tracks inFlightCount', () => {
  let state = previewReducer(INITIAL_PREVIEW_STATE, { type: 'request_started' });
  assert.equal(state.status, 'booting');
  assert.equal(state.inFlightCount, 1);

  const tree = makeTree(1);
  state = previewReducer(state, { type: 'tree_received', seq: 1, tree });
  assert.equal(state.status, 'ready');
  assert.equal(state.tree?.version, 1);
  assert.equal(state.inFlightCount, 0);
  assert.equal(state.lastAppliedSeq, 1);
});

test('ignores out-of-order stale responses so newer UI state is never clobbered', () => {
  let state = previewReducer(INITIAL_PREVIEW_STATE, { type: 'request_started' });
  state = previewReducer(state, { type: 'request_started' });
  assert.equal(state.inFlightCount, 2);

  // Newer response (seq 2) arrives first
  state = previewReducer(state, {
    type: 'tree_received',
    seq: 2,
    tree: makeTree(2, 'desktop'),
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.tree?.device, 'desktop');
  assert.equal(state.inFlightCount, 1);

  // Older response (seq 1) arrives late — must be ignored, while decrementing inFlightCount
  state = previewReducer(state, {
    type: 'tree_received',
    seq: 1,
    tree: makeTree(1, 'android'),
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.tree?.device, 'desktop');
  assert.equal(state.tree?.version, 2);
  assert.equal(state.inFlightCount, 0);
});

test('stale request failure does not overwrite newer ready tree', () => {
  let state = previewReducer(INITIAL_PREVIEW_STATE, { type: 'request_started' });
  state = previewReducer(state, { type: 'request_started' });

  state = previewReducer(state, {
    type: 'tree_received',
    seq: 5,
    tree: makeTree(5),
  });
  state = previewReducer(state, {
    type: 'request_failed',
    seq: 4,
    message: 'timeout',
  });

  assert.equal(state.status, 'ready');
  assert.equal(state.error, null);
  assert.equal(state.tree?.version, 5);
  assert.equal(state.inFlightCount, 0);
});

test('transitions booting -> error -> ready on reconnect retry', () => {
  let state = previewReducer(INITIAL_PREVIEW_STATE, { type: 'request_started' });
  state = previewReducer(state, {
    type: 'request_failed',
    seq: 1,
    message: 'HTTP 503',
  });
  assert.equal(state.status, 'error');
  assert.equal(state.error, 'HTTP 503');
  assert.equal(state.inFlightCount, 0);

  state = previewReducer(state, { type: 'request_started' });
  state = previewReducer(state, {
    type: 'tree_received',
    seq: 2,
    tree: makeTree(2),
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.error, null);
  assert.equal(state.tree?.version, 2);
});

test('slower mutation with higher server version beats faster stale poll with higher client seq', () => {
  let state = previewReducer(INITIAL_PREVIEW_STATE, { type: 'request_started' });
  state = previewReducer(state, { type: 'request_started' });

  // Faster poll (seq 2) returns older server scene version 1 first
  state = previewReducer(state, {
    type: 'tree_received',
    seq: 2,
    tree: makeTree(1, 'android'),
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.tree?.version, 1);
  assert.equal(state.lastAppliedSeq, 2);

  // Slower mutation (seq 1) completes after advancing server scene to version 2 — must win
  state = previewReducer(state, {
    type: 'tree_received',
    seq: 1,
    tree: makeTree(2, 'desktop'),
  });
  assert.equal(state.status, 'ready');
  assert.equal(state.tree?.version, 2);
  assert.equal(state.tree?.device, 'desktop');
  assert.equal(state.lastAppliedSeq, 2);
  assert.equal(state.inFlightCount, 0);
});
