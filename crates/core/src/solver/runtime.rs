//! Decoupled sandboxed JavaScript execution runtimes (`JsRuntime` trait).
//!
//! - [`QuickJsRuntime`]: Embedded QuickJS-NG (`rquickjs >= 0.12.0`) compiled directly into the
//!   binary for zero-dependency Desktop & Android execution, running on a dedicated worker thread
//!   with a 10s interrupt watchdog and 64 MB heap cap.
//! - [`DenoSidecarRuntime`] (Desktop only): Optional sandboxed external `deno` sidecar matching
//!   `yt-dlp`'s EJS recommendation, executing with `--no-prompt` and zero net/env/write permissions.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use crossbeam_channel::{RecvTimeoutError, Sender};
use resonance_contract::{CoreError, CoreResult};
use rquickjs::{Context, Function, Runtime};
use tokio::sync::oneshot;

use crate::innertube::auth::redact_secrets;
use crate::solver::SolvedChallenge;
use crate::solver::ejs_protocol::{EjsInput, EjsOutput};

const WORKER_IDLE_TIMEOUT: Duration = Duration::from_secs(20);
const JS_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(10);
const STEADY_STATE_HEAP_BYTES: usize = 64 * 1024 * 1024;
const PREPROCESS_HEAP_BYTES: usize = 160 * 1024 * 1024;

/// Decoupled JavaScript execution engine for EJS challenge solving.
#[async_trait]
pub trait JsRuntime: Send + Sync {
    fn runtime_name(&self) -> &'static str;

    async fn eval_challenge(
        &self,
        script_path: &Path,
        n_param: Option<String>,
        sig_param: Option<String>,
    ) -> CoreResult<SolvedChallenge>;

    /// Evaluates the `yt-dlp/ejs` `jsc(input)` JSON protocol.
    /// When `preprocess_mode` is `true`, uses a temporary 160 MB heap and drops the
    /// VM immediately afterward so steady-state RSS stays bounded.
    async fn eval_jsc(
        &self,
        bundle_path: &Path,
        input_json: &str,
        preprocess_mode: bool,
    ) -> CoreResult<EjsOutput> {
        let _ = (bundle_path, input_json, preprocess_mode);
        Err(CoreError::Solver(format!(
            "{} does not implement jsc() protocol",
            self.runtime_name()
        )))
    }
}

enum WorkerJob {
    Solve {
        script_path: PathBuf,
        n_param: Option<String>,
        sig_param: Option<String>,
        reply: oneshot::Sender<CoreResult<SolvedChallenge>>,
    },
    Jsc {
        bundle_path: PathBuf,
        input_json: String,
        preprocess_mode: bool,
        reply: oneshot::Sender<CoreResult<EjsOutput>>,
    },
}

/// Embedded QuickJS-NG runtime (`rquickjs`) running on a dedicated worker thread with
/// 64 MB memory limit and 10s interrupt watchdog.
pub struct QuickJsRuntime {
    worker: Mutex<Option<Sender<WorkerJob>>>,
}

