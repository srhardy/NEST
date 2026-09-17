//! Forage — the provider layer.
//!
//! Bots hatch on harness CLIs (perches), but the engine also needs direct
//! completions — routing decisions, summaries, the future zero-cost
//! failover proxy plugs in here. `Forage` is one call: prompt in, text
//! out, provider named on the response.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

/// A completion request. `tier` is the budget hint (concise|balanced|full)
/// the provider layer resolves to a concrete model.
#[derive(Debug, Clone)]
pub struct ForageRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub tier: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ForageResponse {
    pub text: String,
    pub provider: String,
}

/// One provider.
pub trait Forage {
    fn name(&self) -> &str;
    fn complete(&self, req: &ForageRequest) -> Result<ForageResponse>;
}

/// Builtin stub — echoes the prompt back. Keeps the whole loop runnable
/// with zero keys and zero cost; tests use it too.
pub struct EchoForage;
impl Forage for EchoForage {
    fn name(&self) -> &str {
        "echo"
    }
    fn complete(&self, req: &ForageRequest) -> Result<ForageResponse> {
        Ok(ForageResponse {
            text: req.prompt.clone(),
            provider: self.name().into(),
        })
    }
}

/// Any local CLI as a provider — `llm`, `ollama run <m>`, `claude -p`, …
/// Prompt goes to stdin, stdout is the completion. Zero API keys needed
/// for CLIs that authenticate themselves.
pub struct CommandForage {
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl Forage for CommandForage {
    fn name(&self) -> &str {
        &self.name
    }
    fn complete(&self, req: &ForageRequest) -> Result<ForageResponse> {
        let mut child = Command::new(&self.program)
            .args(&self.args)
            .envs(&self.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("cannot spawn provider '{}'", self.program))?;
        child
            .stdin
            .as_mut()
            .context("provider stdin unavailable")?
            .write_all(req.prompt.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            anyhow::bail!(
                "provider '{}' exited {}: {}",
                self.program,
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(ForageResponse {
            text: String::from_utf8_lossy(&out.stdout).to_string(),
            provider: self.name.clone(),
        })
    }
}

/// Any OpenAI-compatible endpoint — the freellmapi/router pattern:
/// `POST {base_url}/chat/completions` through curl, so free-tier
/// aggregators and self-hosted gateways plug in declaratively with no
/// new deps. The key comes from `api_key_env`, never the file itself.
pub struct OpenAiForage {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
}

impl Forage for OpenAiForage {
    fn name(&self) -> &str {
        &self.name
    }
    fn complete(&self, req: &ForageRequest) -> Result<ForageResponse> {
        let body = serde_json::json!({
            "model": self.model,
            "messages": [{"role": "user", "content": req.prompt}],
        });
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let mut cmd = Command::new("curl");
        cmd.args([
            "-fsS",
            "--max-time",
            "60",
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
        ]);
        // the bearer header travels in a 0600 temp file — argv is
        // world-readable on most systems, the token isn't
        let _hdr_file = self.header_file()?;
        if let Some(f) = &_hdr_file {
            cmd.args(["-H", &format!("@{}", f.0.display())]);
        }
        let mut child = cmd
            .args([&url, "-d", "@-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("curl is required for openai-compatible providers")?;
        child
            .stdin
            .as_mut()
            .context("provider stdin unavailable")?
            .write_all(body.to_string().as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            anyhow::bail!(
                "provider '{}' http failed: {}",
                self.name,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let v: serde_json::Value = serde_json::from_slice(&out.stdout)
            .with_context(|| format!("provider '{}' returned non-JSON", self.name))?;
        let text = v
            .pointer("/choices/0/message/content")
            .and_then(|c| c.as_str())
            .with_context(|| format!("provider '{}' returned no completion", self.name))?;
        Ok(ForageResponse {
            text: text.to_string(),
            provider: self.name.clone(),
        })
    }
}

/// A 0600 file holding `authorization: Bearer …` for `curl -H @file`;
/// deleted on drop. Keeps the token out of the process list.
struct HeaderFile(std::path::PathBuf);
impl HeaderFile {
    fn new(key: &str) -> Result<HeaderFile> {
        let dir = std::env::temp_dir().join(format!("nest-hdr-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("header");
        {
            #[cfg(unix)]
            use std::os::unix::fs::OpenOptionsExt;
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            opts.mode(0o600);
            let mut f = opts.open(&path)?;
            f.write_all(format!("authorization: Bearer {key}\n").as_bytes())?;
        }
        Ok(HeaderFile(path))
    }
}
impl Drop for HeaderFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_dir(self.0.parent().unwrap());
    }
}

impl OpenAiForage {
    fn header_file(&self) -> Result<Option<HeaderFile>> {
        match &self.api_key {
            Some(k) => Ok(Some(HeaderFile::new(k)?)),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------
// The free-first router
// ---------------------------------------------------------------------------

use crate::spec::ProviderSpec;

/// Cost rank for ordering — free providers always answer first.
fn cost_rank(cost: &str) -> u8 {
    match cost {
        "free" => 0,
        "cheap" => 1,
        _ => 2,
    }
}

/// Is the provider callable right now? Command kinds need their binary
/// on PATH; openai kinds need a base_url, curl, and (if declared) the
/// key env var set.
fn provider_available(spec: &ProviderSpec) -> bool {
    if spec.kind == "openai" {
        if spec.base_url.is_none() {
            return false;
        }
        if let Some(var) = &spec.api_key_env {
            if std::env::var(var).unwrap_or_default().is_empty() {
                return false;
            }
        }
        return crate::perch::which("curl").is_some();
    }
    let bin = match spec.kind.as_str() {
        "ollama" => "ollama",
        _ => spec.program.as_deref().unwrap_or(""),
    };
    if bin.is_empty() {
        return false;
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| crate::perch::is_exec_pub(&d.join(bin))))
        .unwrap_or(false)
}

/// Turn a spec into a callable provider.
fn provider_forage(spec: &ProviderSpec) -> Option<Box<dyn Forage>> {
    match spec.kind.as_str() {
        "ollama" => Some(Box::new(CommandForage {
            name: spec.name.clone(),
            program: "ollama".into(),
            args: vec!["run".into(), spec.model.clone().unwrap_or_default()],
            env: spec.env.clone(),
        })),
        "openai" => Some(Box::new(OpenAiForage {
            name: spec.name.clone(),
            base_url: spec.base_url.clone()?,
            model: spec.model.clone().unwrap_or_default(),
            api_key: spec
                .api_key_env
                .as_ref()
                .and_then(|v| std::env::var(v).ok())
                .filter(|k| !k.is_empty()),
        })),
        _ => Some(Box::new(CommandForage {
            name: spec.name.clone(),
            program: spec.program.clone()?,
            args: spec.args.clone(),
            env: spec.env.clone(),
        })),
    }
}

/// One row of the providers table for the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub kind: String,
    pub cost: String,
    pub model: Option<String>,
    pub available: bool,
    /// CodexBar-style: "12/50 · resets 21:04 UTC" — absent for unlimited.
    pub usage: Option<String>,
    /// true when this window's limit is spent — the router skips it.
    pub exhausted: bool,
}

/// The forage router: providers declared in `providers.toml`, ordered
/// free → cheap → paid, skipping anything not installed or over its
/// limit window. `complete` walks the order and the first provider that
/// answers wins — failover is the whole point.
pub struct Router {
    providers: Vec<ProviderSpec>,
    /// nest home — usage counters live in <home>/usage/. None (tests)
    /// disables tracking.
    home: Option<std::path::PathBuf>,
    /// Fallback when nothing is declared or available — never errors.
    echo: EchoForage,
}

impl Router {
    pub fn new(mut providers: Vec<ProviderSpec>) -> Self {
        providers.sort_by_key(|p| cost_rank(&p.cost));
        Self {
            providers,
            home: None,
            echo: EchoForage,
        }
    }

    /// Load the router from the nest's providers.toml files.
    pub fn from_dirs(dirs: &crate::home::NestDirs) -> Self {
        let (providers, _) = crate::spec::load_providers(&dirs.provider_files());
        let mut r = Self::new(providers);
        r.home = Some(dirs.home.clone());
        r
    }

    fn usage(&self, spec: &ProviderSpec) -> Option<crate::usage::Usage> {
        self.home
            .as_deref()
            .and_then(|h| crate::usage::status(h, spec))
    }

    /// Provider rows for `nest providers` / the shell: the order shown is
    /// the order the router tries them.
    pub fn list(&self) -> Vec<ProviderInfo> {
        self.providers
            .iter()
            .map(|p| {
                let u = self.usage(p);
                ProviderInfo {
                    name: p.name.clone(),
                    kind: p.kind.clone(),
                    cost: p.cost.clone(),
                    model: p.model.clone(),
                    available: provider_available(p),
                    usage: u.as_ref().map(|u| {
                        let resets = chrono_free_time(u.resets_at);
                        format!("{}/{} · resets {}", u.used, u.limit, resets)
                    }),
                    exhausted: u.as_ref().is_some_and(|u| u.exhausted),
                }
            })
            .collect()
    }

    /// Complete with failover: try each installed, unspent provider in
    /// cost order; the echo stub answers when nothing real is available
    /// so the loop never dead-ends on a noob machine.
    pub fn complete(&self, req: &ForageRequest) -> Result<ForageResponse> {
        let mut errors = Vec::new();
        let candidates: Vec<&ProviderSpec> = match req.model.as_deref() {
            Some(want) => {
                // forced by name — must exist and be installed
                match self.providers.iter().find(|p| p.name == want) {
                    Some(p) if !provider_available(p) => {
                        anyhow::bail!("provider '{want}' is not installed")
                    }
                    Some(p) => match self.usage(p) {
                        Some(u) if u.exhausted => anyhow::bail!(
                            "provider '{want}' is over its limit — resets {}",
                            chrono_free_time(u.resets_at)
                        ),
                        _ => vec![p],
                    },
                    None => anyhow::bail!("unknown provider '{want}'"),
                }
            }
            None => self
                .providers
                .iter()
                .filter(|p| provider_available(p))
                .filter(|p| self.usage(p).is_none_or(|u| !u.exhausted))
                .collect(),
        };
        for spec in candidates {
            // reserve the window slot before spending it — atomic across
            // processes; a counter error is a provider failure, never a
            // free pass past the cap
            if let Some(h) = &self.home {
                match crate::usage::try_reserve(h, spec) {
                    Ok(true) => {}
                    Ok(false) => {
                        errors.push(format!("{}: limit window spent", spec.name));
                        continue;
                    }
                    Err(e) => {
                        errors.push(format!("{}: {e:#}", spec.name));
                        continue;
                    }
                }
            }
            let Some(forage) = provider_forage(spec) else {
                continue;
            };
            match forage.complete(req) {
                Ok(mut res) => {
                    res.provider = format!("{} ({})", res.provider, spec.cost);
                    return Ok(res);
                }
                Err(e) => errors.push(format!("{}: {e:#}", spec.name)),
            }
        }
        if req.model.is_some() {
            anyhow::bail!("provider failed: {}", errors.join("; "));
        }
        // nothing available (or everything failed) — echo keeps the loop
        // running on a machine with zero providers
        self.echo.complete(req)
    }
}

/// unix seconds → "HH:MM UTC" — chrono-free, good enough for a reset hint.
fn chrono_free_time(ts: u64) -> String {
    let secs = ts % 86400;
    format!("{:02}:{:02} UTC", secs / 3600, (secs % 3600) / 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::ProviderSpec;

    fn prov(name: &str, cost: &str, program: &str, args: &[&str]) -> ProviderSpec {
        ProviderSpec {
            name: name.into(),
            kind: "command".into(),
            program: Some(program.into()),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: Default::default(),
            model: None,
            cost: cost.into(),
            base_url: None,
            api_key_env: None,
            limit: None,
            window: None,
        }
    }

    fn req(model: Option<&str>) -> ForageRequest {
        ForageRequest {
            prompt: "hello".into(),
            model: model.map(|s| s.into()),
            tier: None,
        }
    }

    #[test]
    fn free_providers_answer_before_paid() {
        // `cat` is a real provider on PATH — echoes stdin back
        let router = Router::new(vec![
            prov("paid-bot", "paid", "definitely-missing-bin", &[]),
            prov("free-cat", "free", "cat", &[]),
            prov("cheap-bot", "cheap", "cat", &[]),
        ]);
        let order: Vec<String> = router.list().iter().map(|p| p.name.clone()).collect();
        assert_eq!(order, ["free-cat", "cheap-bot", "paid-bot"]);
        let res = router.complete(&req(None)).unwrap();
        assert_eq!(res.provider, "free-cat (free)");
        assert_eq!(res.text, "hello");
    }

    #[test]
    fn failover_walks_past_broken_providers() {
        // free provider exists but exits non-zero → router falls to cheap
        let router = Router::new(vec![
            prov("free-broken", "free", "sh", &["-c", "exit 1"]),
            prov("cheap-ok", "cheap", "cat", &[]),
        ]);
        let res = router.complete(&req(None)).unwrap();
        assert_eq!(res.provider, "cheap-ok (cheap)");
    }

    #[test]
    fn forced_provider_must_exist_and_be_installed() {
        let router = Router::new(vec![prov("free-cat", "free", "cat", &[])]);
        assert!(router.complete(&req(Some("nope"))).is_err());
        assert!(router.complete(&req(Some("free-cat"))).is_ok());
        let missing = Router::new(vec![prov("ghost", "free", "definitely-missing-bin", &[])]);
        let err = format!("{:#}", missing.complete(&req(Some("ghost"))).unwrap_err());
        assert!(err.contains("not installed"), "{err}");
    }

    #[test]
    fn echo_when_nothing_declared() {
        let router = Router::new(vec![]);
        let res = router.complete(&req(None)).unwrap();
        assert_eq!(res.provider, "echo");
    }
}
