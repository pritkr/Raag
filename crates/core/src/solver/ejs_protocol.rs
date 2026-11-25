//! Typed `yt-dlp/ejs` `jsc(input)` JSON protocol for two-stage player AST preprocessing
//! and steady-state `n` / `sig` challenge solving.

use std::collections::HashMap;

use resonance_contract::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};

use crate::innertube::auth::redact_secrets;
use crate::solver::SolvedChallenge;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EjsInput<'a> {
    Player {
        player: &'a str,
        requests: Vec<EjsChallengeRequest>,
        output_preprocessed: bool,
    },
    Preprocessed {
        preprocessed_player: &'a str,
        requests: Vec<EjsChallengeRequest>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct EjsChallengeRequest {
    #[serde(rename = "type")]
    pub kind: &'static str, // "n" or "sig"
    pub challenges: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EjsOutput {
    Result {
        #[serde(default)]
        preprocessed_player: Option<String>,
        #[serde(default)]
        responses: Vec<EjsChallengeResponse>,
    },
    Error {
        error: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EjsChallengeResponse {
    Result { data: HashMap<String, String> },
    Error { error: String },
}

impl EjsInput<'_> {
    /// Builds `requests` for optional `n` and `sig` challenges, returning `(requests, has_n, has_sig)`.
    pub fn build_requests(
        n_param: Option<&str>,
        sig_param: Option<&str>,
    ) -> (Vec<EjsChallengeRequest>, bool, bool) {
        let mut requests = Vec::with_capacity(2);
        let has_n = if let Some(n) = n_param {
            requests.push(EjsChallengeRequest {
                kind: "n",
                challenges: vec![n.to_owned()],
            });
            true
        } else {
            false
        };
        let has_sig = if let Some(sig) = sig_param {
            requests.push(EjsChallengeRequest {
                kind: "sig",
                challenges: vec![sig.to_owned()],
            });
            true
        } else {
            false
        };
        (requests, has_n, has_sig)
    }
}

impl EjsOutput {
    /// Extracts `(SolvedChallenge, Option<preprocessed_player>)` from a parsed `jsc(input)` output.
    pub fn into_solved(
        self,
        n_param: Option<&str>,
        sig_param: Option<&str>,
    ) -> CoreResult<(SolvedChallenge, Option<String>)> {
        match self {
            EjsOutput::Error { error } => Err(CoreError::Solver(redact_secrets(&format!(
                "ejs jsc error: {error}"
            )))),
            EjsOutput::Result {
                preprocessed_player,
                responses,
            } => {
                let mut idx = 0usize;
                let solved_n = if let Some(raw_n) = n_param {
                    let resp = responses.get(idx).ok_or_else(|| {
                        CoreError::Solver("ejs output missing n response".to_owned())
                    })?;
                    idx += 1;
                    Some(extract_challenge_value(resp, raw_n, "n")?)
                } else {
                    None
                };

                let solved_sig = if let Some(raw_sig) = sig_param {
                    let resp = responses.get(idx).ok_or_else(|| {
                        CoreError::Solver("ejs output missing sig response".to_owned())
                    })?;
                    Some(extract_challenge_value(resp, raw_sig, "sig")?)
                } else {
                    None
                };

                Ok((
                    SolvedChallenge {
                        n_param: solved_n,
                        sig_param: solved_sig,
                    },
                    preprocessed_player,
                ))
            }
        }
    }
}

fn extract_challenge_value(
    resp: &EjsChallengeResponse,
    raw_challenge: &str,
    kind: &str,
) -> CoreResult<String> {
    match resp {
        EjsChallengeResponse::Error { error } => Err(CoreError::Solver(redact_secrets(&format!(
            "ejs {kind} challenge error: {error}"
        )))),
        EjsChallengeResponse::Result { data } => {
            data.get(raw_challenge).cloned().ok_or_else(|| {
                CoreError::Solver(format!("ejs {kind} response missing key for challenge"))
            })
        }
    }
}
