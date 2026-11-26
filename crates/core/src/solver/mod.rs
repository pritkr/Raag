//! Sandboxed EJS Challenge Solver with decoupled [`JsRuntime`], SHA-256 manifest pinning,
//! and L1 RAM + L2 SQLite `n`-challenge memoization (< 2 ms after first hit).
//!
//! Architecture:
//! - [`runtime`]: [`JsRuntime`] trait + embedded [`QuickJsRuntime`] (`rquickjs >= 0.12.0`)
//!   + optional Desktop [`DenoSidecarRuntime`].
//! - [`manifest`]: [`EjsBundleManifest`] with `pinned_version` + SHA-256 verification.
//! - [`memo`]: [`EjsMemoCache`] persisting solved `n`-params & AST metadata in SQLite.

pub mod ejs_protocol;
pub mod manifest;
pub mod memo;
pub mod player_fetch;
pub mod runtime;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use resonance_contract::{CoreError, CoreResult};

use crate::cache::SqliteCacheStore;

pub use ejs_protocol::{EjsChallengeRequest, EjsChallengeResponse, EjsInput, EjsOutput};
pub use manifest::{EMBEDDED_EJS_MANIFEST_JSON, EjsBundleManifest};
pub use memo::EjsMemoCache;
pub use player_fetch::{FetchedPlayerScript, PlayerScriptFetcher, evict_old_bundles_in_dir};
#[cfg(not(target_os = "android"))]
pub use runtime::DenoSidecarRuntime;
pub use runtime::{JsRuntime, QuickJsRuntime};

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

pub struct QuickJsSolver {
    cache_dir: PathBuf,
    ejs_dir: PathBuf,
    runtime: Arc<dyn JsRuntime>,
    memo: EjsMemoCache,
    /// Counter for deterministic telemetry/QA verification of JS invocations vs cache hits.
    js_eval_count: AtomicU64,
}

