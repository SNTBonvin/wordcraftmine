# European Portuguese interface and proofing

> **Last reviewed:** 2026-10-12 · **Last updated:** 2026-10-12 · **Change:** minor (European Portuguese interface catalog, 2,061 entries, and pt-PT spelling from the installed `pt_PT` dictionary) · **Target:** WordCraft 0.6.1

Choose **File › Options › Interface language › Português (Portugal)**, or follow a European
Portuguese system display language (`pt-PT`, `pt_PT.UTF-8`, and the regions that write like
Portugal: `pt-AO`, `pt-MZ`, `pt-CV`, `pt-GW`, `pt-ST`, `pt-TL`, `pt-MO`, `pt-GQ`). The catalog
code is `pt-pt`; `ui.language` with `{"value":"pt-pt"}` selects it explicitly, and
`WORDCRAFT_LANGUAGE=pt-PT` does the same for a headless screenshot or a launch while the
preference is `auto`. `pt-BR` keeps resolving to the Brazilian catalog, and a bare `pt` names
neither vocabulary, so it stays in English. Document text, command ids, CLI/MCP/control responses
and file paths do not change.

`crates/ui-egui/src/i18n/pt-pt.tsv` has all 2,061 entries of the Brazilian catalog (the same
keys). It was adapted from this repository's own `pt-br.tsv` and the meaning of the English
labels; no proprietary translation resources were consulted. 574 entries differ from the Brazilian
ones. Strings added later fall back to English until they are translated.

## Terminology

| Source | European Portuguese | Brazilian Portuguese |
|---|---|---|
| File / Save / Delete | Ficheiro / Guardar / Eliminar | Arquivo / Salvar / Excluir |
| Font / Character | Tipo de letra / Carácter | Fonte / Caractere |
| Undo / Reset | Anular / Repor | Desfazer / Redefinir |
| Cut / Crop | Cortar / Recortar | Recortar / Cortar |
| Section / Record | Secção / Registo | Seção / Registro |
| Password / User Name | Palavra-passe / Nome de utilizador | Senha / Nome de usuário |
| Indent / Border | Avanço / Limite | Recuo / Borda |
| Bullets / Bookmark | Marcas / Marcador | Marcadores / Indicador |
| Table of Contents / Index | Índice / Índice remissivo | Sumário / Índice |
| Track Changes / Tracked changes | Registar alterações / Alterações registadas | Controlar alterações / Alterações controladas |
| Mail Merge / Merge Cells | Impressão em série / Unir células | Mala direta / Mesclar células |
| Link / Layout / View | Ligação / Esquema / Ver | Link / Layout / Exibir |
| Ribbon / tab | Friso / separador | Faixa de opções / guia |
| Settings / Default | Definições / Predefinição | Configurações / Padrão |
| Strikethrough / Subscript / Superscript | Rasurado / Inferior à linha / Superior à linha | Tachado / Subscrito / Sobrescrito |
| Landscape / Portrait | Horizontal / Vertical | Paisagem / Retrato |
| Synonyms / Polish / Czech | Sinónimos / Polaco / Checo | Sinônimos / Polonês / Tcheco |

Grammar follows European usage: no `você` (`Ao escrever`, not `Enquanto você digita`), the article
before possessives (`as suas alterações`), enclitic pronouns (`substitui-o`, `ignoram-nos`),
`a` + infinitive for the progressive (`A aguardar o Zotero…`), and `num`/`numa`. Spelling follows
the 1990 agreement as applied in Portugal (`secção`, `facto`, `carácter`, `económico`, `hífenes`).
Product and format names stay as they are.

## Proofing

Text tagged `pt-PT` (or one of the regions above) is `ProofLang::PtPt`: it is spelled against the
Hunspell `pt_PT` dictionary installed on the system, found in the same places as the other
dictionaries (`crates/proof/src/hunspell.rs`): the `WORDCRAFT_DICTIONARIES` folder,
`%LOCALAPPDATA%\WordCraft\dictionaries` on Windows, the system Hunspell folders, and LibreOffice's
`dict-pt-PT` extension. The dictionary is not bundled (it is GPL/LGPL/MPL). Both UTF-8 and
ISO 8859-1 files are read. Without a `pt_PT` dictionary, European text is checked with the
Brazilian one, as it was before the two spellings were told apart; with neither, there are no
spelling marks.

With the dictionary of the Natura project (Universidade do Minho, the one LibreOffice ships) the
affix engine reads all 42,642 stems and 1,290 rules: `facto`, `equipa`, `ecrã`, `ficheiro` and
`económico` are words, `econômico`, `ônibus` and `direcção` are flagged, and suggestions come from
the same dictionary (`economico` → `económico`, `tambem` → `também`). Grammar rules and
hyphenation patterns are shared with Brazilian Portuguese. **Review › Language** offers
**Português (Portugal)**, the status bar names it, and a `pt-pt` interface writes new documents in
`pt-PT`.

## Verification and remaining scope

`cargo test -p wordcraft-ui-egui --lib i18n::tests` covers the European locale tags and preference
order, saving and restoring `pt-pt`, unchanged document content, full key parity with the
Brazilian catalog, the vocabulary pairs above, a check that Brazilian wording (`você`, `arquivo`,
`salvar`, `senha` …) doesn't slip back in, placeholders, count wording, English fallback and the
accented glyphs in the bundled Inter and JetBrains Mono. `cargo test -p wordcraft-proof` covers the
region split (`pt-PT`, `pt_AO`, `pt-BR`, bare `pt`), the two dictionaries side by side with
synthetic test dictionaries, suggestions per spelling, and ISO 8859-1 decoding.

Not covered yet: hyphenated prefixed forms and some clitic combinations the dictionary doesn't
derive (`fê-lo`), a few ribbon labels that are English in both Portuguese catalogs, and a review
of the catalog by a second native speaker, which is welcome.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-12 | minor | European Portuguese catalog (2,061 entries), terminology, pt-PT proofing against the installed `pt_PT` dictionary, and regression checks |
