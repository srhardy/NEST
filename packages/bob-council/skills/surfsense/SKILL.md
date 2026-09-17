---
name: SurfSense
description: Self-hosted semantic search and research assistant for personal memory.
triggers: ["surfsense", "search my research", "local memory", "browse memory"]
---

# SurfSense Skill

## Overview
SurfSense is a self-hosted semantic memory engine that indexes URLs and research data. It provides a local semantic search endpoint.

## Configuration
- **API Endpoint**: `http://localhost:3000` (web/api)
- **Watcher**: Watchtower auto-updates ensure persistence and security.
- **Support**: Integrated with local LLMs (Ollama/Ocho).

## Tools (Assistant Instructions)

### 1. Search Research
Search the semantic index for previous findings or browsing history.
Endpoint: `http://localhost:3000/api/search` (POST `{ "query": "..." }`)

### 2. Index Page
Manually push a URL to SurfSense for later recall.
Endpoint: `http://localhost:3000/api/index` (POST `{ "url": "..." }`)

## Maintenance
Use `docker ps` to find `surfsense-bac` and `surfsense-web` containers.
Check logs for `surfsense-web` to monitor indexing progress.
