# Control protocol

For normal desktop use, copy the reusable MCP configuration from **File → Options → Agents**
and explicitly enable connection requests once. Invitations are optional. Native discovery
defaults off; its saved switch is separate from consent for each attachment.
The separate restricted listener needs no special launch flags. See [live agents](live-agents.md). The developer control port described
below remains an explicit, broader automation interface.

Start the app with a control port: `wordcraft --control 7981` (or `WORDCRAFT_CONTROL_PORT=7981`).
It listens on `127.0.0.1` only. Send one JSON object per line, with the window's key (see
[Keys](#keys)); each gets one reply line:

```json
{"id": 1, "key": "…", "method": "engine.execute", "params": {"command": "text.insert", "params": {"text": "Hello"}}}
{"id": 1, "ok": true, "result": {"anchor": {...}, "focus": {...}}}
```

Errors come back as `{"id": …, "ok": false, "error": "message"}`. A failed command leaves the
document unchanged. A line that isn't valid JSON gets one error reply, then the server closes the
connection (so an HTTP request sent to the port can't run a command); reconnect to continue.

Transport failures are not automatically retried by the MCP bridge. A lost reply can mean the
operation already happened (`outcome_unknown`); read the document before retrying. Queued
requests carry cancellation and a deadline, which the UI checks before dispatch. Cancellation
or timeout during an already running command cannot roll it back or guarantee no changes.

## Live-session interface

Native Unix windows publish a filesystem Unix socket in a private 0700 session directory
(socket 0600); Flatpak uses `$XDG_RUNTIME_DIR/app/ai.storyteller.wordcraft/` without adding
network permission. Windows uses mutually authenticated numeric loopback TCP. Descriptors at
`<settings>/agents/sessions/<sessionId>.json` contain `sessionId`, `documentId`, `title`,
`version`, `protocolVersion: 1` and either `transport: "unix"` with `socketPath` or
`transport: "tcp"` with `address` and `authScheme: "hmac-sha256-v1"`. They contain no access key. Windows live credentials use
`<settings>/control-key.live-<port>`, independently of the legacy listener. A stale descriptor
must pass connection/status validation; missing/refused endpoints are swept without deleting
keys or sockets. Temporary failure preserves the descriptor. Listing caps report truncation;
an invitation directly selects a session. Disabling requests or closing the window removes it.

Live TCP first requires `auth.challenge` (client nonce plus expected session/document) and
`auth.prove` (client proof). The app returns its fresh nonce and server proof first. HMAC-SHA256
binds both nonces, exact identity and role with a domain-separated length-prefixed transcript.
The bridge verifies the server before sending its client proof; neither side sends the secret.
Reconnection repeats authentication. Four pre-authentication slots with a three-second overall
deadline are separate from sixteen authenticated slots. Old live TCP schemes fail closed.

After this handshake (or Unix filesystem authorization), the listener accepts only
`agent.status`, `agent.call`, `agent.disconnect` and `agent.check`.
`agent.check` validates session/document identity and reports protocol/permission metadata,
without document text, registration or approval. Other requests also include `agentName` and
the MCP process's stable `clientId`. The host assigns socket identity; clients cannot forge it.
`agent.call` wraps a permitted operation. The app checks approval, allowlist and revision state
even through generic `execute` or `batch`.

`agent.status` first returns `approval_required`; only an explicit user **Allow** establishes
an attachment. The name is self-reported, not a verified provider identity. **Deny** or Escape
returns `approval_denied`; pending requests expire after two minutes. Polling cannot extend
them. **Deny and stop requests** disables discovery, while a short cross-client cooldown
limits repeated prompts. MCP `connect_session` requests attachment;
`get_session_status` polls the decision. One agent may attach. The display name is distinct
from the host-reserved revision author, which cannot collide with the owner or existing authors.
Its selection and editing settings are isolated from the owner's.

A fresh socket for the same approved logical client preserves approval, but invalidates its
read and selection. Disconnect must match both client and current socket; old sockets cannot
evict a reconnected client. Closed sockets release attachment independently of queued cleanup.
The app's **Disconnect agent** and document replacement revoke the old session; if discovery
remains enabled, a new identity is published and the next attachment requires approval.

Connections, line size, queue capacity, partial-line deadlines and UI dispatch time are bounded.
Authenticated connections can stay idle indefinitely. Disabling requests closes sockets and
cancels unstarted commands. These are local cooperating processes under the same OS user;
consent and socket permissions do not sandbox other software running as that user.

## Keys

This section describes legacy developer TCP authentication; live TCP uses the mutual proof
handshake above.

Any local program (and possibly a web page) can reach a loopback port, so every request needs the
window's key. Once the port is bound, the app writes a random key (256 bits, as hex) to
`<settings>/control-key.<instance>` and removes the file when the window closes; a new start
writes a new key. `<settings>` is the folder that holds the preferences: on Linux
`$XDG_CONFIG_HOME/wordcraft` (by default `~/.config/wordcraft`), on macOS
`~/Library/Application Support/WordCraft`, on Windows `%APPDATA%\WordCraft`. `<instance>` is the
Flatpak instance id inside Flatpak, and `local-<port>` otherwise. On Unix the file has mode 0600.

```sh
cat ~/.config/wordcraft/control-key.local-7981   # Linux, outside Flatpak, port 7981
```

- Send the key as top-level `"key"` with every request, or once per connection with
  `{"method": "auth", "key": "…"}` (reply `{"ok": true, "result": {}}`); later requests on that
  connection may leave `key` out. Only `auth` reads `params.key`; for other methods `params.key`
  is an ordinary parameter (`ui.key {"key": "Enter"}` presses Enter).
- The key is checked on every request. A missing or wrong key gets
  `{"ok": false, "error": "unauthorized"}`, and the app closes the connection.
- A line over 1 MiB gets `{"ok": false, "error": "line too long"}`, and the app closes the
  connection.
- A connection that sends no authorised request within 30 s of being accepted is closed. That is
  one limit: blank lines and bad JSON do not extend it. After an authorised request the
  connection may stay idle indefinitely; an incomplete line must finish within 60 s.
  The bridge may reconnect after observing closure before sending a request, but never replays
  a request once transmission starts.

`wordcraft-cli mcp --connect` takes the key from `WORDCRAFT_CONTROL_KEY`, or else from the key
file for the port it connects to, and reads it again on every new connection (the app may have
restarted). It sends the key only to loopback addresses (`127.0.0.0/8`, `::1`, `localhost`). For
any other host it sends no key, says so, and the app refuses its requests: to drive an app on
another machine, forward its port to `127.0.0.1` (for example `ssh -L 7981:127.0.0.1:7981 host`)
and set `WORDCRAFT_CONTROL_KEY` to that app's key.

## Methods

| Method | Params | Result |
|---|---|---|
| `engine.execute` | `command`, `params` | the command's result (any id from `engine.commands`, including UI commands `ui.tab`, `ui.dialog`, `ui.backstage`, `ui.zotero.*` (see `docs/zotero.md`), `ui.language` (`{"value": "auto"|"en"|"zh-hans"|"zh-hant"|"ja"|"pt-br"|"es"|"uk"|"sr"|"sr-latn"|"et"|"tr"|"ru"|"pl"|"de"|"it"|"cs"|"hu"|"fr"}`), `ui.theme` (`{"value": "system"|"light"|"dark"}`; `system` follows the OS appearance, light when it reports none), `ui.interfaceSize` (`{"percent": 80–200}`, clamped; no value reads it; scales the interface on top of the system display scale, document zoom stays `view.zoom`), `ui.animations` (`{"value": "system"|"on"|"off"}`; no value reads it; `system` follows the system's animation setting, which the desktop app reads on Windows, macOS, Linux and the BSDs and the web app from `prefers-reduced-motion`, and means on where the system doesn't say; the result's `enabled` is what the setting comes to and `system` what the system reports, `null` when it doesn't say), `ui.fileBrowser` (desktop: `{"purpose": "document"|"picture"|"recipients"|"save"}` opens WordCraft's own file browser, for when the system file dialog doesn't show up, #576), `ui.caretAnimation` (`{"value": true|false}`; no value reads it; off, the caret jumps and blinks on and off instead of gliding and fading), `ui.startScreen` (`{"value": true|false}`; no value reads it; on, the default, the desktop and web apps start on File › Home when launched with nothing to open, #766)…) |
| `engine.execute` | `command`, `params` | the command's result (any id from `engine.commands`, including UI commands `ui.tab`, `ui.dialog`, `ui.backstage`, `ui.zotero.*` (see `docs/zotero.md`), `ui.language` (`{"value": "auto"|"en"|"zh-hans"|"zh-hant"|"ja"|"pt-br"|"es"|"uk"|"sr"|"sr-latn"|"et"|"tr"|"ru"}`), `ui.theme` (`{"value": "system"|"light"|"dark"}`; `system` follows the OS appearance, light when it reports none), `ui.interfaceSize` (`{"percent": 80–200}`, clamped; no value reads it; scales the interface on top of the system display scale, document zoom stays `view.zoom`), `ui.animations` (`{"value": "system"|"on"|"off"}`; no value reads it; `system` follows the system's animation setting, which the desktop app reads on Windows, macOS, Linux and the BSDs and the web app from `prefers-reduced-motion`, and means on where the system doesn't say; the result's `enabled` is what the setting comes to and `system` what the system reports, `null` when it doesn't say), `ui.fileBrowser` (desktop: `{"purpose": "document"|"picture"|"recipients"|"save"}` opens WordCraft's own file browser, for when the system file dialog doesn't show up, #576), `ui.caretAnimation` (`{"value": true|false}`; no value reads it; off, the caret jumps and blinks on and off instead of gliding and fading), `ui.contextMenu` (opens the page's context menu below the caret, as Shift+F10 does)…) |
| `engine.commands` | — | every command: id, label, location, shortcut, params, enabled |
| `document.inspect` | `text?` | blocks (text, style, runs, lists, tables), parts, sections, selection, pages |
| `ui.inspect` | — | UI state, view, dialog, window size, page rects on screen, caret, comment balloons (`balloons`: id + screen rect; `balloon`: the selected one), perf |
| `ui.click` | `x`, `y`, `button?`, `count?`, `shift?`, `cmd?`, `alt?` | real pointer input at window coordinates |
| `ui.clickText` | `page` (0-based), `x`, `y` (points from the page's top-left), `count?` | click inside a page |
| `ui.move` / `ui.drag` | `x`,`y` / `x`,`y`,`toX`,`toY`,`steps?` | pointer move / drag |
| `ui.press` / `ui.release` | `x`,`y`, modifiers | primary button down / up there (with `ui.move` between: a drag you can screenshot halfway) |
| `ui.key` | `key` (`B`, `Enter`, `Left`…), `shift?`, `alt?`, `cmd?` | key press through egui (shortcuts apply) |
| `ui.text` | `text` | typed text through egui |
| `ui.screenshot` | `path?` | PNG of the window: written to `path`, or without one returned as base64 `png` in the reply |
| `ui.render` | `path?`, `page?` (1-based), `scale?` | PNG of a page, rendered by the engine: written to `path`, or without one returned as base64 `png` in the reply (never a fixed name in the shared temporary folder) |
| `ui.parity` | — | feature-catalog parity |
| `ui.resize` / `ui.focus` / `app.quit` | | window control |

Any other method name is treated as a command id: `{"method": "format.bold"}` works.

The `CharProps` and `ParaProps` that `format.set`, `para.set` and `styles.modify` take are listed
in [MCP server](mcp.md#character-and-paragraph-properties).

Positions are `{"story": "body" | {"part": n}, "path": [block, row, cell, block, …], "off": byteOffset}`.

### Native printing

`engine.execute` can call `ui.printSettings` with `{}` to inspect the printer list, selected
printer, copies, duplex, grayscale, discovery/submission status and backend receipt/error.
`{"refresh":true}` starts background discovery; poll the same command until `discovering` is
false. Set `printer` to a discovered name, integer `copies` from 1 to 999, `duplex` to
`printerDefault`, `off`, `longEdge` or `shortEdge`, and boolean `grayscale`. Invalid updates are
rejected without changing earlier settings. `ui.printSubmit` explicitly submits the current
print-ready PDF without opening a dialog; inspect `ui.printSettings` until `busy` becomes false
and check `status`/`detail`. The browser keeps its own print hook. PDF export remains separate.

`ui.printSubmit` / `ui.print` also accept the upstream PDF page settings (`range`, `pages`,
`parity`, `copies`, `collate`). Native printing renders the selected pages once and passes
copies/collation to the spooler; the browser and `file.printPdf` keep copies in the PDF.
Omitting native `copies` uses the configured `ui.printSettings` value. `collate` defaults to
File › Print's setting; invalid ranges submit no job.

`engine.commands {"search":"print"}` lists the engine print commands and the app-owned
`ui.printSettings`, `ui.printSubmit`, and legacy `ui.print` alias, with current enablement
and their parameter documentation. Search is case-insensitive across IDs, labels, locations
and parameter descriptions. Listing commands never discovers printers, submits a job or opens
a dialog. Settings inspection is always available; submission becomes enabled when the native
printer is ready, or when the browser host provides its print flow.
