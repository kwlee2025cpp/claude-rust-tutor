//! Provider configs — port of llm_configs.py's 5 dataclasses.
//!
//! Each variant carries (api_key, model) and implements the four hooks
//! the generic client needs: `api_url`, `headers`, `body`, `parse`.

use std::env;

use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub enum Provider {
    Gemini { api_key: String, model: String },
    Claude { api_key: String, model: String },
    Grok { api_key: String, model: String },
    NvidiaNim { api_key: String, model: String },
    Perplexity { api_key: String, model: String },
    /// TU Korea AI Gateway (campus credits). OpenAI-compatible wire format.
    /// `model` is the WIRE id — the `gateway/` selector prefix is stripped in
    /// `build_provider`, because the Gateway's own catalogue ids are bare
    /// (`claude-fable-5-1`, `gemini-3.5-flash-lite`).
    Gateway { api_key: String, model: String },
}

impl Provider {
    pub fn label(&self) -> &'static str {
        match self {
            Provider::Gemini { .. } => "Gemini",
            Provider::Claude { .. } => "Claude",
            Provider::Grok { .. } => "Grok",
            Provider::NvidiaNim { .. } => "NvidiaNIM",
            Provider::Perplexity { .. } => "Perplexity",
            Provider::Gateway { .. } => "TUKoreaGateway",
        }
    }

    /// POST endpoint URL. Gemini embeds the API key in the URL.
    pub fn api_url(&self) -> String {
        match self {
            Provider::Gemini { api_key, model } => format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={key}",
                key = api_key.trim()
            ),
            Provider::Claude { .. } => "https://api.anthropic.com/v1/messages".into(),
            Provider::Grok { .. } => "https://api.x.ai/v1/chat/completions".into(),
            Provider::NvidiaNim { .. } => "https://integrate.api.nvidia.com/v1/chat/completions".into(),
            Provider::Perplexity { .. } => "https://api.perplexity.ai/chat/completions".into(),
            // OpenAI-compatible base `…/v1/gateway` + the standard path.
            Provider::Gateway { .. } => {
                "https://factchat-cloud.mindlogic.ai/v1/gateway/chat/completions".into()
            }
        }
    }

    /// Provider-specific headers. Returned as `(name, value)` pairs so the
    /// caller can build a `reqwest::HeaderMap`.
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        match self {
            Provider::Gemini { .. } => vec![("Content-Type", "application/json".into())],
            Provider::Claude { api_key, .. } => vec![
                ("x-api-key", api_key.trim().into()),
                ("anthropic-version", "2023-06-01".into()),
                ("Content-Type", "application/json".into()),
            ],
            Provider::Grok { api_key, .. }
            | Provider::NvidiaNim { api_key, .. }
            | Provider::Perplexity { api_key, .. }
            | Provider::Gateway { api_key, .. } => vec![
                ("Authorization", format!("Bearer {}", api_key.trim())),
                ("Content-Type", "application/json".into()),
            ],
        }
    }

    pub fn body(&self, prompt: &str) -> Value {
        match self {
            Provider::Gemini { .. } => json!({
                "contents": [{"parts": [{"text": prompt}]}]
            }),
            Provider::Grok { model, .. } => json!({
                "messages": [{"role": "user", "content": prompt}],
                "model": model,
                "stream": false,
                "temperature": 0,
            }),
            Provider::NvidiaNim { model, .. } => json!({
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.2,
                "top_p": 0.7,
                "max_tokens": 96,
                "stream": false,
            }),
            Provider::Claude { model, .. } => {
                let max_tokens = 1024;
                json!({
                    "model": model,
                    "messages": [{
                        "role": "user",
                        "content": format!("Please answer within {max_tokens} tokens\n{prompt}")
                    }],
                    "temperature": 0.2,
                    "top_p": 0.7,
                    "max_tokens": max_tokens,
                    "stream": false,
                })
            }
            // Deliberately minimal: `model`/`messages`/`temperature`/
            // `max_tokens`/`stream` only. The Gateway fronts many vendors, so
            // every extra knob is another chance at a 400 from whichever
            // backend the id resolves to. `max_tokens` is the credit guard.
            //
            // `reasoning_effort: "none"` is the ONE exception, and it is not a
            // tuning knob -- without it this provider silently delivers almost
            // nothing. `max_tokens` caps reasoning AND visible output together,
            // so a model that thinks by default spends the budget before it
            // speaks: measured on gemini-3.5-flash through this Gateway
            // (grader-hub run 36992477165, 2026-10-02), a real 0/5 prompt came
            // back with 41 visible tokens of a cut-off answer and ~980 of the
            // 1024 spent on hidden reasoning. The same prompt with this field
            // returned 720 visible tokens -- a complete answer -- and cost LESS
            // (18.09 vs 20.79 credits), because reasoning bills at the output
            // rate too. Every student reply between 2026-09-30 00:58 and
            // 2026-10-02 19:11 KST was truncated this way. Sent unconditionally
            // rather than gated on a model family: it is part of the
            // OpenAI-compatible schema the Gateway speaks, a backend that does
            // not reason has nothing to do with it, and a family allow-list
            // would have to be edited for every new id in a 59-model catalogue
            // -- the failure it prevents is silent, so the default must be safe.
            Provider::Gateway { model, .. } => json!({
                "model": model,
                "messages": [{"role": "user", "content": prompt}],
                "temperature": 0.2,
                "max_tokens": 1024,
                "reasoning_effort": "none",
                "stream": false,
            }),
            Provider::Perplexity { model, .. } => {
                let max_tokens = 384;
                let half = max_tokens / 2;
                let resolved = match model.as_str() {
                    "sonar-deep-research" | "sonar-reasoning-pro" | "sonar-reasoning"
                    | "sonar-pro" | "sonar" => model.as_str(),
                    _ => "sonar",
                };
                json!({
                    "model": resolved,
                    "messages": [{
                        "role": "user",
                        "content": format!("Please answer within {half} tokens.Do not include code.\n{prompt}")
                    }],
                    "temperature": 0.2,
                    "top_p": 0.7,
                    "max_tokens": max_tokens,
                    "stream": false,
                })
            }
        }
    }

    pub fn parse(&self, response: &Value) -> anyhow::Result<String> {
        match self {
            Provider::Gemini { .. } => {
                let parts = response
                    .pointer("/candidates/0/content/parts")
                    .and_then(Value::as_array)
                    .ok_or_else(|| anyhow::anyhow!("gemini: missing candidates[0].content.parts"))?;
                let mut chunks = Vec::with_capacity(parts.len());
                for p in parts {
                    let t = p
                        .get("text")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("gemini: missing parts[].text"))?;
                    chunks.push(t.to_string());
                }
                Ok(chunks.join("\n"))
            }
            Provider::Claude { .. } => response
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("claude: missing content[0].text")),
            Provider::Grok { .. }
            | Provider::NvidiaNim { .. }
            | Provider::Perplexity { .. }
            | Provider::Gateway { .. } => response
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("{}: missing choices[0].message.content", self.label())),
        }
    }
}

