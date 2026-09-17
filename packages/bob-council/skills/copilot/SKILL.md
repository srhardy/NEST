---
name: Copilot Web Consulting
description: reasoning fallback using GitHub Copilot via Chrome browser (Zero-Cost). Use when Gemini tokens are exhausted or complex debugging is required.
triggers: ["copilot", "reasoning fallback", "fix with browser", "consult copilot"]
---

# Copilot Web Consulting Skill

## Overview
This skill allows the assistant to consult the **GitHub Copilot Chat** web interface as a "reasoning fuel tank." This is the primary fallback for when Gemini 3.1 Flash (the primary zero-cost engine) is exhausted or when a technical problem requires high-level architectural insight.

> [!IMPORTANT]
> This skill utilizes the user's existing GitHub Copilot subscription via the Chrome browser. It is compliant with the **Zero-Cost Policy** as it does not incur additional API usage fees.

## When to Use
1.  **Token Exhaustion**: When `model_status.sh` indicates Gemini quotas are hit.
2.  **Complex Debugging**: When multi-step code fixes are failing or hallucinations are observed in lower-tier models.
3.  **Architectural Guidance**: When the assistant needs to verify a complex system design or library integration.

## Usage Instructions

### 1. Access the "Fuel Tank"
Use the `browser_subagent` to navigate to:
`https://github.com/copilot`

### 2. Describe the Context
Provide the following to the subagent Task:
- The content of the relevant code files.
- The error log or current failure state.
- The specific question or "fix" requested.

### 3. Strategy for Consultation
- Focus on asking for **specific code fixes** or **implementation steps**.
- Ask Copilot to "Think step-by-step" to maximize output quality.
- Verify the suggested fix against the local codebase before applying.

## Example Subagent Task Prompt
```
1. Open Chrome and go to https://github.com/copilot.
2. Provide the code from [FILE_PATH].
3. Ask: "[PROBLEM_DESCRIPTION]. Suggest a fix that adheres to [CONSTRAINTS]."
4. Return the suggested code block precisely.
```

## Maintenance
Keep this skill updated if the GitHub Copilot UI structure changes significantly, requiring updated `click_browser_pixel` or `type_browser_text` instructions.
