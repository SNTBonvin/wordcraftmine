# German interface

> **Last reviewed:** 2026-10-11 · **Last updated:** 2026-10-11 · **Change:** minor (German interface catalog and regression checks) · **Target:** WordCraft 0.5.0

Choose **File › Options › Interface language › Deutsch**, or follow a German system display language
(`de-DE`, `de-AT`, `de-CH`, `de_DE.UTF-8`). The catalog code is `de`; `ui.language` with
`{"value":"de"}` selects it explicitly, and `WORDCRAFT_LANGUAGE=de-DE` does the same for a headless
screenshot or a launch while the preference is `auto`. The choice is saved with the other interface
settings. Document text, command ids, CLI/MCP/control responses and file paths do not change.

`crates/ui-egui/src/i18n/de.tsv` has all 1,440 keys of the Spanish catalog. It is written from the
meaning of the English labels in ordinary German computing vocabulary; no proprietary translation
resources were consulted. The catalog format has no plural forms, so counts use count-neutral
wording that is grammatical for every number (`Wörter: 0`, `Wörter: 1`, `Wörter: 21`;
`Zeilen: 2`). Placeholders, `\n` breaks in ribbon labels and dialog ellipses are kept, and strings
added later fall back to English until they are translated.

## Conventions

- Second person is **du** (as in the Spanish catalog's *tú*); commands are infinitives
  (*Speichern*, *Kopieren*); standard German spelling with `ß`, neutral across Germany, Austria
  and Switzerland.
- Decimal comma in translated numbers (`Zeilenabstand 1,5`). Margin presets keep their inch values
  (`Normal  1" rundum`).
- Product and format names stay as they are (WordCraft, PDF, Markdown, MCP, Zotero …).
- Count-neutral wording instead of plurals: `{words} words` → `Wörter: {words}`,
  `{selected} of {words} words` → `Wörter: {words}; markiert: {selected}`, `{n} lines` → `Zeilen: {n}`.

## Terminology

| Source | German |
|---|---|
| Home / Insert / Design | Start / Einfügen / Gestaltung |
| Font / Font Size / Paragraph | Schriftart / Schriftgrad / Absatz |
| Style / Styles / Style Set | Formatvorlage / Formatvorlagen / Formatvorlagensatz |
| Review / Track Changes | Überprüfen / Änderungen nachverfolgen |
| References / Citation / Bibliography | Verweise / Zitat / Literaturverzeichnis |
| Header / Footer / Header Row | Kopfzeile / Fußzeile / Titelzeile (Tabelle) |
| Caption / Table of Contents / Index | Beschriftung / Inhaltsverzeichnis / Stichwortverzeichnis |
| Mailings / Mail Merge / Merge Field | Sendungen / Serienbrief / Seriendruckfeld |
| Equation / Formula | Gleichung / Formel |
| Undo / Redo / Repeat | Rückgängig / Wiederherstellen / Wiederholen |
| Undo {action} | Rückgängig: {action} |
| Page {page} of {pages} | Seite {page} von {pages} |
| Bookmark / Drop Cap / Watermark | Lesezeichen / Initiale / Wasserzeichen |

## Verification and remaining scope

`cargo test -p wordcraft-ui-egui --lib i18n::tests` covers German locale tags and preference order,
saving and restoring `de`, unknown preferences, unchanged document content, key parity with
Spanish, placeholders, count wording, English fallback, and the glyphs `ÄäÖöÜüß„“` in the bundled
Inter (Regular, Medium, SemiBold) and JetBrains Mono. The shared catalog checks (validation, every
tab and command translated) include German automatically.

Spelling, grammar and hyphenation stay English-only: this is an interface translation, not German
proofing. Several long ribbon labels (for example *Literatur hinzufügen/ändern*) are longer than
their English sources, so a visual check of the ribbon at the default interface size is worthwhile.
A native-speaker skim of the catalog is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-11 | minor | German catalog (1,440 entries), terminology, scope and regression checks |