/// Which provider a model name maps to. Mirrors the python tutor's
/// `get_config_class` (llm_utils.py): a case-insensitive **prefix** match on
/// the model id. `gemini*`→Gemini, `claude*`→Claude, `grok*`→Grok,
/// `nvidia*`→NvidiaNIM, `sonar*`→Perplexity. Returns `None` for an unknown
/// model so the caller can decide whether to error or fall back.
///
/// NOTE: python additionally maps the explicit nvidia model id
/// `google/gemma-2-9b-it` to NvidiaNIM. We require an `INPUT_MODEL` that
/// starts with `nvidia` to select NvidiaNIM here; the org default
/// (`DEFAULT_MODEL`) does not use the gemma id, and the no-`INPUT_MODEL`
/// key-order fallback below still reaches NvidiaNIM, so this is sufficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderKind {
    Gateway,
    Gemini,
    Claude,
    Grok,
    NvidiaNim,
    Perplexity,
}

pub(crate) const GATEWAY_PREFIX: &str = "gateway/";

fn provider_kind_for_model(model: &str) -> Option<ProviderKind> {
    // Order matters HERE and nowhere else in this function: the Gateway's
    // catalogue ids ARE vendor ids (`claude-fable-5-1`, `gemini-3.5-flash-lite`),
    // so `gateway/` must be tested first. Without the selector prefix a
    // Gateway id matches the vendor arm and the call leaves for the vendor's
    // PUBLIC api on the vendor's key — billed to the wrong account, silently.
    // The prefix is the only thing that distinguishes "this id, via campus
    // credits" from "this id, via the vendor".
    let m = model.trim().to_ascii_lowercase();
    if m.starts_with(GATEWAY_PREFIX) {
        Some(ProviderKind::Gateway)
    } else if m.starts_with("gemini") {
        Some(ProviderKind::Gemini)
    } else if m.starts_with("claude") {
        Some(ProviderKind::Claude)
    } else if m.starts_with("grok") {
        Some(ProviderKind::Grok)
    } else if m.starts_with("nvidia") {
        Some(ProviderKind::NvidiaNim)
    } else if m.starts_with("sonar") {
        Some(ProviderKind::Perplexity)
    } else {
        None
    }
}

