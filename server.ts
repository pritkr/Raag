/**
 * @license
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

import { spawn, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import express from 'express';
import { createServer as createViteServer } from 'vite';

const PORT = Number(process.env.PORT ?? 3000);
const NSH_PORT = Number(process.env.NSH_PREVIEW_PORT ?? 3001);
const NSH_ORIGIN = `http://127.0.0.1:${NSH_PORT}`;

let nshProc: ChildProcess | null = null;

function findNshPreviewBinary(): string | null {
  const candidates = [
    path.resolve('target/release/nsh_preview'),
    path.resolve('target/debug/nsh_preview'),
  ];
  for (const bin of candidates) {
    if (fs.existsSync(bin)) {
      return bin;
    }
  }
  return null;
}

async function isNshReady(): Promise<boolean> {
  try {
    const res = await fetch(`${NSH_ORIGIN}/api/nsh/tree`);
    return res.ok;
  } catch {
    return false;
  }
}

async function ensureNshPreviewRunning(): Promise<void> {
  if (await isNshReady()) {
    return;
  }
  const bin = findNshPreviewBinary();
  if (!bin) {
    console.warn('[server] Pure Rust nsh_preview binary not built yet; waiting for build.');
    return;
  }
  nshProc = spawn(bin, [], {
    env: { ...process.env, NSH_PREVIEW_PORT: String(NSH_PORT) },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  nshProc.stdout?.on('data', (d) => process.stdout.write(`[nsh_preview] ${d}`));
  nshProc.stderr?.on('data', (d) => process.stderr.write(`[nsh_preview] ${d}`));
  nshProc.on('exit', (code) => {
    console.log(`[server] nsh_preview exited with code ${code}`);
    nshProc = null;
  });

  for (let i = 0; i < 40; i++) {
    if (await isNshReady()) {
      return;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
}

async function main(): Promise<void> {
  await ensureNshPreviewRunning();

  const app = express();
  app.use(express.json({ limit: '256kb' }));

  // Proxy live Pure Rust NSH Engine MountBatch & Event API
  app.all('/api/nsh/*', async (req, res) => {
    try {
      if (!nshProc && !(await isNshReady())) {
        await ensureNshPreviewRunning();
      }
      const targetUrl = `${NSH_ORIGIN}${req.originalUrl}`;
      const init: RequestInit = {
        method: req.method,
        headers: { 'content-type': 'application/json' },
      };
      if (req.method !== 'GET' && req.method !== 'HEAD') {
        init.body = JSON.stringify(req.body ?? {});
      }
      const upstream = await fetch(targetUrl, init);
      const text = await upstream.text();
      res
        .status(upstream.status)
        .type(upstream.headers.get('content-type') ?? 'application/json')
        .send(text);
    } catch (err) {
      res.status(503).json({
        error: 'Pure Rust NSH Preview Engine is starting up',
        detail: err instanceof Error ? err.message : String(err),
      });
    }
  });

  // Serve built Android arm64-v8a APK directly from public/
  app.use('/resonance-arm64-debug.apk', (_req, res, next) => {
    const apkPath = path.resolve('public/resonance-arm64-debug.apk');
    if (fs.existsSync(apkPath)) {
      res.setHeader('Content-Type', 'application/vnd.android.package-archive');
      res.setHeader(
        'Content-Disposition',
        'attachment; filename="resonance-arm64-debug.apk"',
      );
      return res.sendFile(apkPath);
    }
    next();
  });

  if (process.env.NODE_ENV !== 'production') {
    const vite = await createViteServer({
      server: { middlewareMode: true },
      appType: 'spa',
    });
    app.use(vite.middlewares);
  } else {
    app.use(express.static(path.resolve('dist')));
    app.get('*', (_req, res) => {
      res.sendFile(path.resolve('dist/index.html'));
    });
  }

  app.listen(PORT, '0.0.0.0', () => {
    console.log(`Resonance NSH Host listening on http://0.0.0.0:${PORT}`);
  });
}

process.on('SIGTERM', () => {
  nshProc?.kill('SIGTERM');
  process.exit(0);
});
process.on('SIGINT', () => {
  nshProc?.kill('SIGINT');
  process.exit(0);
});

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
