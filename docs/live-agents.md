# Agents in an already open WordCraft window

> **Status:** second PR #1432 review fixes, 2026-10-14. Current local Linux full CI passed
> (2,039 tests, two existing ignored), with native integration and visually checked offscreen UI.
> Native CI results and installed-package verification are tracked separately in Draft PR #1432.

Open WordCraft normally. **File → Options → Agents** provides a reusable MCP configuration.
Register it in your external agent client once; some clients need a tool reload or new agent
session after registration. WordCraft stays open. Client registration is distinct from making
the bridge available or approving an attachment.

Connection requests are **off by default** in native windows. Enable **Allow agent connection
requests** explicitly once. This is a saved preference, separate from the per-attachment editing grant. Turn it off to stop discovery and revoke
all access to this window. Reopening WordCraft or changing documents does not require repeating
the setup; every new attachment still requires the user's decision.

## Connect and approve

1. The agent calls `list_sessions` and chooses the document intended by the user. An optional
   **Copy invitation** selects its exact session; it contains document identity and a short
   pointer to `get_usage_guide`, never an executable path or a duplicate guide.
2. `connect_session {"sessionId":"…","agentName":"Copyeditor"}` requests an attachment.
3. WordCraft shows **Agent Copyeditor wants to access Document: Allow / Deny** and marks
   the name as self-reported by the requesting client. Until the user
   allows it, the agent cannot read document text or execute edits. On `approval_required`,
   the client waits for the user and retries `get_session_status`. On `approval_denied`, stop.
   Escape denies the request. Pending requests expire after two minutes; polling cannot extend
   them. **Deny and stop requests** disables discovery until the owner re-enables it. A short
   cooldown after denial also suppresses new-client-ID prompt storms.
4. Approval confirms the exact session, document and host-assigned revision author. The agent
   reads `get_usage_guide` and `inspect_document`, selects an explicit target, edits and verifies.
5. A status-bar button shows listening, a pending request or the connected client and opens the Agents pane. **Disconnect agent**
   revokes this attachment and invitation; discovery remains available with a fresh session.
   Another attachment, including a restarted client, requires approval again.

`disconnect_session` releases the client while leaving discovery available. A new socket for
an already approved logical client preserves its approval but requires a fresh read and target.
Changing documents revokes all old attachment state and publishes a new identity. Invitations
are selectors, not credentials or substitutes for consent.

The canonical guide comes from MCP initialization and `get_usage_guide`; no client-specific
skill installation is required. The server starts without an open WordCraft window and waits;
it never creates a headless document as a fallback. Discovery filters canonical JSON descriptors before limiting returned sessions and sweeps
unchanged descriptors whose endpoints are missing or refuse connections. It never deletes
keys or sockets belonging to a potentially new window. Timeouts and busy endpoints remain
unverified; `truncated` and `sweepLimited` report bounded scan/check work. Direct invitation
lookup bypasses the listing cap. A connection/check must verify the intended running session.

## Installation and local checks

The copied JSON contains `mcpServers.wordcraft.command` and `args`. Clients with another format
can use the same values. Paths including spaces remain one executable argument. Configuration
contains no document grants or access keys. Moving the installation requires refreshing the
client configuration.

| Installation | Exported bridge |
|---|---|
| Linux/BSD native and macOS app | This installation's executable, `--mcp-live` |
| AppImage | Original AppImage path, `--mcp-live`, not its temporary mount |
| Windows MSI/portable | Matching sibling `wordcraft-cli.exe`, `mcp --live` |
| Flatpak | `/usr/bin/flatpak run --command=wordcraft-cli ai.storyteller.wordcraft mcp --live` |

The embedded Unix stdio entry point runs before GUI/log/preferences initialization, creating no
extra window. Windows release GUI executables use the GUI subsystem, so the matching console
CLI is required. Setup reports the version and exact command, checks the Windows CLI's version
and `--live` support, and provides a matching-release installation link if it is unavailable.
After installing both matching portable executables or reinstalling the release, **Refresh**
rechecks setup. Probes run lazily in a shared background job, leaving launch and UI frames
responsive; checks reuse the same setup work. Primary setup/check/recovery messages are
localized, with technical details in the advanced section. Version/help probes bound time and output, using an anonymous regular-file
stdout so an inherited handle cannot block their read. There are no automatic PATH changes or writes to external client settings.

**Check live connection** starts a bounded asynchronous local check: MCP initialization, tool
discovery and `check_session` against this exact document, without reading its text, registering
an agent or requesting approval. Failure, timeout and cancellation clean up the child process.
The check does not establish external client registration. Flatpak/AppImage checks use their
internal installed binary; they do not prove that a host client's launcher has been registered
or that the external package wrapper works on every system.

