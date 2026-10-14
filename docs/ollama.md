# Ollama in WordCraft

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (redesigned chat, saved conversation history; document/research harness with Ollama and direct llama.cpp backends)

Open **Agents → Agent Chat**. [Codex subscription chat](codex-chat.md) is also available in the same pane. The resizable chat pane opens on the right, beside the document.
WordCraft reads the available models from Ollama; it does not download model weights itself.

- **Local Ollama:** start your installed Ollama app or `ollama serve`, then choose a local model.
  The default server is `http://127.0.0.1:11434`; only loopback endpoints are accepted.
- **Ollama Cloud:** without an API key, WordCraft uses cloud models registered in your signed-in
  local Ollama app, such as `gemma4:31b-cloud`. Add a cloud model in Ollama if the list is empty.
  With a key in **Connection settings**, WordCraft connects directly to `https://ollama.com`
  and reads its current cloud catalog. An account's Ollama usage limits still apply.
- **llama.cpp:** point Connection settings at a loopback llama-server, default
  `http://127.0.0.1:8085`. Models are read from `/v1/models`. This bypasses the Ollama daemon;
  existing GGUF weights can be reused without another download. For example:

  ```sh
  llama-server -m /path/to/model.gguf --host 127.0.0.1 --port 8085 --ctx-size 16384 --parallel 1 --reasoning off
  ```

- **Refresh models** reloads the catalog. Changing a provider or model starts a new model context.
  **New chat** starts another conversation and keeps the previous one in local history, without undoing document edits.
- Backend, model and server choices persist between app launches. Editing permission always resets to off.
  API keys remain in memory for the current app session, and are never saved in preferences,
  document files, control inspection, or source code. `OLLAMA_API_KEY` is also supported.

## Document tools and editing

Every message starts with an automatic snapshot of the actual document title, selected text,
editing permission and body preview; fresh tool reads supersede it. This prevents a skipped read
from leaving a small model with no document context. Local Ollama uses a 16k context window.

The assistant can discover WordCraft commands and their parameters, inspect the current document,
then execute commands for document properties, text, formatting, lists, tables, pictures, equations, references,
comments, and layout. Requests and streaming replies run on background workers. Document tools
return to the UI thread so edits appear in the open document immediately.

**Allow AI editing** defaults to off. While off, only document reading and inspection are allowed;
selection, formatting and content changes are rejected. The permission is checked for every
command, including commands already queued by a model. Switching documents cancels pending
work and turns editing off. If you manually change the document during inference, the assistant
must read it again before editing.

Each AI editing command creates a separate **Ollama: …** or **Codex: …** step in the normal Undo history.
**Undo AI change** reverts the latest consecutive AI step and stops an in-flight request. It is
disabled after a later manual edit; use the ordinary Undo history to undo those later steps first.
This avoids discarding manual writing. Completed edits remain undoable after Stop or New chat.

The assistant cannot dispatch file save/open/new, filesystem access, macros, password protection,
system settings or arbitrary shell commands. Create a new draft inside the current document by
asking explicitly to replace its body; save it yourself using the normal WordCraft workflow.

Local Ollama and llama.cpp use a schema-constrained JSON action language for small models.
The compact tool surface is document reading, verified writing, command discovery/execution,
and public web search/fetch. Cloud models can use native Ollama tools. Other models use the
same structured action protocol. Invalid actions fail with a visible error. Requests have timeouts, bounded
responses and queues, a 16-step turn limit, and a maximum of four background workers.

## Internet tools

**Web search and browsing** enables `web_search` and `web_fetch`. With an Ollama API key these use
Ollama's web APIs. Without a key, search uses public Bing RSS results and fetch reads public HTTPS
pages through WordCraft's existing HTML reader. Fetch validates and pins DNS addresses, rejects
local/private networks, limits redirects and response sizes, and never forwards cloud credentials.
The **Retrieved sources** list comes directly from successful search/fetch results, independently
of the model's prose. Those links can be opened from the pane. Switching web search and browsing off cancels
the current request and hides web tools on subsequent turns. This switch controls web tools;
Ollama Cloud inference still requires its cloud connection.

Document and web text are treated as untrusted data in the assistant's system instructions.
The `write_text` skill handles append, replacement of a selected passage, and replacement of an
explicitly requested whole draft. It compares the inserted fragment with the requested text and
rolls back a mismatch. The UI separately reports actual AI steps in Undo history, so a model's
claim is not the only evidence of a change. These guarantees concern validated operations;
model interpretation and the factual accuracy of research cannot be guaranteed at 100%.

## Commands and diagnostics

The ribbon and control channel use `ui.ollama`, `ui.ollamaSettings`, `ui.ollamaRefresh`,
`ui.ollamaSend`, `ui.ollamaStop`, `ui.ollamaNewChat`, `ui.ollamaEdit`, `ui.ollamaInternet`, and
`ui.ollamaUndo`. `engine.commands` describes them; `ui.inspect` includes credential-free chat
state. These are desktop features; the WASM UI shows that desktop Ollama is required.

Protocol references: [Ollama cloud](https://docs.ollama.com/cloud),
[tool calling](https://docs.ollama.com/capabilities/tool-calling), and
[web search and fetch](https://docs.ollama.com/capabilities/web-search), and
[llama.cpp server](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md).

## Verification (2026-10-13)

`cargo xtask ci` passed formatting, Clippy, workspace tests, asset attribution, layering and
all required WASM checks. Live synthetic-document tests passed with Qwen 3.5 4B through both
Ollama and direct llama.cpp, and Gemma 4 31B through Ollama Cloud: append, read-only rejection
and undo. A stricter live test checked the exact sample-document title after automatic
context grounding was added. Direct llama.cpp also completed live public search/fetch with retrieved source links
and no document edits. Headless screenshots confirmed the ribbon, right task pane and Backstage
spacing; no Microsoft assets were used.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Local/cloud Ollama and direct llama.cpp; compact schema-constrained Word/research harness, verified writing, enforced editing permission, individual AI undo and web tools |

## Chat history

All providers share the [local history browser and deletion controls](codex-chat.md#chat-history-and-deletion).
Use the clock button to search and resume chats, each row's **•••** to rename/delete, and the
header's **•••** to delete all history. Model and Codex thinking choices are inside the composer;
**+** offers project linking, web access and the hard document-editing switch.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Shared rounded chat composer, local searchable history, resume/rename/delete commands and softer AI review highlights |
