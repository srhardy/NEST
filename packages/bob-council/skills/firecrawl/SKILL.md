---
name: Firecrawl
description: High-performance, self-hosted web crawler and scraper for structured data extraction.
triggers: ["crawl", "scrape", "extract from web", "firecrawl"]
---

# Firecrawl Skill

## Overview
Firecrawl is a self-hosted engine for professional-grade web crawling. It can turn any website into high-quality LLM-ready markdown or structured JSON.

## Configuration
- **API Endpoint**: `http://localhost:3002` (default)
- **Zero-Cost Policy**: Runs locally via Docker, no external API keys required.

## Tools (Assistant Instructions)

### 1. Crawl URL
Send a POST request to `http://localhost:3002/v1/crawl`.
Payload:
```json
{
  "url": "https://example.com",
  "limit": 10
}
```

### 2. Extract Data
Send a POST request to `http://localhost:3002/v1/scrape`.
Payload:
```json
{
  "url": "https://example.com/page",
  "formats": ["markdown"]
}
```

## Maintenance
Use `docker compose ps` in `~/Documents/projects/firecrawl` to check service health.
