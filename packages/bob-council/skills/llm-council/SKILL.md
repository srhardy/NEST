---
name: LLM Council
description: Multi-perspective AI debate framework where multiple Gemini instances (with varied temperatures) independently answer your question, anonymously rank each other, and a Chairman synthesizes the best final answer.
triggers:
  - council
  - debate
  - multi-model
  - second opinion
  - compare answers
---

# LLM Council

## Overview
LLM Council is a local web app that sends your query to multiple LLM instances in parallel, asks them to anonymously rank each other's responses, and then a Chairman model synthesizes the best final answer.

> **IMPORTANT**: This fork has been refactored to use **Google Gemini 3.1 Flash** exclusively (via direct REST API), eliminating the need for paid OpenRouter credits. Diverse perspectives are achieved through **temperature variance** (0.3, 0.7, 1.0) across three parallel Gemini calls.

## 3-Stage Process
1. **Stage 1 — First Opinions**: Query sent to 3 Gemini instances (varied temperatures) in parallel
2. **Stage 2 — Peer Review**: Each instance anonymously ranks the other responses
3. **Stage 3 — Chairman Synthesis**: A designated chairman model compiles the final answer

## Prerequisites
- Python 3.10+ with [uv](https://docs.astral.sh/uv/)
- Node.js (for the React frontend)
- A Google API Key (free at https://aistudio.google.com/apikey)

## Quick Start

### 1. Install dependencies
```bash
cd ~/Documents/projects/llm-council
uv sync
cd frontend && npm install && cd ..
```

### 2. Configure API key
```bash
cp .env.example .env
# Edit .env and add your GOOGLE_API_KEY
```

### 3. Run
```bash
./start.sh
# Backend: http://localhost:8001
# Frontend: http://localhost:5173
```

## Cost Policy Compliance
- **Zero external API costs** — uses direct Google Gemini REST API
- **No OpenRouter dependency** — removed entirely
- Default model: `gemini-3.1-flash` (cheapest tier)
- Temperature diversity simulates multi-model council without multi-model costs

## When to Invoke This Skill
Use the LLM Council when you need:
- A **second opinion** on a complex question
- To **compare different reasoning approaches**
- To **validate** an important decision with diverse perspectives
- To **debate** trade-offs in architecture or strategy

## Project Location
`~/Documents/projects/llm-council`
