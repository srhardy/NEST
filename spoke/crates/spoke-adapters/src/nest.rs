use std::path::{Path, PathBuf};

use spoke_core::ir::{Capabilities, Op, Plan};

use crate::{claude_mcp_entry, Ctx, Harness};

/// Nest — the noob-first desktop AI app (github.com/srhardy/NEST).
///
/// Nest's config surface lives in `.nest/`, evolving the proven extension
/// formats verified in the Command Code seed engine:
/// - `AGENTS.md` is the project memory file (cross-tool convention, root).
/// - Skills are `<root>/skills/<name>/SKILL.md` directories
///   (`~/.nest/skills` personal, `.nest/skills` project).
/// - MCP servers are `mcpServers` entries in `.nest/mcp.json`, accepting
///   `type`/`transport` of `stdio|http|sse` — the Claude-shaped entry is
///   valid verbatim.
/// - Agents (`.nest/agents/<name>.md`), cron, trusted-hooks and mods exist
///   in the design but are not projected yet — surfaced as capability gaps.
pub struct Nest;

impl Harness for Nest {
    fn id(&self) -> &'static str {
        "nest"
    }

    fn display(&self) -> &'static str {
        "Nest"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            instructions: true,
            skills: true,
            commands: false,
            hooks: false,
            mcp: true,
            permissions: false,
            env: false,
        }
    }

    fn root(&self, home: &Path) -> PathBuf {
        home.join(".nest")
    }

    fn plan(&self, ctx: &Ctx<'_>) -> Plan {
        let mut plan = Plan::default();

        // AGENTS.md is the cross-tool project-memory convention.
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
            let value = claude_mcp_entry(server);
            if value.is_null() {
                plan.warn(format!(
                    "mcp server '{}' has neither command nor url — skipped",
                    server.name
                ));
                continue;
            }
            plan.push(Op::MergeJson {
                path: ctx.project.join(".nest/mcp.json"),
                pointer: vec!["mcpServers".to_string(), server.name.clone()],
                value,
                why: format!("register MCP server '{}'", server.name),
            });
        }

        if !ctx.manifest.skills.is_empty() {
            plan.warn(format!(
                "{} skill package(s) declared — installation arrives with the packages spoke (M1)",
                ctx.manifest.skills.len()
            ));
        }
        plan.warn("trusted-hooks, permissions, mods, and cron are not projected for Nest yet");
        plan
    }
}
