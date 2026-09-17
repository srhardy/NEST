use std::path::{Path, PathBuf};

use serde::Serialize;
use spoke_core::ir::{Capabilities, Op, Plan};
use spoke_core::manifest::{Automation, Bot, BotMode, Route, Service, ToolPolicy};

use crate::{claude_mcp_entry, toml_string, Ctx, Harness};

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
/// - `[[bots]]` are generated agent files in `.nest/agents/<name>.md`
///   (YAML frontmatter + persona body, fully Spoke-owned).
/// - `[[automations]]` are generated trigger tables in
///   `.nest/automations.toml` (fully Spoke-owned).
/// - `[[routing]]` (the Boss's dispatch table) and `[[services]]` render to
///   `.nest/routing.toml` / `.nest/services.toml` (fully Spoke-owned).
/// - Trusted-hooks and mods exist in the design but are not projected
///   yet — surfaced as capability gaps.
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

        // Bots -> generated agent files. Spoke owns these files outright:
        // the manifest is the truth, so edits must happen there.
        if !ctx.manifest.bots.is_empty() {
            let declared_skills: Vec<&str> = ctx
                .manifest
                .skills
                .iter()
                .map(|s| s.name.as_str())
                .collect();
            let declared_mcp: Vec<&str> =
                ctx.manifest.mcp.iter().map(|s| s.name.as_str()).collect();
            let mut files = Vec::new();
            for bot in &ctx.manifest.bots {
                for skill in &bot.skills {
                    if !declared_skills.contains(&skill.as_str()) {
                        plan.warn(format!(
                            "bot '{}' references skill '{skill}' — not declared in agent.toml",
                            bot.name
                        ));
                    }
                }
                for server in &bot.mcp {
                    if !declared_mcp.contains(&server.as_str()) {
                        plan.warn(format!(
                            "bot '{}' references mcp server '{server}' — not declared in agent.toml",
                            bot.name
                        ));
                    }
                }
                files.push((format!("{}.md", bot.name), render_bot_md(bot)));
            }
            plan.push(Op::WriteFiles {
                root: ctx.project.join(".nest"),
                name: "agents".to_string(),
                files,
                why: format!(
                    "render {} bot(s) into .nest/agents/",
                    ctx.manifest.bots.len()
                ),
            });
            plan.warn(
                "removing a [[bots]] entry does not delete its .nest/agents/<name>.md yet — \
                 delete the file by hand for now"
                    .to_string(),
            );
        }

        // Automations -> a single generated TOML table file.
        if !ctx.manifest.automations.is_empty() {
            let bot_names: Vec<&str> = ctx.manifest.bots.iter().map(|b| b.name.as_str()).collect();
            for automation in &ctx.manifest.automations {
                if let Some(bot) = &automation.bot {
                    if !bot_names.contains(&bot.as_str()) {
                        plan.warn(format!(
                            "automation '{}' runs as bot '{bot}' — not declared in [[bots]]",
                            automation.name
                        ));
                    }
                }
            }
            plan.push(Op::WriteFiles {
                root: ctx.project.to_path_buf(),
                name: ".nest".to_string(),
                files: vec![(
                    "automations.toml".to_string(),
                    render_automations_toml(&ctx.manifest.automations),
                )],
                why: format!(
                    "render {} automation(s) into .nest/automations.toml",
                    ctx.manifest.automations.len()
                ),
            });
        }

        // Routing -> .nest/routing.toml: the Boss's dispatch table.
        // Dangling bot refs warn but are never dropped.
        if !ctx.manifest.routing.is_empty() {
            let bot_names: Vec<&str> = ctx.manifest.bots.iter().map(|b| b.name.as_str()).collect();
            for route in &ctx.manifest.routing {
                if !bot_names.contains(&route.bot.as_str()) {
                    plan.warn(format!(
                        "route targets bot '{}' — not declared in [[bots]]",
                        route.bot
                    ));
                }
                for fallback in &route.fallback {
                    if !bot_names.contains(&fallback.as_str()) {
                        plan.warn(format!(
                            "route -> '{}' has fallback '{fallback}' — not declared in [[bots]]",
                            route.bot
                        ));
                    }
                }
            }
            plan.push(Op::WriteFiles {
                root: ctx.project.to_path_buf(),
                name: ".nest".to_string(),
                files: vec![(
                    "routing.toml".to_string(),
                    render_routing_toml(&ctx.manifest.routing),
                )],
                why: format!(
                    "render {} route(s) into .nest/routing.toml",
                    ctx.manifest.routing.len()
                ),
            });
        }

        // Services -> .nest/services.toml: declared daemons the app can
        // start and health-check (orchestrate.sh as data).
        if !ctx.manifest.services.is_empty() {
            plan.push(Op::WriteFiles {
                root: ctx.project.to_path_buf(),
                name: ".nest".to_string(),
                files: vec![(
                    "services.toml".to_string(),
                    render_services_toml(&ctx.manifest.services),
                )],
                why: format!(
                    "render {} service(s) into .nest/services.toml",
                    ctx.manifest.services.len()
                ),
            });
        }

        if !ctx.manifest.skills.is_empty() {
            plan.warn(format!(
                "{} skill package(s) declared — installation arrives with the packages spoke (M1)",
                ctx.manifest.skills.len()
            ));
        }
        plan.warn("trusted-hooks and mods are not projected for Nest yet");
        plan
    }
}

