use std::path::{Path, PathBuf};

use spoke_core::ir::{Capabilities, Op, Plan};

use crate::{claude_mcp_entry, Ctx, Harness, TELEMETRY_ENV};

pub struct ClaudeCode;

impl Harness for ClaudeCode {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn display(&self) -> &'static str {
        "Claude Code"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            instructions: true,
            skills: true,
            commands: true,
            hooks: true,
            mcp: true,
            permissions: true,
            env: true,
        }
    }

    fn root(&self, home: &Path) -> PathBuf {
        home.join(".claude")
    }

    fn plan(&self, ctx: &Ctx<'_>) -> Plan {
        let mut plan = Plan::default();

        // Instruction bridge: canonical text lives in AGENTS.md; Claude imports it.
        let instructions = ctx.instruction_lines();
        if instructions.trim().is_empty() {
            plan.warn("no instructions declared in agent.toml — skipping the CLAUDE.md bridge");
        } else {
            let mut body = instructions;
            body.push_str("@AGENTS.md\n");
            plan.push(Op::WriteBlock {
                path: ctx.project.join("CLAUDE.md"),
                id: "instructions".to_string(),
                contents: body,
                why: "project instructions (bridged to AGENTS.md)".to_string(),
            });
        }

        // MCP servers, project-scoped so the config is reviewable and shareable.
        for server in &ctx.manifest.mcp {
            let value = claude_mcp_entry(server);
            if value.is_null() {
                plan.warn(format!(
                    "mcp server '{}' has neither command nor url — skipped",
                    server.name
                ));
                continue;
            }
            plan.push(Op::MergeJson {
                path: ctx.project.join(".mcp.json"),
                pointer: vec!["mcpServers".to_string(), server.name.clone()],
                value,
                why: format!("register MCP server '{}'", server.name),
            });
        }

        // Telemetry beacons off (per-key merge so existing env entries survive).
        for (key, value) in TELEMETRY_ENV {
            plan.push(Op::MergeJson {
                path: ctx.project.join(".claude").join("settings.json"),
                pointer: vec!["env".to_string(), (*key).to_string()],
                value: serde_json::Value::String((*value).to_string()),
                why: "disable third-party telemetry beacons".to_string(),
            });
        }

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