impl QuickJsSolver {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            cache_dir: data_dir.join("js_solver"),
            ejs_dir: data_dir.join("ejs"),
            runtime: Arc::new(QuickJsRuntime::new()),
            memo: EjsMemoCache::default(),
            js_eval_count: AtomicU64::new(0),
        }
    }

    /// Overrides the underlying [`JsRuntime`] (e.g., Desktop [`DenoSidecarRuntime`] or test stub).
    pub fn with_runtime(mut self, runtime: Arc<dyn JsRuntime>) -> Self {
        self.runtime = runtime;
        self
    }

    /// Attaches SQLite WAL persistence to the solver's `n`-challenge memoization cache
    /// so repeated `n`-challenges resolve in < 2 ms across restarts.
    pub fn with_sqlite_cache(mut self, sqlite: Arc<SqliteCacheStore>) -> Self {
        self.memo = self.memo.with_sqlite(sqlite);
        self
    }

    pub fn runtime_name(&self) -> &'static str {
        self.runtime.runtime_name()
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

    /// Hot-installs an updated `yt-dlp-ejs` bundle asset atomically with SHA-256 verification
    /// and version-pinning enforcement.
    pub fn install_ejs_bundle_update(
        &self,
        manifest: &EjsBundleManifest,
        bundle_js: &str,
        expected_sha256_hex: &str,
    ) -> CoreResult<PathBuf> {
        let verified_sha = manifest.verify_bundle_integrity(bundle_js, expected_sha256_hex)?;

        fs::create_dir_all(&self.ejs_dir)
            .map_err(|e| CoreError::Solver(format!("create ejs dir: {e}")))?;

        let bundle_path = self
            .ejs_dir
            .join(format!("yt.solver.{}.js", manifest.bundle_version));
        let tmp_bundle = bundle_path.with_extension("js.tmp");
        fs::write(&tmp_bundle, bundle_js.as_bytes())
            .map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::rename(&tmp_bundle, &bundle_path).map_err(|e| CoreError::Solver(e.to_string()))?;

        let mut persisted_manifest = manifest.clone();
        if persisted_manifest.sha256_hex.is_empty() {
            persisted_manifest.sha256_hex = verified_sha;
        }
        let manifest_path = self.ejs_dir.join("solver_manifest.json");
        let tmp_manifest = manifest_path.with_extension("json.tmp");
        let manifest_json = serde_json::to_string_pretty(&persisted_manifest)
            .map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::write(&tmp_manifest, manifest_json).map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::rename(&tmp_manifest, &manifest_path).map_err(|e| CoreError::Solver(e.to_string()))?;

        // Clear cached n-params when solver bundle updates to avoid stale outputs.
        self.memo.clear();
        Ok(bundle_path)
    }

    pub fn js_eval_count(&self) -> u64 {
        self.js_eval_count.load(Ordering::SeqCst)
    }

    pub fn cached_n_param(&self, player_hash: &str, raw_n: &str) -> Option<String> {
        self.memo.get(player_hash, raw_n)
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
        f.sync_all().map_err(|e| CoreError::Solver(e.to_string()))?;
        fs::rename(&tmp, &target).map_err(|e| CoreError::Solver(e.to_string()))?;
        Ok(target)
    }

    /// Evaluates `solveN(n)` and/or `solveSig(sig)` (or `jsc(input)` if present) via the configured [`JsRuntime`],
    /// short-circuiting from L1 RAM / L2 SQLite memoization whenever possible.
    pub async fn solve(&self, input: ChallengeInput) -> CoreResult<SolvedChallenge> {
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
        self.js_eval_count.fetch_add(1, Ordering::SeqCst);

        let mut res = self
            .runtime
            .eval_challenge(&script_path, need_n_eval.clone(), input.sig_param.clone())
            .await?;

        if let (Some(raw_n), Some(solved_n)) = (need_n_eval, res.n_param.clone()) {
            let meta = serde_json::json!({
                "runtime": self.runtime.runtime_name(),
                "script_bytes": input.solver_js.len(),
            })
            .to_string();
            self.memo
                .insert(&input.player_hash, &raw_n, &solved_n, Some(&meta));
        } else if res.n_param.is_none() {
            res.n_param = cached_n;
        }
        Ok(res)
    }

    /// Two-stage `yt-dlp/ejs` `jsc(input)` solver:
    /// 1. Checks L1/L2 `EjsMemoCache` for `n_param` (< 1 ms on hit).
    /// 2. On cold `player_hash` (no `pre_<hash>.js`), runs `EjsInput::Player { output_preprocessed: true }`
    ///    with temporary 160 MB heap, persists `pre_<hash>.js` atomically, and evicts older bundles (> 3).
    /// 3. On warm `player_hash` (`pre_<hash>.js` exists), runs `EjsInput::Preprocessed` at 64 MB heap.
    pub async fn solve_with_player(
        &self,
        player_hash: &str,
        base_js: &str,
        ejs_bundle_js: &str,
        n_param: Option<String>,
        sig_param: Option<String>,
    ) -> CoreResult<SolvedChallenge> {
        let cached_n = n_param
            .as_deref()
            .and_then(|n| self.cached_n_param(player_hash, n));

        if cached_n.is_some() && sig_param.is_none() {
            return Ok(SolvedChallenge {
                n_param: cached_n,
                sig_param: None,
            });
        }

        let need_n_eval = if cached_n.is_some() {
            None
        } else {
            n_param.clone()
        };

        let bundle_sha = EjsBundleManifest::sha256_of(ejs_bundle_js);
        let bundle_key = format!("jsc_{}", &bundle_sha[..16]);
        let bundle_path = self.cache_solver_script(&bundle_key, ejs_bundle_js)?;

        let pre_path = self.cache_dir.join(format!("pre_{player_hash}.js"));
        let (requests, _, _) =
            EjsInput::build_requests(need_n_eval.as_deref(), sig_param.as_deref());

        self.js_eval_count.fetch_add(1, Ordering::SeqCst);

        let output = if pre_path.exists() {
            let pre_js = fs::read_to_string(&pre_path)
                .map_err(|e| CoreError::Solver(format!("read preprocessed player: {e}")))?;
            let input_json = serde_json::to_string(&EjsInput::Preprocessed {
                preprocessed_player: &pre_js,
                requests,
            })
            .map_err(|e| CoreError::Solver(format!("serialize preprocessed input: {e}")))?;
            self.runtime
                .eval_jsc(&bundle_path, &input_json, false)
                .await?
        } else {
            let input_json = serde_json::to_string(&EjsInput::Player {
                player: base_js,
                requests,
                output_preprocessed: true,
            })
            .map_err(|e| CoreError::Solver(format!("serialize player input: {e}")))?;
            self.runtime
                .eval_jsc(&bundle_path, &input_json, true)
                .await?
        };

        let (mut solved, maybe_pre) =
            output.into_solved(need_n_eval.as_deref(), sig_param.as_deref())?;

        if let Some(preprocessed) = maybe_pre
            && !pre_path.exists()
        {
            player_fetch::write_atomic(&pre_path, preprocessed.as_bytes())?;
            evict_old_bundles_in_dir(&self.cache_dir, 3);
        }

        if let (Some(raw_n), Some(solved_n)) = (need_n_eval, solved.n_param.clone()) {
            if !is_valid_solved_value(&solved_n) {
                return Err(CoreError::Solver(
                    "ejs returned invalid n challenge output".to_owned(),
                ));
            }
            let meta = serde_json::json!({
                "runtime": self.runtime.runtime_name(),
                "protocol": "jsc",
                "bundle_sha": &bundle_sha[..16],
            })
            .to_string();
            self.memo
                .insert(player_hash, &raw_n, &solved_n, Some(&meta));
        } else if solved.n_param.is_none() {
            solved.n_param = cached_n;
        }
        if let Some(ref sig) = solved.sig_param
            && !is_valid_solved_value(sig)
        {
            return Err(CoreError::Solver(
                "ejs returned invalid sig challenge output".to_owned(),
            ));
        }

        Ok(solved)
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
        let mut parsed = url::Url::parse(&base_url)
            .map_err(|e| CoreError::Solver(format!("invalid url: {e}")))?;

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
}

/// Helper wrapper to share a single `QuickJsSolver` across providers.
pub type SharedSolver = Arc<QuickJsSolver>;

fn is_valid_solved_value(val: &str) -> bool {
    !val.is_empty() && val.bytes().all(|b| b.is_ascii_graphic())
}
