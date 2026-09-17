# Fork survey + BobTheBuilder deep-dive — what NEST can borrow

Survey of all 142 repos under `srhardy/*` (140 forks + 2 owned), run at current
upstream (all forks synced first — see §1). The question: where are the gems
worth folding into NEST's design?

---

## 1. Fork freshness — what "updated" actually meant

Inventory method: `installation/repositories` API → per-repo parent resolution
→ upstream compare (`behind_by`) → `POST /repos/srhardy/<fork>/merge-upstream`
(the exact API behind GitHub's "Sync fork" button — non-destructive, conflicts
fail cleanly).

| Result | Count | Detail |
|---|---|---|
| Already current | 24 | incl. llm-council, create-t3-app, pi-acp, gauntlet-loop |
| **Synced this session** | **109** | upstream merged into your forks (fast-forward or merge) |
| Merge conflicts | 4 | `ghost-pepper`, `graphify`, `ComfyUI-Easy-Install`, `openwork` — your forks have local changes touching the same lines; need a manual merge choice, not forced |
| Diverged (no common ancestor) | 3 | `membrane`, `accomplish`, `skills` — the fork was likely re-created/re-imported, not a live fork; "sync" isn't meaningful — re-fork if you want current |

Worst staleness before sync: `hermes-agent` 33,274 behind, `openhuman` 18,748,
`oh-my-openagent` 11,614, `autumn` 8,962, `oh-my-pi` 6,576, `vllm` 5,928,
`remotion` 5,623, `SurfSense` 5,577, `NemoClaw` 5,554. All now current.

> Ongoing: forks drift again. A future `[[automations]]` run could literally be
> "spoke sync-forks" weekly — NEST eating its own cooking.

---

## 2. BobTheBuilder — the Boss pattern (deep-dive)

`srhardy/BobTheBuilder` is not an app — it's a **complete Boss→Assistant→Council
configuration pack** for Kilo Code + Gemini CLI, and it is the single most
NEST-shaped thing in the account. Contents: `agents/` (8 consolidated agents),
`instructions/` (workspace policy, security rules, cost policy, auto-detection),
`skills/` (6 SKILL.md dirs incl. legal suite, llm-council, firecrawl, surfsense,
autonomous-bot, copilot), `workflows/` (8 named flows), `assistant/` (kilo.json +
orchestration scripts + master AGENTS.md).

### What maps onto what (the fit is uncanny)

| BobTheBuilder | NEST today / BIG-PLAN slot |
|---|---|
| `agents/*.md` w/ YAML frontmatter (`name`, `description`, `mode: subagent`, `steps`, `permission: {bash: ask, edit: ask, read: allow…}`) | `[[bots]]` → `.nest/agents/<name>.md` — same shape already shipped; **his `permission:` map is the missing piece** — per-tool allow/ask deserves a `permissions` field on `[[bots]]` |
| Boss → Senior Assistant → Council hierarchy | The boss-bot / orchestrator pattern: a bot whose job is delegating to other bots. Add `routes`/`delegates` to the bot schema or a `boss` flag |
| `auto-detection.md` routing table (keywords → agent/skill + escalation ladder: fail twice → LLM Council → Copilot fallback) | A `[[routing]]` section or `.nest/routing.toml` — intent→bot dispatch + fallback chains |
| `orchestrate.sh` — health-checked service table (OpenMemory :8765, Firecrawl :3002, Agent Zero :50001, Council :8001) | `[[services]]`/`spoke services` — declared external deps the app starts/health-checks |
| Deny-by-default security rules + allowed-domain list | The egress allowlist per bot (BIG-PLAN §7.4 already calls for it) |
| `cost-policy.md` zero-cost-first | Already shipped: `budget.preset` + `bots[].model` tier |
| Skills with `triggers:` in SKILL.md frontmatter | Trigger-scoped invocation — Spoke package skills can carry this verbatim |
| `workflows/*.md` named flows (remember, orchestrate, model-status) | Slash-style invocations → `[[automations]]` `event = "command:<name>"` or skill commands |

### The real gem: it *is* the roster NEST needs to seed

