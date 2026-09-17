use std::path::{Path, PathBuf};

use spoke_core::ir::{Capabilities, Op, Plan};

use crate::{codex_mcp_table, Ctx, Harness};

pub struct Codex;

impl Harness for Codex {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn display(&self) -> &'static str {
        "OpenAI Codex CLI"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            instructions: true,
            skills: true,
            commands: true,
            hooks: true,
            mcp: true,
            permissions: true,
            env: false,
        }
    }

    fn root(&self, home: &Path) -> PathBuf {
        home.join(".codex")
    }

    fn plan(&self, ctx: &Ctx<'_>) -> Plan {
        let mut plan = Plan::default();

        // AGENTS.md is Codex's native instruction file — and Claude imports it, so it is
        // the canonical location for project instructions.
        let instructions = ctx.instruction_lines();
        if instructions.trim().is_empty() {
            plan.warn("no instructions declared in agent.toml — skipping AGENTS.md");
        } else {
            plan.push(Op::WriteBlock {
                path: ctx.project.join("AGENTS.md"),
                id: "instructions".to_string(),
                contents: instructions,
                why: "project instructions (canonical)".to_string(),
            });
        }

        for server in &ctx.manifest.mcp {
            let table = codex_mcp_table(server);
            if table.is_empty() {
                plan.warn(format!(
                    "mcp server '{}' has neither command nor url — skipped",
                    server.name
                ));
                continue;
            }
            plan.push(Op::MergeToml {
                path: ctx.project.join(".codex").join("config.toml"),
                pointer: vec!["mcp_servers".to_string(), server.name.clone()],
                table_toml: table,
                why: format!("register MCP server '{}'", server.name),
            });
        }

        plan.warn(
            "telemetry-off environment variables have no Codex equivalent — use `spoke wrap` (budget spoke, M2)",
        );
        if !ctx.manifest.skills.is_empty() {
            plan.warn(format!(
                "{} skill package(s) declared — installation arrives with the packages spoke (M1)",
                ctx.manifest.skills.len()
            ));
        }
        plan.warn("hooks are not projected yet (budget spoke, M2); permission rules arrive with the policy spoke (M3)");
        plan
    }
}
