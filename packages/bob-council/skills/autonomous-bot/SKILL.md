---
name: Autonomous Bot (thepopebot)
description: Deploy and manage an autonomous 24/7 AI agent that uses GitHub Actions for free compute, commits its own work as PRs, and self-evolves. Configured for Google Gemini (zero-cost).
triggers:
  - bot
  - autonomous
  - popebot
  - 24/7 agent
  - background agent
  - scheduled task
---

# Autonomous Bot (thepopebot)

## Overview
thepopebot is an autonomous AI agent infrastructure where **the repository IS the agent**. Every action the agent takes is a git commit. It uses GitHub Actions for free compute and runs tasks 24/7 via scheduled cron jobs, webhooks, or chat commands.

> **IMPORTANT**: This fork has been refactored to use **Google Gemini 3.1 Flash** as the default LLM provider instead of Anthropic Claude. This ensures zero additional API costs under the current subscription policy.

## Architecture
```
User (Chat/Telegram/Webhook/Cron)
    → Event Handler (Next.js)
        → Creates job/* branch on GitHub
            → GitHub Actions spins up Docker container
                → Pi coding agent executes the task
                    → Commits results, opens PR
                        → Auto-merge workflow handles the rest
```

## Key Features
- **Free compute** via GitHub Actions (included with every GitHub account)
- **Self-evolving** — agent modifies its own code through auditable PRs
- **Git-native** — every action is a commit; full history, fork = clone your agent
- **Scheduled jobs** via `config/CRONS.json`
- **Multi-channel** — Web chat, Telegram, webhook API

## Prerequisites
- Node.js 18+, Git, GitHub CLI (`gh`), Docker
- A GitHub Personal Access Token (repo + workflow scopes)
- A Google API Key (for Gemini — get one free at https://aistudio.google.com/apikey)

## Quick Start

### 1. Initialize the project (already cloned)
```bash
cd ~/Documents/projects/thepopebot
npm install
```

### 2. Run the setup wizard
```bash
npm run setup
```
The wizard will:
- Create a GitHub repository
- Collect your **GOOGLE_API_KEY** (not Anthropic!)
- Set GitHub secrets and variables
- Generate `.env`
- Build the project

### 3. Start the agent
```bash
docker compose up -d
```

### 4. Schedule recurring tasks
Edit `config/CRONS.json` to add scheduled jobs:
```json
[
  {
    "schedule": "0 9 * * *",
    "job": "Check for news about AI and summarize the top 5 stories"
  }
]
```

## Configuration

### Environment Variables (Key ones)
| Variable | Description |
|----------|-------------|
| `LLM_PROVIDER` | Set to `google` (default after refactor) |
| `LLM_MODEL` | Defaults to `gemini-3.1-flash` |
| `GOOGLE_API_KEY` | Your free Gemini API key |
| `APP_URL` | Public URL for webhooks |

### Cost Policy Compliance
- Default model: `gemini-3.1-flash` (zero-cost tier)
- Anthropic provider code has been **removed** from `lib/ai/model.js`
- `@langchain/anthropic` can be uninstalled from `package.json` if desired

## Project Location
`~/Documents/projects/thepopebot`
