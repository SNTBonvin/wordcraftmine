# Branded builds

A downstream product built on WordCraft can change its name, accent colour, icons and a few
buttons at **build time**, without patching the UI code. Point `WORDCRAFT_BRAND_DIR` at a folder
with a `brand.toml`:

```sh
WORDCRAFT_BRAND_DIR=/path/to/mybrand cargo build --release -p wordcraft
```

Without the variable nothing changes: the generated constants are WordCraft's defaults and every
hook falls through to the existing code path.

The brand folder lives **outside this repository**. Its fonts, icons and licences are the
downstream product's; they never enter WordCraft (see the asset rules in `CLAUDE.md`).

## `brand.toml`

```toml
name = "Acme Writer"                 # window title, About, Save Changes, settings folder, docx Application
tagline = "One line for About."
website = "https://example.com"

[accent]                             # one source for the accent; hover/text shades are derived
light = "#1F2D4A"
dark = "#7A95CD"

[hide]                               # hidden in the UI; the commands stay
discord = true
artcraft = true                      # the ArtCraft logo and name in About
share = true
autosave = true
tabs = ["Developer", "Zotero"]

[relabel]                            # English UI string → replacement, in every language
"About WordCraft" = "About Acme Writer"

[icons]                              # optional icon font; unmapped icons keep WordCraft's
font = "icons/MyIcons.ttf"
glyphs = "icons/MyIcons.json"        # {"glyph_name": codepoint, …}
map = "icons.map"                    # lines: "<wordcraft icon name> <glyph_name>"

[[tab]]                              # an extra ribbon tab, placed before Help
name = "Acme"
[[tab.group]]
name = "Assistant"
commands = [{ id = "acme.assistant", label = "Assistant", icon = "sparkle_glyph" }]

[[licence]]                          # extra entries in About › Licenses
name = "My Icons"
file = "icons/LICENSE"
```

The tab's commands are registered (so the ribbon, search and keytips see them) but disabled
("Not available in this version yet") until the product implements them.

## Where it is read

| crate | file | what |
|---|---|---|
| `ui-egui` | `build.rs` → `OUT_DIR/brand_gen.rs`, read through `ui_egui::brand` | everything above |
| `control-key` | `build.rs` | settings folder named after `name` (own preferences, dictionary, key files) |
| `docx` | `build.rs` | `docProps/app.xml` Application: `"<name> <version>"` |

Hooks in the UI: window title (`apps/wordcraft`), `theme::Tokens::branded`, `icons::paint`,
`i18n::t`/`tc` (`[relabel]`), ribbon tabs, Share, AutoSave, Discord (title bar, Help tab, File
pane, keytips) and the About dialog (branded text and a Licenses tab).
