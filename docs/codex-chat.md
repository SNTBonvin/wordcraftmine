# Codex chat in WordCraft

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (subscription-backed in-app Codex provider)

Open **Agents → Codex Chat**, or choose **Codex** in the Agent chat provider menu.
WordCraft starts the installed `codex app-server` over private stdin/stdout and uses its managed
ChatGPT sign-in. No OpenAI API key is required. If you already signed in through Codex CLI, that
account is reused; otherwise **Sign in with ChatGPT** opens the official browser sign-in flow.
An API-key-only account is not accepted by this provider. Your Codex account limits apply.

The model menu comes from `model/list`; access is confirmed by a successful model turn, not by
the catalog alone. Connection settings show available subscription usage information, and link
to usage management. Account email, OAuth tokens, and raw account responses are never placed in
WordCraft preferences, document files, chat diagnostics or logs.

## Live document tools

Codex receives the same document snapshot and writing/research instructions as the local agent
harness. Its dynamic document and public web tools use the existing UI-thread dispatcher:
**Allow AI editing** is off after launch, each action checks live permission, cancellation,
document identity and revision, and verified writing rolls back mismatches. The most recent
consecutive AI change can be undone using **Undo change**. Codex steps are labelled **Codex: …**
in the main Undo history. Switching documents cancels work and turns editing off.

The provider disables shell execution, code mode, hooks, connectors, plugins, computer/browser
control, subagents, and native Codex web search. User-configured MCP connections are disabled
for the child process; any remaining active MCP tool catalog causes the turn to fail closed.
Unexpected approval requests are declined. A read-only sandbox is retained as an additional
guard. Only WordCraft's six document/research tools are handled by the client. Turning off
**Search the web** cancels the turn and removes the research tools from later turns. Cloud
inference still requires internet. The MCP connection ribbon toggle controls external document
control independently of in-app Codex chat.

Each turn uses an ephemeral Codex thread with the completed conversational text and a fresh
live document snapshot. WordCraft keeps its transcript in memory; it does not persist chat
history between launches. Stop kills the child app-server and abandons unfinished messages;
completed document changes remain undoable. Requests, event queues, response sizes and action
counts are bounded. Browser sign-in and model turns time out after five minutes.

## Installation discovery

WordCraft searches the process PATH, standard Homebrew locations, the Codex app, and installed
VS Code Codex extension binaries. `WORDCRAFT_CODEX_BIN` can point to an absolute executable
path. Codex owns account storage and renewal. WordCraft creates an empty
`$CODEX_HOME/wordcraft-workspace` working directory (normally `~/.codex/wordcraft-workspace`),
so the assistant does not inherit the document's folder as its project. It reads only connection
names from Codex configuration to disable external MCP tools, without altering the configuration.
Connection names must use letters, numbers, underscores or hyphens.

The chat pane uses original drawn UI: a compact provider/model header, suggestion cards, rounded
user messages, selectable formatted assistant text, Copy buttons, a rounded composer and
streaming responses. No third-party graphical assets are copied.

Commands: `ui.codexChat`, `ui.codexSignIn`, and the shared `ui.ollamaSettings` (source `codex`),
`ui.ollamaRefresh`, `ui.ollamaSend`, `ui.ollamaStop`, `ui.ollamaNewChat`, `ui.ollamaEdit`,
`ui.ollamaInternet`, `ui.ollamaUndo`. `ui.inspect.ollama` reports credential-free provider,
account-plan label, usage summary and chat state. These commands are described by `engine.commands`.

Protocol and authentication: [official Codex app-server documentation](https://learn.chatgpt.com/docs/app-server).
Dynamic tools are an experimental app-server surface; incompatible versions fail visibly.

## Verification (2026-10-13)

The signed-in ChatGPT Plus account loaded the live Codex model catalog. A synthetic-document
integration test passed document reading, exact insertion including an initial newline,
independent AI undo, read-only protection, public web search and page fetch with retrieved
sources. Regression tests cover the shared edit gate, host configuration, conversation context,
bounded protocol errors and usage rendering. Headless screenshots verified both the welcome
screen and a completed Codex response beside the unchanged sample document. The workspace
quality gates cover formatting, Clippy, tests, assets, layering and WASM.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Codex subscription provider, browser sign-in, model discovery, streamed chat and guarded live document/research tools; refreshed chat layout |