BobTheBuilder's 8 agents (planner/executor/debugger/verifier/researcher/
codebase-mapper/ui-auditor/assumptions-analyzer) are a ready-made starter
council. As a `package.toml` with `[[agents]]` entries it installs into
`~/.nest/agents/` via the package machinery just shipped in PR #2 — NEST's
first real bot pack, authored by the user himself. The personas already encode
the discipline BIG-PLAN wants (atomic tasks, verify steps, escalation).

---

## 3. Gems across the 140 forks (grouped by NEST subsystem)

Nothing folded in yet — this is the map of where the value sits.

### Boss/orchestration (Bob's pattern, other voices)
- **multica-ai/multica** — agents as board teammates: pick up issues, report
  progress, raise blockers. *Gem:* the work-queue UX — bots on a board,
  not just a chat roster.
- **openai/symphony** — issue-board → isolated autonomous runs → proof-of-work
  (CI status, PR feedback, walkthrough videos). *Gem:* "proof of work" as the
  automation output contract — a NEST automation should report what it *did*,
  not just that it ran.
- **traycerai/traycer** — BYOA orchestration, shared context across providers,
  agent-to-agent messaging. *Gem:* unified context across model switch —
  bot keeps memory while model changes (the `model:` field shouldn't lose
  `memory:`).
- **disler/fusion-harness + duolahypercho/fusion-fable** — multi-model fusion:
  N-way opinions → fused answer. *Gem:* council-style synthesis as a bot
  capability (pairs with llm-council).
- **agent0ai/agent-zero** — full-Linux-desktop autonomous agent w/ plugin hub,
  projects+memory+secrets isolation, multi-agent delegation. *Gem:* the
  project-scoped isolation model and the host-bridge CLI pattern.
- **Git-on-my-level/agentctl** — local control layer for coding-agent CLIs:
  lifecycle metadata, bounded events, retrievable results, no control plane.
  *Gem:* the thinnest honest answer to "how does NEST spawn Claude/Codex/Pi"
  — wrap, don't reimplement.
- **earendil-works/pi (pi-mono)** — Pi agent harness: coding-agent CLI +
  agent-core runtime + unified multi-provider `pi-ai` API. *Gem:* a clean
  provider-abstraction layer reference (TS) if NEST starts thin-engine.
- **stephengpope/thepopebot** — three-doors-one-brain: browser chat +
  Telegram + live terminal all hit one event handler that drives coding
  agents, opens PRs, DMs when done. *Gem:* the multi-channel single-brain
  routing — NEST tabs + future mobile/Telegram as just "doors".
- **openai/codex + anomalyco/opencode + Gitlawb/openclaude** — the three big
  open agent runtimes. *Gems:* harness compatibility is already why Spoke
  exists; `openclaude` ("runs anywhere, uses anything") is a provider-agnostic
  Claude-runtime worth mining.
- **hotovo/aider-desk** — "transparent, steerable AI orchestration" desktop
  app for aider. *Gem:* mature desktop-agent UX patterns.
- **different-ai/openwork** — open-source Cowork/Codex desktop alternative
  (⚠ fork has merge conflicts — local changes diverged).
- **NVIDIA/NemoClaw** — sandboxed agent reference stack: guided onboarding,
  managed inference, network policy, snapshots. *Gem:* network-policy +
  snapshot lifecycle vocabulary for bot sandboxes.