/// Build the chosen provider, reading its matching `INPUT_<provider>_API_KEY`.
fn build_provider(kind: ProviderKind, model: String) -> anyhow::Result<Provider> {
    let p = match kind {
        ProviderKind::Gateway => {
            // Strip by LENGTH off the original string, not the lowercased
            // copy, so the wire id keeps its case. The prefix matched
            // case-insensitively above, hence the length is known-good.
            let wire = model[GATEWAY_PREFIX.len()..].trim().to_string();
            if wire.is_empty() {
                anyhow::bail!(
                    "INPUT_MODEL='{model}' has the '{GATEWAY_PREFIX}' selector but no model id \
                     after it; use e.g. INPUT_MODEL={GATEWAY_PREFIX}claude-fable-5-1 \
                     (ids come from GET /v1/gateway/models/)"
                );
            }
            Provider::Gateway {
                api_key: require_key("INPUT_TUKOREA_GATEWAY_API_KEY", "TUKoreaGateway")?,
                model: wire,
            }
        }
        ProviderKind::Gemini => Provider::Gemini {
            api_key: require_key("INPUT_GEMINI_API_KEY", "Gemini")?,
            model,
        },
        ProviderKind::Claude => Provider::Claude {
            api_key: require_key("INPUT_CLAUDE_API_KEY", "Claude")?,
            model,
        },
        ProviderKind::Grok => Provider::Grok {
            api_key: require_key("INPUT_GROK_API_KEY", "Grok")?,
            model,
        },
        ProviderKind::NvidiaNim => Provider::NvidiaNim {
            api_key: require_key("INPUT_NVIDIA_API_KEY", "NvidiaNIM")?,
            model,
        },
        ProviderKind::Perplexity => Provider::Perplexity {
            api_key: require_key("INPUT_PERPLEXITY_API_KEY", "Perplexity")?,
            model,
        },
    };
    Ok(p)
}

/// Select the active LLM provider.
///
/// Primary path — mirror the python tutor (llm_utils.py `get_config_class`):
/// when `INPUT_MODEL` is set, **select the provider FROM the model name** by
/// prefix, and use that provider's matching `INPUT_<provider>_API_KEY`. This
/// is what makes the org's `DEFAULT_MODEL` (passed as `INPUT_MODEL`) honored —
/// e.g. a `claude-…` default picks Claude even when a Gemini key is also set.
///
/// Fallback path — only when `INPUT_MODEL` is empty: keep the historical
/// first-non-empty-key order (Gemini → Claude → Grok → Nvidia → Perplexity)
/// with each provider's default model.
///
/// All env names use UNDERSCORES, not hyphens: a hyphenated name inherited
/// from the process startup environment is invisible to `std::env::var`
/// (libstd drops `-`-containing names from its inherited environ), so a
/// hyphenated key is unreadable here even when `docker run -e` set it. See
/// `Inputs::from_env`.
pub fn select_from_env() -> anyhow::Result<Provider> {
    let model_override = env::var("INPUT_MODEL").ok().filter(|s| !s.trim().is_empty());

    // Primary: provider-by-model when INPUT_MODEL is set.
    if let Some(model) = model_override {
        match provider_kind_for_model(&model) {
            Some(kind) => return build_provider(kind, model.trim().to_string()),
            None => anyhow::bail!(
                "INPUT_MODEL='{model}' does not match any known provider prefix \
                 (gateway/|gemini|claude|grok|nvidia|sonar)"
            ),
        }
    }

    // Fallback: no INPUT_MODEL — first non-empty key wins, with that
    // provider's default model.
    if let Some(key) = nonempty("INPUT_GEMINI_API_KEY") {
        return Ok(Provider::Gemini { api_key: key, model: "gemini-2.5-flash".into() });
    }
    if let Some(key) = nonempty("INPUT_CLAUDE_API_KEY") {
        return Ok(Provider::Claude { api_key: key, model: "claude-sonnet-4-20250514".into() });
    }
    if let Some(key) = nonempty("INPUT_GROK_API_KEY") {
        return Ok(Provider::Grok { api_key: key, model: "grok-code-fast".into() });
    }
    if let Some(key) = nonempty("INPUT_NVIDIA_API_KEY") {
        return Ok(Provider::NvidiaNim { api_key: key, model: "google/gemma-2-9b-it".into() });
    }
    if let Some(key) = nonempty("INPUT_PERPLEXITY_API_KEY") {
        return Ok(Provider::Perplexity { api_key: key, model: "sonar".into() });
    }
    if nonempty("INPUT_TUKOREA_GATEWAY_API_KEY").is_some() {
        anyhow::bail!(
            "INPUT_TUKOREA_GATEWAY_API_KEY is set but INPUT_MODEL is empty. The Gateway \
             fronts many vendors and has no default model, so the id must be explicit: \
             set INPUT_MODEL={GATEWAY_PREFIX}<id> (ids from GET /v1/gateway/models/)"
        )
    }
    anyhow::bail!("no LLM API key set in env (INPUT_*_API_KEY)")
}