Native Unix live connections use a filesystem Unix socket in a private 0700 session directory,
with a 0600 socket. Under Flatpak it lives beneath
`$XDG_RUNTIME_DIR/app/ai.storyteller.wordcraft/`; otherwise the runtime directory is used, with a
short private `/tmp` fallback for systems without one. The enclosing private directory prevents
other OS users from accessing the socket. Windows uses numeric loopback TCP and an internal credential file. Before any document RPC,
the bridge and app mutually prove possession of the secret with HMAC-SHA256, bound to role,
session/document identity and two fresh 32-byte nonces. The raw secret is never sent. A freed
port impersonator cannot pass server verification. Four pre-authentication slots with an overall
three-second deadline are separate from sixteen authenticated slots. Older live TCP descriptors
without the advertised proof scheme fail with an update instruction; legacy `--control` authentication
is preserved. Legacy `--control` stays authenticated TCP on all platforms.

Flatpak networking permissions remain unchanged: the live transport does not require
`--share=network`. Host bridges discover descriptors under
`~/.var/app/ai.storyteller.wordcraft/config/wordcraft`. Isolated clients must be able to access
the same descriptor and socket paths; this feature does not make remote/cloud connections.

## Editing and reliability

One agent is supported per document. It edits the in-memory document, including unsaved work,
with an independent selection. The application reserves a distinct revision author rather than
trusting the display name as author identity. Existing owner/revision/comment authors cannot be
claimed by changing `agentName`. Agent edits are tracked; the user's editing settings, caret and
undo grouping are isolated. The profile excludes opening/replacing documents, saving/exporting,
filesystem access, UI clicks and accepting/rejecting changes. The user reviews and saves.

Any owner document revision invalidates the agent's read and selection. The agent must reread,
reassess and choose a new explicit target. `select_text`, `select.text`, `select.range`,
`select.all`, `select.owner` or a collapsed absolute `caret.set`/`caret.docStart`/`caret.docEnd`
establish a target; relative moves and `extend:true` cannot clear invalidation. `inspect_document`
includes `ownerSelection`, and `select.owner` copies ordinary owner selection without moving it.
This conservative policy can block concurrent owner typing; collaborative conflict merging or
an atomic read-select-edit operation remains future work.

Selection mapping prefers retained paragraph identity over equal text. Strict agent-target mapping rejects ambiguous changed regions. Owner UI mapping is advisory:
an otherwise successful edit may collapse the owner selection to a validated edit-start or
post-command caret. Its result includes `agentWarnings` with `owner_selection_relocated` and
a next action to inspect the successful edit without retrying it. Agent target invalidation and
explicit reselection requirements are preserved. Traversal/paragraph/text limits apply to the
changed region, so small edits in documents over 65,536 paragraphs remain supported. A bounded fallback for explicitly allowed formatting commands preserves positions
only after certifying unchanged paragraph paths/text and retained identities. Tests include
blank/duplicate paragraphs, formatting-only edits and generated Unicode positions.

Accepted sockets are explicitly returned to blocking mode on every platform. Unix client
sockets use portable fd APIs for close-on-exec and nonblocking connect, then restore blocking
I/O; platforms requiring it enable `SO_NOSIGPIPE`. Authenticated
connections may remain idle; incomplete lines retain a bounded deadline. The MCP bridge checks
for already-observed peer closure before sending a new request. This can safely reconnect an
idle closed socket; it never replays an operation once transmission has started.

`outcome_unknown` means an operation may have happened before its reply was lost. Inspect before
any retry. Queued commands are checked for cancellation and expiry; a timeout during an already
running command cannot undo it or prove no changes. `batch` and multiline edits are ordered,
not atomic: inspect individual failures and verify the final document.

## Validation and limits

Regression sources cover accepted nonblocking sockets, idle/partial framing, private Unix
transport, consent/denial, protected owner revisions, isolated state, duplicate paragraphs,
optional invitations and non-registering checks. Offscreen UI exercises approval buttons,
configuration copy, status navigation and revocation. The `Live agents` workflow runs native
transport/engine/MCP tests on Linux, macOS and Windows; configured coverage is not evidence of
successful runs or installed-package behavior.

This is local integration for cooperating programs under the same OS user. Another program
running as that user can already read credentials or modify local files; neither consent UI nor
socket permissions are an OS sandbox against it. Browser WordCraft has no native listener.
Built-in chat, multiple agents, arbitrary CLI candidate selection/download management and an
exactly-once operation receipt store remain outside this implementation.
