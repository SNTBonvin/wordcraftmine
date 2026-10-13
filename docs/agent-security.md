# Agent security and data controls

> **Last reviewed:** 2026-10-13 · **Last updated:** 2026-10-13 · **Change:** minor (explicit agent permissions and file approvals)

Agent chat → **••• → Settings** separates Models & connection, Permissions, Project files and
Data controls. The chat composer retains model/reasoning selection, attachments and Stop. A compact project row on the
chat screen shows the linked folder, approved-file count and a Manage project shortcut. A linked
folder with no approved files is explicitly marked as having no enabled file context.
The layout was observed in ChatGPT in Opera; artwork remains original and no website assets are bundled.

## Permissions

- Document editing starts disabled. The UI-thread dispatcher checks the live switch, cancellation,
  document identity and revision before each write. Models cannot dispatch settings commands,
  save/open files, run macros, change protection or invoke shell tools.
- Codex, Ollama Cloud and model names identifying cloud use require the human to enable **Share
  context with cloud models** before any chat request includes document text or images. Consent is
  in memory, bound to the provider, and cleared on application launch, provider/server changes and saved
  chat restoration. It cannot be granted through control commands or model tools. Sending without
  consent preserves the draft and shows an inline **Allow cloud sharing and send** control; it never
  opens Settings automatically. Settings remains an explicit way to revoke or grant sharing.
- Local servers may themselves delegate requests to cloud services. WordCraft cannot determine
  how an arbitrary local daemon routes or retains data. The settings explain this boundary.
- Web access is separate. Turning it off cancels the active worker. Search queries go to Bing;
  fetched pages use HTTPS, bounded replies, checked/pinned public DNS addresses and revalidated
  redirects. Requests bypass system proxy variables. Cloud keys are never forwarded to local
  model endpoints or web pages.

## Linked projects

Linking a folder selects **all eligible files by default**, indexed on a background worker. There
is no approval-count, directory-depth or total-entry cutoff. Settings → Project files provides
Select all / Deselect all and individual exclusions; the file list renders visible rows without
a total-row limit. Refresh discovers new files; Select all includes them. Listing/search/retrieval
only sees selected files. Selections and exclusions stay in memory; linking/reopening a project or restoring a chat
indexes and selects eligible files again. Cloud-sharing consent remains separate. Revoking a file or all access cancels active work
and starts fresh live context; existing saved chat messages are retained until explicitly deleted.

Search can narrow by folder/file path and scans selected files beyond the former 128-file limit.
Results remain limited to 30 matches per request, and individual reads remain bounded.

Hidden entries, common credential filenames and private-key extensions are excluded. Reads are
bounded and reject traversal, symbolic links and resolved paths outside the root. Credential-content
checks reject common key formats, private-key blocks and common secret assignments before retrieval
or search snippets leave the worker. These checks are additional safeguards, not complete secret
classification: users must review approved files. Approved files are read live and can change.

## Local history and authentication

**Temporary Chat** / disabling **Save new chats locally** prevents new conversation capture. Existing
history is retained. Creating a saved branch requires local saving to be enabled. This setting does
not change the cloud provider’s retention or training policy. The UI offers management and confirmed
individual/all-history deletion. History stores visible messages, citations and attachment paths;
never API keys or tool-call authority. History files are plaintext JSON, atomically written with
mode `0600` on Unix. Processes running as the same OS user can read them. OS disk encryption is
needed for encryption at rest.

Ollama keys stay in memory (entered by the user or read from `OLLAMA_API_KEY`). Codex uses its
installed CLI’s existing account authentication over private stdio. User-configured MCP servers,
shell, plugins and other unrelated Codex capabilities are disabled for document-assistant threads.

The separate Codex/MCP control channel binds only to localhost, authenticates with 32 bytes of OS
randomness, stores the key with mode `0600`, bounds request sizes/concurrency and closes its port when
disabled. Trusted external control clients have broader app capabilities than in-app model tools.
It does **not** isolate agents from other programs running as the same OS user. Do not publish or
forward this port, or give its key to untrusted clients. Settings exposes its enabled state/toggle.

## Audit and PR evidence

On 2026-10-13, `cargo-audit 0.22.2` with the current RustSec database found **zero reported
vulnerabilities** in `Cargo.lock`. It reported two informational unmaintained-dependency warnings:
`rustybuzz 0.20.1` (RUSTSEC-2026-0206), and `ttf-parser 0.25.1` (RUSTSEC-2026-0192). They remain visible
for maintainer review; no advisory is suppressed. Replacing these core font dependencies requires
separate font shaping/metrics compatibility work. `.github/workflows/security.yml` checks lockfile
changes and runs weekly so future vulnerability advisories fail CI.

Regression coverage includes denied cloud sends before worker creation, transient permissions,
temporary-history noncapture, unapproved-file listing/retrieval/search, credential contents, traversal,
symlinks, editing revocation, cloud-key forwarding and private-network web targets. Passing these
checks is evidence for a PR review, not a claim of a full penetration test or formal security audit.
Production releases retain the repository’s own signing/notarization pipeline; the installed local
personal build is ad-hoc signed.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-13 | minor | Newly linked projects select all eligible files; unlimited selection, background indexing, bulk controls and compact project indicator |
| 2026-10-13 | minor | Inline cloud consent keeps Send in chat; atomic connection validation, busy-send draft protection and interrupted-response preservation |
| 2026-10-13 | minor | Explicit agent Settings, cloud consent, file approvals, temporary history and recurring dependency audit |
