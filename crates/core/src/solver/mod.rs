//! Updatable `yt-dlp-ejs` Solver Bundle running inside Native QuickJS-NG (`rquickjs >= 0.12.0`).
//!
//! Architecture:
//! - Executes `yt-dlp-ejs` (AST extraction via `meriyah` / SWC) rather than raw `base.js`.
//! - Shipped as a hot-updatable asset (`assets/ejs/solver_manifest.json` + `<data_dir>/ejs/`)
//!   so YouTube player churn (e.g. "found 0 sig function possibilities") is fixed by updating
//!   the solver bundle asset without recompiling the binary.
//! - Caches solved `n`-parameters in memory keyed by `(player_id, raw_n)` to eliminate
//!   repeated JS execution latency across tracks sharing the same player revision.
//! - Runs strictly on a dedicated worker thread with a 10s interrupt watchdog (NEVER blocks UI).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{RecvTimeoutError, Sender};
use resonance_contract::{CoreError, CoreResult};
use rquickjs::{Context, Function, Runtime};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;

const WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const JS_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(10);

pub const EMBEDDED_EJS_MANIFEST_JSON: &str =
    include_str!("../../../../assets/ejs/solver_manifest.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EjsBundleManifest {
    pub bundle_name: String,
    pub bundle_version: String,
    pub min_quickjs_ng_version: String,
    pub ast_parser: String,
    pub updatable_remote_url: String,
    pub entrypoint_fn: String,
    #[serde(default)]
    pub notes: String,
}

impl EjsBundleManifest {
    pub fn embedded_default() -> CoreResult<Self> {
        serde_json::from_str(EMBEDDED_EJS_MANIFEST_JSON)
            .map_err(|e| CoreError::Solver(format!("invalid solver_manifest.json: {e}")))
    }
}

#[derive(Debug, Clone)]
pub struct ChallengeInput {
    pub player_hash: String,
    pub solver_js: String,
    pub n_param: Option<String>,
    pub sig_param: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolvedChallenge {
    pub n_param: Option<String>,
    pub sig_param: Option<String>,
}

enum WorkerJob {
    Solve {
        script_path: PathBuf,
        n_param: Option<String>,
        sig_param: Option<String>,
        reply: oneshot::Sender<CoreResult<SolvedChallenge>>,
    },
}

pub struct QuickJsSolver {
    cache_dir: PathBuf,
    ejs_dir: PathBuf,
    worker: Mutex<Option<Sender<WorkerJob>>>,
    /// Per-player-ID solved `n`-param cache: `(player_id, raw_n) -> solved_n`.
    n_param_cache: Mutex<HashMap<(String, String), String>>,
    /// Counter for deterministic telemetry/QA verification of JS invocations vs cache hits.
    js_eval_count: std::sync::atomic::AtomicU64,
}

impl QuickJsSolver {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            cache_dir: data_dir.join("js_solver"),
            ejs_dir: data_dir.join("ejs"),
            worker: Mutex::new(None),
            n_param_cache: Mutex::new(HashMap::new()),
            js_eval_count: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Loads the active `yt-dlp-ejs` manifest (preferring a hot-updated manifest in `<data_dir>/ejs/`).
    pub fn active_ejs_manifest(&self) -> CoreResult<EjsBundleManifest> {
        let override_manifest = self.ejs_dir.join("solver_manifest.json");
        if override_manifest.exists()
            && let Ok(raw) = fs::read_to_string(&override_manifest)
            && let Ok(parsed) = serde_json::from_str::<EjsBundleManifest>(&raw)
        {
            return Ok(parsed);
        }
        EjsBundleManifest::embedded_default()
    }

    /// Hot-installs an updated `yt-dlp-ejs` bundle asset atomically with SHA-256 verification.
    pub fn install_ejs_bundle_update(
        &self,
        manifest: &EjsBundleManifest,
        bundle_js: &str,
        expected_sha256_hex: &str,
    ) -> CoreResult<PathBuf> {
        let actual_hex = hex::encode(Sha256::digest(bundle_js.as_bytes()));
        if !expected_sha256_hex.is_empty() && actual_hex != expected_sha256_hex.to_lowercase() {
            return Err(CoreError::Solver(format!(
                "EJS bundle SHA-256 mismatch: expected {expected_sha256_hex}, got {actual_hex}"
            )));
        }
        fs::create_dir_all(&self.ejs_dir)
            .map_err(|e| CoreError::Solver(format!("create ejs dir: {e}")))?;

        let bundle_path = self
            .ejs_dir
            .join(format!("yt.solver.{}.js", manifest.bundle_version));
        let tmp_bundle = bundle_path.with_extension("js.tmp");
        fs::write(&tmp_bundle, bundle_js.as_bytes())
            .map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::rename(&tmp_bundle, &bundle_path).map_err(|e| CoreError::Solver(e.to_string()))?;

        let manifest_path = self.ejs_dir.join("solver_manifest.json");
        let tmp_manifest = manifest_path.with_extension("json.tmp");
        let manifest_json = serde_json::to_string_pretty(manifest)
            .map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::write(&tmp_manifest, manifest_json).map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::rename(&tmp_manifest, &manifest_path)
            .map_err(|e| CoreError::Solver(e.to_string()))?;

        // Clear cached n-params when solver bundle updates to avoid stale outputs.
        self.n_param_cache.lock().unwrap().clear();
        Ok(bundle_path)
    }