fn nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|s| !s.trim().is_empty())
}

/// Read a required provider key; error with a clear message if missing/empty.
fn require_key(name: &str, provider: &str) -> anyhow::Result<String> {
    nonempty(name).ok_or_else(|| {
        anyhow::anyhow!(
            "INPUT_MODEL selected provider {provider}, but its key {name} is unset or empty"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every env var the gateway tests touch — cleared before and after each
    /// so a key leaking from the runner env cannot change a verdict.
    const GATEWAY_TEST_VARS: [&str; 7] = [
        "INPUT_GEMINI_API_KEY",
        "INPUT_CLAUDE_API_KEY",
        "INPUT_GROK_API_KEY",
        "INPUT_NVIDIA_API_KEY",
        "INPUT_PERPLEXITY_API_KEY",
        "INPUT_TUKOREA_GATEWAY_API_KEY",
        "INPUT_MODEL",
    ];

    #[test]
    fn gemini_url_embeds_key_and_model() {
        let p = Provider::Gemini { api_key: "K".into(), model: "gemini-2.5-flash".into() };
        let url = p.api_url();
        assert!(url.contains("gemini-2.5-flash:generateContent"));
        assert!(url.ends_with("?key=K"));
    }

    #[test]
    fn gemini_url_trims_whitespace_from_key() {
        // Regression: a key with a trailing newline (common from
        // `$(cat secret)` style injection) must not break the URL. Every other
        // provider trims; Gemini now does too.
        let p = Provider::Gemini { api_key: "  K\n".into(), model: "gemini-2.5-flash".into() };
        let url = p.api_url();
        assert!(url.ends_with("?key=K"), "expected trimmed key, got: {url}");
    }

    #[test]
    fn model_maps_to_provider_by_prefix() {
        // `ProviderKind` + `provider_kind_for_model` are in scope via
        // `use super::*` at the top of this module.
        // Mirror python's get_config_class prefix match.
        assert_eq!(provider_kind_for_model("gemini-2.5-flash"), Some(ProviderKind::Gemini));
        assert_eq!(provider_kind_for_model("claude-sonnet-4-20250514"), Some(ProviderKind::Claude));
        assert_eq!(provider_kind_for_model("grok-code-fast"), Some(ProviderKind::Grok));
        assert_eq!(provider_kind_for_model("nvidia/llama"), Some(ProviderKind::NvidiaNim));
        assert_eq!(provider_kind_for_model("sonar-pro"), Some(ProviderKind::Perplexity));
        // Case-insensitive + whitespace-tolerant, like python's .lower().
        assert_eq!(provider_kind_for_model("  Claude-3  "), Some(ProviderKind::Claude));
        // Unknown -> None (caller errors).
        assert_eq!(provider_kind_for_model("mistral-large"), None);
    }

    #[test]
    fn claude_headers_carry_api_key_and_version() {
        let p = Provider::Claude { api_key: "K".into(), model: "claude-sonnet-4-20250514".into() };
        let hs = p.headers();
        assert!(hs.iter().any(|(k, v)| *k == "x-api-key" && v == "K"));
        assert!(hs.iter().any(|(k, v)| *k == "anthropic-version" && v == "2023-06-01"));
    }

    #[test]
    fn grok_body_includes_model_and_temp_zero() {
        let p = Provider::Grok { api_key: "K".into(), model: "grok-code-fast".into() };
        let b = p.body("hi");
        assert_eq!(b["model"], "grok-code-fast");
        assert_eq!(b["temperature"], 0);
        assert_eq!(b["messages"][0]["content"], "hi");
    }

    #[test]
    fn perplexity_model_fallback_to_sonar() {
        let p = Provider::Perplexity { api_key: "K".into(), model: "made-up-model".into() };
        let b = p.body("hi");
        assert_eq!(b["model"], "sonar");
    }

    #[test]
    fn gemini_parses_candidates_parts() {
        let p = Provider::Gemini { api_key: "K".into(), model: "gemini-2.5-flash".into() };
        let r = serde_json::json!({
            "candidates": [{"content": {"parts": [{"text": "hello"}, {"text": "world"}]}}]
        });
        assert_eq!(p.parse(&r).unwrap(), "hello\nworld");
    }

    #[test]
    fn claude_parses_content_text() {
        let p = Provider::Claude { api_key: "K".into(), model: "claude-sonnet-4-20250514".into() };
        let r = serde_json::json!({"content": [{"text": "ok"}]});
        assert_eq!(p.parse(&r).unwrap(), "ok");
    }

    #[test]
    fn grok_parses_choices_content() {
        let p = Provider::Grok { api_key: "K".into(), model: "grok-code-fast".into() };
        let r = serde_json::json!({"choices": [{"message": {"content": "ok"}}]});
        assert_eq!(p.parse(&r).unwrap(), "ok");
    }

    // Regression for the empty-feedback bug: the env contract must use
    // UNDERSCORE names, because a hyphenated name inherited from the process
    // startup environment (as `docker run -e "INPUT_X-Y=..."` produces) is
    // invisible to `std::env::var` — it is dropped from Rust's startup env
    // snapshot. This test asserts the underscore Claude key resolves.
    //
    // NB: the hyphen-invisibility itself cannot be reproduced in-process —
    // `env::set_var("A-B", ..)` followed by `env::var("A-B")` DOES read back,
    // because libstd only filters the *inherited* environ, not vars set after
    // startup. The invisibility is proven out-of-process in the task's docker
    // repro; here we just lock in that the underscore selector works.
    #[test]
    fn select_from_env_uses_underscore_names() {
        // SAFETY: single-threaded within this test; we set then remove the
        // vars we touch, leaving process env as we found it.
        unsafe {
            // Clear any provider keys that might leak from the runner env.
            for k in ["INPUT_GEMINI_API_KEY", "INPUT_CLAUDE_API_KEY",
                      "INPUT_GROK_API_KEY", "INPUT_NVIDIA_API_KEY",
                      "INPUT_PERPLEXITY_API_KEY", "INPUT_MODEL"] {
                env::remove_var(k);
            }
            env::set_var("INPUT_CLAUDE_API_KEY", "sk-test");
        }

        let p = select_from_env().expect("underscore claude key should resolve");
        match p {
            Provider::Claude { api_key, .. } => assert_eq!(api_key, "sk-test"),
            other => panic!("expected Claude provider, got {other:?}"),
        }

        unsafe {
            env::remove_var("INPUT_CLAUDE_API_KEY");
        }
    }

    // Regression for the p000 Gemini-400: with INPUT_MODEL set to a Claude
    // model AND a Gemini key also present, the OLD first-non-empty-key order
    // picked Gemini and sent a Claude default model to Gemini -> 400. The fix
    // selects the provider FROM the model name, so Claude wins and uses the
    // Claude key. This is the core behavior change.
    #[test]
    fn input_model_selects_provider_by_name_over_key_order() {
        unsafe {
            for k in ["INPUT_GEMINI_API_KEY", "INPUT_CLAUDE_API_KEY",
                      "INPUT_GROK_API_KEY", "INPUT_NVIDIA_API_KEY",
                      "INPUT_PERPLEXITY_API_KEY", "INPUT_MODEL"] {
                env::remove_var(k);
            }
            // Both keys present; Gemini would win under the old order.
            env::set_var("INPUT_GEMINI_API_KEY", "gem-key");
            env::set_var("INPUT_CLAUDE_API_KEY", "claude-key");
            env::set_var("INPUT_MODEL", "claude-sonnet-4-20250514");
        }

        let p = select_from_env().expect("claude model + claude key should resolve");
        match p {
            Provider::Claude { api_key, model } => {
                assert_eq!(api_key, "claude-key");
                assert_eq!(model, "claude-sonnet-4-20250514");
            }
            other => panic!("expected Claude provider from INPUT_MODEL, got {other:?}"),
        }

        unsafe {
            for k in ["INPUT_GEMINI_API_KEY", "INPUT_CLAUDE_API_KEY", "INPUT_MODEL"] {
                env::remove_var(k);
            }
        }
    }

    // When INPUT_MODEL names a provider whose key is missing, error clearly
    // rather than silently falling through to another provider.
    #[test]
    fn input_model_without_matching_key_errors() {
        unsafe {
            for k in ["INPUT_GEMINI_API_KEY", "INPUT_CLAUDE_API_KEY",
                      "INPUT_GROK_API_KEY", "INPUT_NVIDIA_API_KEY",
                      "INPUT_PERPLEXITY_API_KEY", "INPUT_MODEL"] {
                env::remove_var(k);
            }
            // Model selects Claude, but only a Gemini key is set.
            env::set_var("INPUT_GEMINI_API_KEY", "gem-key");
            env::set_var("INPUT_MODEL", "claude-sonnet-4-20250514");
        }

        assert!(select_from_env().is_err(), "missing Claude key should error");

        unsafe {
            for k in ["INPUT_GEMINI_API_KEY", "INPUT_MODEL"] {
                env::remove_var(k);
            }
        }
    }

    // ---- TU Korea Gateway (v0.2) ------------------------------------------

    // THE trap this provider exists to close: the Gateway's catalogue ids are
    // vendor ids. Without the selector prefix they match the vendor arm and
    // the call leaves for the vendor's public API on the vendor's key.
    #[test]
    fn gateway_prefix_wins_and_bare_ids_still_go_to_the_vendor() {
        assert_eq!(provider_kind_for_model("gateway/claude-fable-5-1"), Some(ProviderKind::Gateway));
        assert_eq!(provider_kind_for_model("gateway/gemini-3.5-flash-lite"), Some(ProviderKind::Gateway));
        assert_eq!(provider_kind_for_model("  GATEWAY/Claude-Fable-5  "), Some(ProviderKind::Gateway));
        // Bare Gateway ids: documented, deliberate, and NOT the Gateway.
        assert_eq!(provider_kind_for_model("claude-fable-5-1"), Some(ProviderKind::Claude));
        assert_eq!(provider_kind_for_model("gemini-3.5-flash-lite"), Some(ProviderKind::Gemini));
    }

    #[test]
    fn gateway_url_is_openai_compatible_path() {
        let p = Provider::Gateway { api_key: "K".into(), model: "claude-fable-5-1".into() };
        assert_eq!(p.api_url(), "https://factchat-cloud.mindlogic.ai/v1/gateway/chat/completions");
    }

    #[test]
    fn gateway_headers_carry_bearer() {
        let p = Provider::Gateway { api_key: "  K\n".into(), model: "m".into() };
        let hs = p.headers();
        assert!(hs.iter().any(|(k, v)| *k == "Authorization" && v == "Bearer K"));
        assert!(hs.iter().any(|(k, v)| *k == "Content-Type" && v == "application/json"));
    }

    #[test]
    fn gateway_body_is_minimal_and_capped() {
        let p = Provider::Gateway { api_key: "K".into(), model: "claude-fable-5-1".into() };
        let b = p.body("hi");
        assert_eq!(b["model"], "claude-fable-5-1");
        assert_eq!(b["messages"][0]["content"], "hi");
        assert_eq!(b["max_tokens"], 1024);
        assert_eq!(b["stream"], false);
        // No top_p: one less knob for a fronted backend to reject.
        assert!(b.get("top_p").is_none(), "body should stay minimal: {b}");
    }

    // The one field that is NOT optional. `max_tokens` caps hidden reasoning
    // and visible output together, so without this a reasoning model answers
    // the student in ~40 tokens and the rest of the budget is spent thinking
    // (measured 2026-10-02: 41 visible tokens vs 720 with the field, at a
    // HIGHER cost). The failure is silent -- a 200, a plausible opening
    // sentence, no error anywhere -- so it is asserted for every Gateway
    // model, not just the family it was measured on.
    #[test]
    fn gateway_body_disables_hidden_reasoning_for_every_model() {
        for id in [
            "gemini-3.5-flash",
            "gemini-3.5-flash-lite",
            "claude-fable-5-1",
            "claude-haiku-4-5-20251001",
            "some-future-id",
        ] {
            let p = Provider::Gateway { api_key: "K".into(), model: id.into() };
            let b = p.body("hi");
            assert_eq!(
                b["reasoning_effort"], "none",
                "{id}: without reasoning_effort=none the reply is truncated to ~40 tokens: {b}"
            );
        }
    }

    #[test]
    fn gateway_parses_choices_content() {
        let p = Provider::Gateway { api_key: "K".into(), model: "m".into() };
        let r = serde_json::json!({"choices": [{"message": {"content": "ok"}}]});
        assert_eq!(p.parse(&r).unwrap(), "ok");
    }

    // The selector prefix must never reach the wire: the Gateway knows
    // `claude-fable-5-1`, not `gateway/claude-fable-5-1`. Case is preserved.
    #[test]
    fn gateway_strips_selector_prefix_from_wire_id() {
        unsafe {
            for k in GATEWAY_TEST_VARS { env::remove_var(k); }
            env::set_var("INPUT_TUKOREA_GATEWAY_API_KEY", "gw-key");
            env::set_var("INPUT_MODEL", "gateway/Claude-Fable-5-1");
        }
        let p = select_from_env().expect("gateway/ id + gateway key should resolve");
        match p {
            Provider::Gateway { api_key, model } => {
                assert_eq!(api_key, "gw-key");
                assert_eq!(model, "Claude-Fable-5-1", "selector prefix must be stripped");
            }
            other => panic!("expected Gateway provider, got {other:?}"),
        }
        unsafe { for k in GATEWAY_TEST_VARS { env::remove_var(k); } }
    }

    // A gateway key with no INPUT_MODEL used to fall through to a vendor key
    // (or to the generic "no key" error). It must say what is actually wrong.
    #[test]
    fn gateway_key_without_input_model_errors_clearly() {
        unsafe {
            for k in GATEWAY_TEST_VARS { env::remove_var(k); }
            env::set_var("INPUT_TUKOREA_GATEWAY_API_KEY", "gw-key");
        }
        let e = select_from_env().expect_err("gateway key without INPUT_MODEL must error");
        let msg = e.to_string();
        assert!(msg.contains("INPUT_MODEL"), "error should name INPUT_MODEL: {msg}");
        assert!(msg.contains("gateway/"), "error should show the selector: {msg}");
        unsafe { for k in GATEWAY_TEST_VARS { env::remove_var(k); } }
    }

    #[test]
    fn gateway_selector_without_an_id_errors() {
        unsafe {
            for k in GATEWAY_TEST_VARS { env::remove_var(k); }
            env::set_var("INPUT_TUKOREA_GATEWAY_API_KEY", "gw-key");
            env::set_var("INPUT_MODEL", "gateway/");
        }
        assert!(select_from_env().is_err(), "bare 'gateway/' must error");
        unsafe { for k in GATEWAY_TEST_VARS { env::remove_var(k); } }
    }

    #[test]
    fn gateway_model_without_its_key_errors() {
        unsafe {
            for k in GATEWAY_TEST_VARS { env::remove_var(k); }
            // A vendor key is present; it must NOT be used for a gateway/ id.
            env::set_var("INPUT_CLAUDE_API_KEY", "claude-key");
            env::set_var("INPUT_MODEL", "gateway/claude-fable-5-1");
        }
        let e = select_from_env().expect_err("gateway id without gateway key must error");
        assert!(
            e.to_string().contains("INPUT_TUKOREA_GATEWAY_API_KEY"),
            "error should name the gateway key: {e}"
        );
        unsafe { for k in GATEWAY_TEST_VARS { env::remove_var(k); } }
    }
}