- **volcengine/OpenViking** — "context database": agents treat knowledge as a
  `viking://` filesystem with ls/tree/read/write. *Gem:* memory-as-filesystem
  is a beautifully noob-legible metaphor (fits NEST's file-philosophy).
- **can1357/oh-my-pi + code-yeongyu/oh-my-openagent** — big agent harnesses
  with DAG workflows + memory. *Gems:* orchestration DAGs, memory systems.
- **tinyhumansai/openhuman** — agent harness w/ local-first memory.
- **ShenSeanChen/waku-agent** — deliberately legible local agent:
  ~95-line loop, SQLite memory, watch-it-think dashboard, built-in eval gate.
  *Gem:* "readable in an afternoon" is exactly NEST's noob ethos — the
  dashboard-that-shows-the-loop is a UI gem.
- **gobii-ai/gobii-platform** — "AI employees": durable always-on agents with
  schedules, events, real browsers, cross-agent coordination, reachable by
  email/text. *Gem:* bots-as-employees framing (assign identity, message it).
- **PrimeIntellect-ai/prime-agent** — self-improving RLM harness. *Gem:*
  skill self-improvement loop concept (see hermes-agent).
- **NousResearch/hermes-agent** — self-improving agent: creates skills from
  experience, improves them during use, session search w/ summarization,
  user modeling, runs on $5 VPS, Telegram/Discord/Slack/WhatsApp/Signal from
  one gateway. *Gem:* **the learning loop** — bots that write their own
  skills (Spoke packages authored by bots!) — biggest idea on the list.
- **turnstonelabs/turnstone** — local-first orchestration, agents across your
  own machines (the "cluster of my laptops" answer to headless runs).
- **accomplish-ai/coworker** — the desktop AI coworker (⚠ archived upstream —
  ideas only).
- **itseffi/agentic-os** — personal OS automating workflows across
  Codex/Claude/Pi/OpenClaw agents.

### Memory (bot `memory:` substrate)
- **mem0ai/mem0** — the memory layer BobTheBuilder already runs via Docker
  (OpenMemory :8765). *Gem:* NEST's default memory service — already proven
  in Steven's own stack.
- **MemPalace/mempalace** — verbatim local-first memory w/ palaces structure,
  96.6% LongMemEval, zero API calls. *Gem:* verbatim-not-summarized storage.
- **scrypster/muninndb** — cognitive DB: Ebbinghaus decay, Hebbian learning,
  Bayesian confidence, MCP-native, single binary. *Gem:* memories that fade
  unless used — gorgeous fit for living bots (but BSL license — idea, not code).
- **BennettSchwartz/membrane** — typed/revisable/decayable memory w/
  trust-aware retrieval. (Fork diverged — read upstream directly.)
- **TencentCloud/TencentDB-Agent-Memory** — another agent-memory service.
- **svange/openbrain + NateBJones-Projects/OB1** — "one database, one AI
  gateway, one chat channel — any AI plugs in."

### Bot building blocks / skills corpus
- **msitarzewski/agency-agents** — a full agency of personality'd specialist
  agents (frontend wizard, reality checker, Reddit ninja). *Gem:* the
  richest ready-made roster source — pairs with BobTheBuilder's 8 to seed
  the NEST template catalog.
- **davidondrej/skills** — personal agent skills collection (diverged fork).
- **zubair-trabzada/ai-legal-claude** — 14-capability legal skill suite;
  BobTheBuilder already ships a `legal` skill derived from this world.
- **google-labs-code/stitch-skills** — agent skills.
- **karpathy-llm-wiki + awesome-cursorrules + awesome-design-md +
  awesome-coderabbit** — skills/rules corpora → NEST template packs.
- **loopy (Forward-Future)** — a library of agent loops + a skill that finds/
  adapts/designs repeatable agent workflows. *Gem:* a meta-skill —
  "design a workflow" as a skill.
- **pbakaus/impeccable** — design-quality skill: PRODUCT.md durable-truth,
  24 commands, 61 deterministic detector rules (no LLM needed). *Gem:* the
  deterministic-rules pattern — validation without burning tokens.
- **duolahypercho/gauntlet-loop** — aim-prompt skill: fan-out builders,
  separate harsh critic, blind A/B. *Gem:* the critic pattern — a "qa" bot
  that only exists to be brutal.

### Model access / local engines (BYOK + zero-cost)
- **ggml-org/llama.cpp, vllm-project/vllm, Tiiny-AI/PowerInfer,
  jundot/omlx, ARahim3/mlx-tune, sharpner/turboquant-mlx +
  rachittshah/mlx-turboquant, syv-ai/qwen38-27b-rtx3090,
  karpathy/nanochat, jbulger82/LLAMA_Hub** — the local-model stack spectrum.
  *Gem:* `LLAMA_Hub` (llama.cpp GUI + multi-agent command center + RAG +
  MCP + browser automation) is closest to a complete local engine NEST
  could shell out to.
- **antirez/ds4 (DwarfStar)** — antirez's narrow, self-contained inference
  engine for DeepSeek/GLM/Qwen flashes on consumer Metal/CUDA incl. SSD
  streaming. *Gem:* deliberately-narrow engine philosophy — "a few excellent
  models" > universal support.
- **JustVugg/colibri** — MoE experts streamed from disk, pure C, zero deps.
- **freellmapi + gpt4free + FreeToken + free-claude-code +
  router-for-me/CLIProxyAPI + EasyCLIProxyAPI + standardagents/composer-api +
  badrisnarayanan/antigravity-claude-proxy + EvanZhouDev/openai-oauth +
  duolahypercho/codex-router** — free-tier/subscription provider shims and
  OAuth bridges. *Gem for the zero-cost policy:* a provider-failover proxy
  layer (freellmapi aggregates ~14 providers w/ automatic failover) is the
  cheapest engine possible — pairs with BobTheBuilder's cost-policy.
- **aws-samples/llm-gateway** — gateway reference.
- **steipete/CodexBar** — menu-bar app showing every provider's rate-limit
  windows. *Gem:* "plan around resets" — NEST could surface per-provider
  limit state when picking a bot's model tier.

### Code intelligence / context
- **jgravelle/jcodemunch-mcp** — tree-sitter AST index over MCP: token-
  efficient code exploration. *Gem:* the context-cost reducer for bot
  sessions (fits budget.preset).
- **Graphify-Labs/graphify** — folder→queryable knowledge graph skill
  (multi-harness: Claude/Codex/Cursor/Gemini/OpenCode). (⚠ fork conflicts.)
- **alexziskind1/codeneedle** — code-understanding tool.
- **zubair-trabzada/brain-map** — markdown→knowledge-graph, zero deps.
- **zed-industries/zed + VSCodium/vscodium + nexu-io/open-design** — editor
  internals if NEST ever embeds a real editor.
- **Astro-Han/karpathy-llm-wiki** — build a knowledge base skill-compatible
  with Claude Code/Cursor/Codex.

### Channels / reach / I/O
- **firecrawl/firecrawl + Panniantong/Agent-Reach +
  BunsDev/perplexica-search-engine-ai** — web data + search connectors.
- **matthartman/ghost-pepper** (Swift, ⚠ fork conflicts) — hold-to-talk
  WhisperKit transcription. *Gem:* voice input for the shell.
- **fikrikarim/parlor + TalkWithMe + personaplex** — on-device voice/persona.
- **scorbo2/TalkWithMe** — configurable AI chat w/ TTS/STT.
- **lunu-bounir/clipboard-manager** — clipboard context tricks.
- **osaurus-ai/osaurus** — native macOS agent harness, persistent memory,
  autonomous execution, cryptographic identity. *Gem:* identity/signing for
  bots ("this commit was Bob, not me").
- **hassancs91/PyRunner** — schedule-and-monitor scripts w/ UI —
  `[[automations]]` reference.
- **kestra-io/kestra** — serious event-driven orchestration (if NEST ever
  outgrows cron semantics).
- **drumih/turbo-fieldfare, svkozak/pi-acp, Shrey113/Android-Dex,
  ShenSeanChen/waku-agent, thecodacus/agentbox** — adapters/sandboxes:
  agentbox = chat→ephemeral Docker sandboxes w/ live noVNC — a gem for
  "bot runs in a box you can watch".

### Research/benchmark/specialist
- **TauricResearch/TradingAgents** — multi-agent trading framework.
- **elder-plinius/T3MP3ST** — autonomous red-teaming meta-harness → a
  security-audit bot/skill for NEST.
- **grigio/opencode-benchmark-dashboard + alexziskind1/draftbench +
  promptfoo/promptfoo** — model/agent eval harnesses → the "test drive" and
  rubric-checking machinery for eggbot.
- **zipstack/unstract** — document→structured-data agent platform.
- **666ghj/MiroFish** — swarm-intelligence prediction engine.
- **remotion-dev/remotion** — programmatic video — a bot could render
  walkthrough videos (Symphony's proof-of-work trick).

### UI shell references (Phase 2 — The Shell)
- **pingdotgg/t3code** — "agent harness control surface": mobile + web +
  Electron desktop controlling Claude/Codex/Cursor/GrokBuild/OpenCode/
  Antigravity. *Gem:* the closest public reference for NEST's
  one-window-multi-agent shell — and it already drives Grok's agent.
- **multica, traycer, aider-desk, openwork, accomplish, osaurus,
  CodexBar** (above) — desktop UX patterns to study.
- **MODSetter/SurfSense** — NotebookLM-for-teams (also a BobTheBuilder skill).
- **open-gsd/gsd-core** (was gsd-build/get-shit-done — upstream moved) — the
  agent+workflow framework Steven already uses daily in Gemini.

### Also present (inventory completeness)
Tutorials/assets (`yt-tutorial`, `stitch-skills` early capture,
`ai-engineering-hub`, `ai-executive-assistant`, `free-for-life`,
`awesome-*` lists), infra/tools (`WindowsSlimBuilder`, `clipboard-manager`,
`obsidian-releases`, `photopea`, `Android-Dex`, `CorridorKey`,
`ComfyUI-Easy-Install`, `muninndb`, `autumn`, `unstract`, `openai-oauth`),
leaked-source mirror (`claude-leaked-files` — Claude Code internals snapshot;
reference-grade reading for the skills/hooks formats), misc agent apps
(`Jarvis`, `python-tgpt`, `agentic-os`, `LLAMA_Hub`, `openbrain`,
`hermes-desktop`, `hermes-agent-mission-control`, `dsh-commandcode-go-provider`,
`fusion-fable`, `free-claude-code`, `codex-host`, `codex-router`,
`get-shit-done`, `thepopebot`, `picobot`, `picoclaw`, `zeroclaw`, `CoPaw`,
`NemoClaw`, `oh-my-*`, `buzz` (hive-mind comms in Rust!), `atuin`,
`paperclip`, `turnstone`, `osw-studio`, `gooey-pi`, `git-lrc`, `MiroFish`,
`oracle`, `impeccable`, `openworker`, `ds4`, `colibri`, `odysseus`,
`agentbox`, `waku-agent`, `prime-agent`, `agentctl`, `agent-zero`,
`membrane`, `mempalace`, `mem0`, `membrane`, `OpenViking`, `t3code`,
`traycer`, `multica`, `symphony`, `gobii-platform`, `openhuman`,
`hermes-agent`, `oh-my-openagent`, `pi-mono`, `pi-acp`, `fusion-harness`,
`vllm`, `llama.cpp`, `zed`, `jan`, `vscodium`, `firecrawl`, `SurfSense`,
`Perplexica`, `kestra`, `autumn`, `remotion`, `ComfyUI-Easy-Install`,
`promptfoo`, `unstract`, `open-design`, `awesome-*`, `nanoclaw?` — full
table in fork_status.txt if a complete listing is wanted.

---

## 4. Where I'd fold what (recommendations, not actions)

1. **`[[bots]].permissions`** — adopt BobTheBuilder's per-tool `permission:`
   frontmatter map verbatim (allow/ask/deny per tool). It lands naturally as
   a manifest field → `.nest/agents/<name>.md` frontmatter.
2. **Routing/delegation** — a `boss` bot type + `.nest/routing.toml` from his
   auto-detection table; escalation ladder (fail→council→fallback) as
   `[[automations]]`-adjacent config.
3. **BobTheBuilder as NEST package #1** — wrap `agents/*.md` + skills as a
   `package.toml` with `[[agents]]` + `[[skills]]` — instantly gives NEST its
   starter council and dogfoods the package machinery.
4. **Council skill** — llm-council + fusion-harness + gauntlet-loop patterns
   → one "second-opinion" bot capability.
5. **Services layer** — `[[services]]` (name/port/health/start_cmd) modeled
   on orchestrate.sh, so the app can boot its memory/search backends.
6. **hermes-agent's learning loop** — later phase: bots authoring their own
   skills (Spoke packages written by bots) — the long-term differentiator.
7. **agentctl-style spawn wrappers** — the engine-starting-thin answer:
   wrap Claude/Codex/Pi/A0 CLIs rather than re-implementing an engine.
8. **waku-agent's visible-loop dashboard** — UI gem for Guided mode
   (watch the agent think; noob-friendly transparency).
9. **Zero-cost provider layer** — freellmapi/codex-router style failover
   honors BobTheBuilder's cost-policy — a local provider proxy.
10. **CodexBar-style limit awareness** — surface per-provider reset windows
    when choosing `model:` tiers.
