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

/// Does this provider's backing binary exist on PATH?
fn provider_available(spec: &ProviderSpec) -> bool {
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
fn provider_forage(spec: &ProviderSpec) -> Option<CommandForage> {
    match spec.kind.as_str() {
        "ollama" => Some(CommandForage {
            name: spec.name.clone(),
            program: "ollama".into(),
            args: vec!["run".into(), spec.model.clone().unwrap_or_default()],
            env: spec.env.clone(),
        }),
        _ => Some(CommandForage {
            name: spec.name.clone(),
            program: spec.program.clone()?,
            args: spec.args.clone(),
            env: spec.env.clone(),
        }),
    }
}

/// One row of the providers table for the UI.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderInfo {
    pub name: String,
    pub kind: String,
    pub cost: String,
    pub available: bool,
}

/// The forage router: providers declared in `providers.toml`, ordered
/// free → cheap → paid, skipping anything not installed. `complete`
/// walks the order and the first provider that answers wins — failover
/// is the whole point.
pub struct Router {
    providers: Vec<ProviderSpec>,
    /// Fallback when nothing is declared or available — never errors.
    echo: EchoForage,
}

impl Router {
    pub fn new(mut providers: Vec<ProviderSpec>) -> Self {
        providers.sort_by_key(|p| cost_rank(&p.cost));
        Self {
            providers,
            echo: EchoForage,
        }
    }

    /// Load the router from the nest's providers.toml files.
    pub fn from_dirs(dirs: &crate::home::NestDirs) -> Self {
        let (providers, _) = crate::spec::load_providers(&dirs.provider_files());
        Self::new(providers)
    }

    /// Provider rows for `nest providers` / the shell: the order shown is
    /// the order the router tries them.
    pub fn list(&self) -> Vec<ProviderInfo> {
        self.providers
            .iter()
            .map(|p| ProviderInfo {
                name: p.name.clone(),
                kind: p.kind.clone(),
                cost: p.cost.clone(),
                available: provider_available(p),
            })
            .collect()
    }

    /// Complete with failover: try each installed provider in cost order;
    /// the echo stub answers when nothing real is available so the loop
    /// never dead-ends on a noob machine.
    pub fn complete(&self, req: &ForageRequest) -> Result<ForageResponse> {
        let mut errors = Vec::new();
        let candidates: Vec<&ProviderSpec> = match req.model.as_deref() {
            Some(want) => {
                // forced by name — must exist and be installed
                match self.providers.iter().find(|p| p.name == want) {
                    Some(p) if provider_available(p) => vec![p],
                    Some(_) => anyhow::bail!("provider '{want}' is not installed"),
                    None => anyhow::bail!("unknown provider '{want}'"),
                }
            }
            None => self
                .providers
                .iter()
                .filter(|p| provider_available(p))
                .collect(),
        };
        for spec in candidates {
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