/// Bot -> `.nest/agents/<name>.md`: YAML frontmatter + persona body.
/// Fully generated; the header comment says where the truth lives.
fn render_bot_md(bot: &Bot) -> String {
    let title = bot.title.clone().unwrap_or_else(|| bot.name.clone());
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", bot.name));
    out.push_str(&format!("title: {}\n", toml_string(&title)));
    out.push_str(&format!("job: {}\n", toml_string(&bot.job)));
    if let Some(model) = &bot.model {
        out.push_str(&format!("model: {}\n", toml_string(model)));
    }
    if let Some(mode) = bot.mode {
        let mode = match mode {
            BotMode::Primary => "primary",
            BotMode::Subagent => "subagent",
        };
        out.push_str(&format!("mode: {mode}\n"));
    }
    if bot.boss {
        out.push_str("boss: true\n");
    }
    if let Some(steps) = bot.max_steps {
        out.push_str(&format!("max_steps: {steps}\n"));
    }
    if !bot.permissions.is_empty() {
        out.push_str("permissions:\n");
        for (tool, policy) in &bot.permissions {
            let policy = match policy {
                ToolPolicy::Allow => "allow",
                ToolPolicy::Ask => "ask",
                ToolPolicy::Deny => "deny",
            };
            out.push_str(&format!("  {}: {policy}\n", toml_string(tool)));
        }
    }
    if !bot.skills.is_empty() {
        out.push_str(&format!(
            "skills: [{}]\n",
            bot.skills
                .iter()
                .map(|s| toml_string(s))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !bot.mcp.is_empty() {
        out.push_str(&format!(
            "mcp: [{}]\n",
            bot.mcp
                .iter()
                .map(|s| toml_string(s))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str("---\n\n");
    out.push_str(
        "<!-- generated by spoke from agent.toml — edit the manifest, not this file -->\n\n",
    );
    out.push_str(&format!("# {title}\n"));
    if !bot.voice.is_empty() {
        out.push('\n');
        for line in &bot.voice {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !bot.anti_jobs.is_empty() {
        out.push_str("\n## Not my job\n\n");
        for anti in &bot.anti_jobs {
            out.push_str(&format!("- {anti}\n"));
        }
    }
    if !bot.memory.is_empty() {
        out.push_str("\n## Memory\n\n");
        for note in &bot.memory {
            out.push_str(&format!("- {note}\n"));
        }
    }
    out
}

/// Automations -> `.nest/automations.toml` as `[[automation]]` tables.
fn render_automations_toml(automations: &[Automation]) -> String {
    #[derive(Serialize)]
    struct File<'a> {
        automation: &'a [Automation],
    }
    let body = toml::to_string(&File {
        automation: automations,
    })
    .unwrap_or_else(|_| String::new());
    let mut out =
        String::from("# generated by spoke from agent.toml — edit the manifest, not this file\n");
    out.push_str(&body);
    out
}

/// Routes -> `.nest/routing.toml` as `[[route]]` tables.
fn render_routing_toml(routes: &[Route]) -> String {
    #[derive(Serialize)]
    struct File<'a> {
        route: &'a [Route],
    }
    let body = toml::to_string(&File { route: routes }).unwrap_or_else(|_| String::new());
    let mut out =
        String::from("# generated by spoke from agent.toml — edit the manifest, not this file\n");
    out.push_str(&body);
    out
}

/// Services -> `.nest/services.toml` as `[[service]]` tables.
fn render_services_toml(services: &[Service]) -> String {
    #[derive(Serialize)]
    struct File<'a> {
        service: &'a [Service],
    }
    let body = toml::to_string(&File { service: services }).unwrap_or_else(|_| String::new());
    let mut out =
        String::from("# generated by spoke from agent.toml — edit the manifest, not this file\n");
    out.push_str(&body);
    out
}
