# AI & LLM Configuration Guide

Prism is designed to be model-agnostic and supports multiple AI backends. 

## LLM Providers

### 1. Google Gemini (AI Studio)
Prism uses the **Gemini 1.5 Pro / Flash** models. To handle API quirks between different account types (Free, Enterprise, Pay-as-you-go), we've implemented two critical reliability features:

- **Combined Prompt Strategy:** System instructions are merged directly into the user message for maximum compatibility with all Gemini API versions (v1, v1beta).
- **Automatic Retry Logic (429):** If the Gemini API rate limits your request, Prism will automatically wait and retry (up to 3 times) before reporting an error.
- **Stable Endpoint:** Uses `/v1beta` for features like `systemInstruction` while fallback logic ensures `/v1` compatibility.

**Environment Variable:** `GEMINI_API_KEY` (or `GOOGLE_API_KEY`).

### 2. Claude Code CLI (`claude-cli`)
A unique provider that executes the `claude` command line tool directly. 
- **No API Key Required:** Uses your existing local Claude Code session.
- **Native Tasking:** Best for "Deep Fix" tasks where you want Claude to propose code changes.

### 3. OpenAI & Anthropic (Direct API)
Standard implementations for GPT-4o and Claude-3.5-Sonnet. Requires direct API access.

**Environment Variables:** `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`.

---

## Token Optimization Features

Prism includes several strategies to minimize LLM token consumption and costs:

### 1. Context Truncation
- **Max Diff Tokens:** Diff content is intelligently truncated at clean file/hunk boundaries if it exceeds the `max_diff_tokens` setting.
- **Global Exclusions:** Large generated files, lockfiles, and vendor directories are automatically stripped from the diff before being sent to the AI.

### 2. Phase-0 Injection
Prism runs a single "Objective" agent first. Its concise summary is then injected into all specialist agents, preventing them from having to analyze the whole PR description and linked ticket independently, saving significant input tokens.

### 3. Smart De-duplication
By de-duplicating similar comments in Rust code *before* the summarization phase, Prism avoids sending redundant text to the final Summarizer agent, further reducing output and input token costs.
