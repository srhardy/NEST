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