    pub fn js_eval_count(&self) -> u64 {
        self.js_eval_count
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn cached_n_param(&self, player_hash: &str, raw_n: &str) -> Option<String> {
        self.n_param_cache
            .lock()
            .unwrap()
            .get(&(player_hash.to_owned(), raw_n.to_owned()))
            .cloned()
    }

    /// Persists an extracted solver bundle atomically on disk keyed by `player_hash`.
    pub fn cache_solver_script(&self, player_hash: &str, js_source: &str) -> CoreResult<PathBuf> {
        fs::create_dir_all(&self.cache_dir)
            .map_err(|e| CoreError::Solver(format!("failed to create solver dir: {e}")))?;
        let target = self.cache_dir.join(format!("{player_hash}.js"));
        if target.exists() {
            return Ok(target);
        }
        let tmp = self.cache_dir.join(format!("{player_hash}.js.tmp"));
        let mut f = fs::File::create(&tmp)
            .map_err(|e| CoreError::Solver(format!("failed to write solver cache: {e}")))?;
        f.write_all(js_source.as_bytes())
            .map_err(|e| CoreError::Solver(e.to_string()))?;
        f.sync_all()
            .map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::rename(&tmp, &target).map_err(|e| CoreError::Solver(e.to_string()))?;
        Ok(target)
    }

    /// Evaluates `solveN(n)` and/or `solveSig(sig)` inside native QuickJS-NG on the worker thread,
    /// short-circuiting from the per-`player_hash` solved `n`-param cache whenever possible.
    pub async fn solve(&self, input: ChallengeInput) -> CoreResult<SolvedChallenge> {
        // Fast path: check per-player-ID solved n-param cache first!
        let cached_n = input
            .n_param
            .as_deref()
            .and_then(|n| self.cached_n_param(&input.player_hash, n));

        if cached_n.is_some() && input.sig_param.is_none() {
            return Ok(SolvedChallenge {
                n_param: cached_n,
                sig_param: None,
            });
        }

        let need_n_eval = if cached_n.is_some() {
            None
        } else {
            input.n_param.clone()
        };

        let script_path = self.cache_solver_script(&input.player_hash, &input.solver_js)?;
        self.js_eval_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        let (tx, rx) = oneshot::channel();
        self.dispatch(WorkerJob::Solve {
            script_path: script_path.clone(),
            n_param: need_n_eval.clone(),
            sig_param: input.sig_param.clone(),
            reply: tx,
        })?;
        let mut res = match rx.await {
            Ok(res) => res?,
            Err(_) => {
                // Worker exited on idle timeout right before send; respawn once.
                let (tx, rx) = oneshot::channel();
                self.dispatch(WorkerJob::Solve {
                    script_path,
                    n_param: need_n_eval.clone(),
                    sig_param: input.sig_param.clone(),
                    reply: tx,
                })?;
                rx.await
                    .map_err(|_| CoreError::Solver("solver worker dropped reply".to_owned()))??
            }
        };

        // Store freshly solved n-param per player_id
        if let (Some(raw_n), Some(solved_n)) = (need_n_eval, res.n_param.clone()) {
            self.n_param_cache
                .lock()
                .unwrap()
                .insert((input.player_hash, raw_n), solved_n);
        } else if res.n_param.is_none() {
            res.n_param = cached_n;
        }
        Ok(res)
    }

    /// Extracts a `signatureCipher` query string and applies solved `sig` + `n` parameters.
    pub async fn solve_cipher_url(
        &self,
        player_hash: &str,
        solver_js: &str,
        signature_cipher: &str,
    ) -> CoreResult<String> {
        let mut url_raw = None;
        let mut s_val = None;
        let mut sp_key = "sig".to_owned();

        for (k, v) in url::form_urlencoded::parse(signature_cipher.as_bytes()) {
            match k.as_ref() {
                "url" => url_raw = Some(v.into_owned()),
                "s" => s_val = Some(v.into_owned()),
                "sp" => sp_key = v.into_owned(),
                _ => {}
            }
        }
        let base_url = url_raw
            .ok_or_else(|| CoreError::Solver("signatureCipher missing url param".to_owned()))?;
        let mut parsed =
            url::Url::parse(&base_url).map_err(|e| CoreError::Solver(format!("invalid url: {e}")))?;

        let n_orig = parsed
            .query_pairs()
            .find(|(k, _)| k == "n")
            .map(|(_, v)| v.into_owned());

        let solved = self
            .solve(ChallengeInput {
                player_hash: player_hash.to_owned(),
                solver_js: solver_js.to_owned(),
                n_param: n_orig,
                sig_param: s_val,
            })
            .await?;

        let mut pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        if let Some(new_n) = solved.n_param {
            if let Some(entry) = pairs.iter_mut().find(|(k, _)| k == "n") {
                entry.1 = new_n;
            } else {
                pairs.push(("n".to_owned(), new_n));
            }
        }
        if let Some(new_sig) = solved.sig_param {
            pairs.retain(|(k, _)| k != &sp_key);
            pairs.push((sp_key, new_sig));
        }

        parsed.query_pairs_mut().clear().extend_pairs(pairs);
        Ok(parsed.to_string())
    }

    fn dispatch(&self, mut job: WorkerJob) -> CoreResult<()> {
        let mut guard = self.worker.lock().unwrap();
        if let Some(tx) = guard.as_ref() {
            match tx.send(job) {
                Ok(()) => return Ok(()),
                Err(e) => job = e.0,
            }
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        std::thread::Builder::new()
            .name("resonance-qjs".into())
            .spawn(move || {
                let mut loaded: Option<(PathBuf, Runtime, Context)> = None;
                loop {
                    match rx.recv_timeout(WORKER_IDLE_TIMEOUT) {
                        Ok(WorkerJob::Solve {
                            script_path,
                            n_param,
                            sig_param,
                            reply,
                        }) => {
                            if loaded.as_ref().is_none_or(|(p, _, _)| *p != script_path) {
                                loaded = None;
                                match load_runtime(&script_path) {
                                    Ok((rt, ctx)) => loaded = Some((script_path, rt, ctx)),
                                    Err(e) => {
                                        let _ = reply.send(Err(e));
                                        continue;
                                    }
                                }
                            }
                            let (_, rt, ctx) = loaded.as_ref().unwrap();
                            let deadline = Instant::now() + JS_WATCHDOG_TIMEOUT;
                            rt.set_interrupt_handler(Some(Box::new(move || {
                                Instant::now() >= deadline
                            })));
                            let res = eval_solve(ctx, n_param, sig_param);
                            rt.set_interrupt_handler(None);
                            let _ = reply.send(res);
                        }
                        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return,
                    }
                }
            })
            .map_err(|e| CoreError::Solver(format!("failed to spawn QuickJS thread: {e}")))?;

        tx.send(job)
            .map_err(|_| CoreError::Solver("failed to enqueue QuickJS job".to_owned()))?;
        *guard = Some(tx);
        Ok(())
    }
}

fn load_runtime(script_path: &Path) -> CoreResult<(Runtime, Context)> {
    let source = fs::read_to_string(script_path)
        .map_err(|e| CoreError::Solver(format!("read solver script: {e}")))?;
    let rt = Runtime::new().map_err(|e| CoreError::Solver(format!("QuickJS runtime: {e}")))?;
    rt.set_memory_limit(64 * 1024 * 1024);
    let ctx =
        Context::full(&rt).map_err(|e| CoreError::Solver(format!("QuickJS context: {e}")))?;
    ctx.with(|ctx| {
        ctx.eval::<(), _>(source.as_bytes())
            .map_err(|e| CoreError::Solver(format!("QuickJS eval error: {e}")))
    })?;
    Ok((rt, ctx))
}

fn eval_solve(
    ctx: &Context,
    n_param: Option<String>,
    sig_param: Option<String>,
) -> CoreResult<SolvedChallenge> {
    ctx.with(|ctx| {
        let globals = ctx.globals();
        let solved_n = if let Some(n) = n_param {
            let func: Function = globals
                .get("solveN")
                .map_err(|e| CoreError::Solver(format!("missing solveN: {e}")))?;
            let out: String = func
                .call((n,))
                .map_err(|e| CoreError::Solver(format!("solveN failed: {e}")))?;
            Some(out)
        } else {
            None
        };

        let solved_sig = if let Some(sig) = sig_param {
            let func: Function = globals
                .get("solveSig")
                .map_err(|e| CoreError::Solver(format!("missing solveSig: {e}")))?;
            let out: String = func
                .call((sig,))
                .map_err(|e| CoreError::Solver(format!("solveSig failed: {e}")))?;
            Some(out)
        } else {
            None
        };

        Ok(SolvedChallenge {
            n_param: solved_n,
            sig_param: solved_sig,
        })
    })
}

/// Helper wrapper to share a single `QuickJsSolver` across providers.
pub type SharedSolver = Arc<QuickJsSolver>;