impl Default for QuickJsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl QuickJsRuntime {
    pub fn new() -> Self {
        Self {
            worker: Mutex::new(None),
        }
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
                                match load_quickjs_context(&script_path, STEADY_STATE_HEAP_BYTES) {
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
                            let res = eval_quickjs_solve(ctx, n_param, sig_param);
                            rt.set_interrupt_handler(None);
                            if res.is_err() {
                                loaded = None;
                            }
                            let _ = reply.send(res);
                        }
                        Ok(WorkerJob::Jsc {
                            bundle_path,
                            input_json,
                            preprocess_mode,
                            reply,
                        }) => {
                            if preprocess_mode {
                                // One-time AST preprocess: 160 MB temporary heap, dropped immediately after.
                                let res =
                                    match load_quickjs_context(&bundle_path, PREPROCESS_HEAP_BYTES)
                                    {
                                        Ok((rt, ctx)) => {
                                            let deadline = Instant::now() + JS_WATCHDOG_TIMEOUT;
                                            rt.set_interrupt_handler(Some(Box::new(move || {
                                                Instant::now() >= deadline
                                            })));
                                            let out = eval_quickjs_jsc(&ctx, &input_json);
                                            rt.set_interrupt_handler(None);
                                            drop(ctx);
                                            drop(rt);
                                            out
                                        }
                                        Err(e) => Err(e),
                                    };
                                let _ = reply.send(res);
                            } else {
                                if loaded.as_ref().is_none_or(|(p, _, _)| *p != bundle_path) {
                                    loaded = None;
                                    match load_quickjs_context(
                                        &bundle_path,
                                        STEADY_STATE_HEAP_BYTES,
                                    ) {
                                        Ok((rt, ctx)) => loaded = Some((bundle_path, rt, ctx)),
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
                                let res = eval_quickjs_jsc(ctx, &input_json);
                                rt.set_interrupt_handler(None);
                                if res.is_err() {
                                    loaded = None;
                                }
                                let _ = reply.send(res);
                            }
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

#[async_trait]
impl JsRuntime for QuickJsRuntime {
    fn runtime_name(&self) -> &'static str {
        "quickjs-ng"
    }

    async fn eval_challenge(
        &self,
        script_path: &Path,
        n_param: Option<String>,
        sig_param: Option<String>,
    ) -> CoreResult<SolvedChallenge> {
        let (tx, rx) = oneshot::channel();
        self.dispatch(WorkerJob::Solve {
            script_path: script_path.to_path_buf(),
            n_param: n_param.clone(),
            sig_param: sig_param.clone(),
            reply: tx,
        })?;
        match rx.await {
            Ok(res) => res,
            Err(_) => {
                // Worker exited on idle timeout right before send; respawn once.
                let (tx, rx) = oneshot::channel();
                self.dispatch(WorkerJob::Solve {
                    script_path: script_path.to_path_buf(),
                    n_param,
                    sig_param,
                    reply: tx,
                })?;
                rx.await
                    .map_err(|_| CoreError::Solver("solver worker dropped reply".to_owned()))?
            }
        }
    }

    async fn eval_jsc(
        &self,
        bundle_path: &Path,
        input_json: &str,
        preprocess_mode: bool,
    ) -> CoreResult<EjsOutput> {
        let (tx, rx) = oneshot::channel();
        self.dispatch(WorkerJob::Jsc {
            bundle_path: bundle_path.to_path_buf(),
            input_json: input_json.to_owned(),
            preprocess_mode,
            reply: tx,
        })?;
        match rx.await {
            Ok(res) => res,
            Err(_) => {
                let (tx, rx) = oneshot::channel();
                self.dispatch(WorkerJob::Jsc {
                    bundle_path: bundle_path.to_path_buf(),
                    input_json: input_json.to_owned(),
                    preprocess_mode,
                    reply: tx,
                })?;
                rx.await
                    .map_err(|_| CoreError::Solver("solver worker dropped jsc reply".to_owned()))?
            }
        }
    }
}

fn load_quickjs_context(
    script_path: &Path,
    heap_limit_bytes: usize,
) -> CoreResult<(Runtime, Context)> {
    let source = fs::read_to_string(script_path)
        .map_err(|e| CoreError::Solver(format!("read solver script: {e}")))?;
    let rt = Runtime::new().map_err(|e| CoreError::Solver(format!("QuickJS runtime: {e}")))?;
    rt.set_memory_limit(heap_limit_bytes);
    rt.set_max_stack_size(1024 * 1024);
    let ctx = Context::full(&rt).map_err(|e| CoreError::Solver(format!("QuickJS context: {e}")))?;
    ctx.with(|ctx| {
        ctx.eval::<(), _>(source.as_bytes())
            .map_err(|e| CoreError::Solver(redact_secrets(&format!("QuickJS eval error: {e}"))))
    })?;
    Ok((rt, ctx))
}

fn eval_quickjs_jsc(ctx: &Context, input_json: &str) -> CoreResult<EjsOutput> {
    ctx.with(|ctx| {
        let globals = ctx.globals();
        let _: Function = globals
            .get("jsc")
            .map_err(|e| CoreError::Solver(format!("missing jsc entrypoint: {e}")))?;
        let wrapper: Function = ctx
            .eval("(function(s){ return JSON.stringify(jsc(JSON.parse(s))); })")
            .map_err(|e| CoreError::Solver(redact_secrets(&format!("jsc wrapper error: {e}"))))?;
        let out_json: String = wrapper
            .call((input_json,))
            .map_err(|e| CoreError::Solver(redact_secrets(&format!("jsc call failed: {e}"))))?;
        serde_json::from_str::<EjsOutput>(&out_json)
            .map_err(|e| CoreError::Solver(format!("invalid jsc output json: {e}")))
    })
}

fn eval_quickjs_solve(
    ctx: &Context,
    n_param: Option<String>,
    sig_param: Option<String>,
) -> CoreResult<SolvedChallenge> {
    let has_legacy = ctx.with(|ctx| {
        let globals = ctx.globals();
        globals.get::<_, Function>("solveN").is_ok()
            || globals.get::<_, Function>("solveSig").is_ok()
    });
    if !has_legacy {
        let has_jsc = ctx.with(|ctx| ctx.globals().get::<_, Function>("jsc").is_ok());
        if has_jsc {
            let (requests, _, _) =
                EjsInput::build_requests(n_param.as_deref(), sig_param.as_deref());
            let input = EjsInput::Preprocessed {
                preprocessed_player: "",
                requests,
            };
            let input_json = serde_json::to_string(&input)
                .map_err(|e| CoreError::Solver(format!("serialize jsc input: {e}")))?;
            let output = eval_quickjs_jsc(ctx, &input_json)?;
            let (solved, _) = output.into_solved(n_param.as_deref(), sig_param.as_deref())?;
            return Ok(solved);
        }
    }

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

/// Optional Desktop external `deno` sidecar runtime matching `yt-dlp`'s EJS recommendation.
/// Runs strictly sandboxed (`--no-prompt`, no network, no env, no write permissions).
#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone)]
pub struct DenoSidecarRuntime {
    deno_binary: PathBuf,
    timeout: Duration,
}

#[cfg(not(target_os = "android"))]
impl DenoSidecarRuntime {
    pub fn new(deno_binary: impl Into<PathBuf>) -> Self {
        Self {
            deno_binary: deno_binary.into(),
            timeout: JS_WATCHDOG_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.max(Duration::from_millis(100));
        self
    }

    /// Detects `deno` on `PATH` when `RESONANCE_USE_DENO=1` or `deno --version` succeeds.
    pub fn detect_on_path() -> Option<Self> {
        let env_opt_in = std::env::var("RESONANCE_USE_DENO")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let path_ok = std::process::Command::new("deno")
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if env_opt_in || path_ok {
            Some(Self::new("deno"))
        } else {
            None
        }
    }

    /// Builds the sandboxed command arguments for invoking Deno with zero privileges.
    pub fn build_sandbox_args(wrapper_js: &str) -> Vec<String> {
        vec![
            "eval".to_owned(),
            "--no-prompt".to_owned(),
            "--no-remote".to_owned(),
            "--no-npm".to_owned(),
            wrapper_js.to_owned(),
        ]
    }
}

#[cfg(not(target_os = "android"))]
#[async_trait]
impl JsRuntime for DenoSidecarRuntime {
    fn runtime_name(&self) -> &'static str {
        "deno-sidecar"
    }

    async fn eval_challenge(
        &self,
        script_path: &Path,
        n_param: Option<String>,
        sig_param: Option<String>,
    ) -> CoreResult<SolvedChallenge> {
        let source = fs::read_to_string(script_path)
            .map_err(|e| CoreError::Solver(format!("read solver script: {e}")))?;
        let req_json = serde_json::json!({
            "n": n_param,
            "sig": sig_param,
        })
        .to_string();
        let wrapper = format!(
            "{source}\nconst __req = {req_json};\nconst __out = {{\n  n_param: __req.n !== null ? String(solveN(__req.n)) : null,\n  sig_param: __req.sig !== null ? String(solveSig(__req.sig)) : null,\n}};\nconsole.log(JSON.stringify(__out));"
        );
        let args = Self::build_sandbox_args(&wrapper);

        let deno_bin = self.deno_binary.clone();
        let output = tokio::time::timeout(
            self.timeout,
            tokio::task::spawn_blocking(move || {
                std::process::Command::new(&deno_bin)
                    .args(&args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
            }),
        )
        .await
        .map_err(|_| CoreError::Solver("deno sidecar timed out".to_owned()))?
        .map_err(|e| CoreError::Solver(format!("deno sidecar task join error: {e}")))?
        .map_err(|e| CoreError::Solver(format!("failed to spawn deno sidecar: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CoreError::Solver(format!(
                "deno sidecar exited with {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .ok_or_else(|| CoreError::Solver("deno sidecar produced empty output".to_owned()))?;
        let parsed: serde_json::Value = serde_json::from_str(last_line)
            .map_err(|e| CoreError::Solver(format!("invalid deno output json: {e}")))?;

        Ok(SolvedChallenge {
            n_param: parsed
                .get("n_param")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            sig_param: parsed
                .get("sig_param")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
        })
    }

    async fn eval_jsc(
        &self,
        bundle_path: &Path,
        input_json: &str,
        _preprocess_mode: bool,
    ) -> CoreResult<EjsOutput> {
        let source = fs::read_to_string(bundle_path)
            .map_err(|e| CoreError::Solver(format!("read solver bundle: {e}")))?;
        let wrapper = format!(
            "{source}\nconst __in = JSON.parse({input_json_str});\nconsole.log(JSON.stringify(jsc(__in)));",
            input_json_str = serde_json::to_string(input_json)
                .map_err(|e| CoreError::Solver(format!("quote jsc input: {e}")))?
        );
        let args = Self::build_sandbox_args(&wrapper);
        let deno_bin = self.deno_binary.clone();
        let output = tokio::time::timeout(
            self.timeout,
            tokio::task::spawn_blocking(move || {
                std::process::Command::new(&deno_bin)
                    .args(&args)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
            }),
        )
        .await
        .map_err(|_| CoreError::Solver("deno sidecar timed out".to_owned()))?
        .map_err(|e| CoreError::Solver(format!("deno sidecar task join error: {e}")))?
        .map_err(|e| CoreError::Solver(format!("failed to spawn deno sidecar: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CoreError::Solver(redact_secrets(&format!(
                "deno sidecar exited with {}: {}",
                output.status,
                stderr.trim()
            ))));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .ok_or_else(|| CoreError::Solver("deno sidecar produced empty output".to_owned()))?;
        serde_json::from_str::<EjsOutput>(last_line)
            .map_err(|e| CoreError::Solver(format!("invalid deno jsc output json: {e}")))
    }
}
