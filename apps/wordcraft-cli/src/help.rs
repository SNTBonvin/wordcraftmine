//! The help: one page per subcommand, and the overview that lists them.

/// A subcommand's help page.
pub struct Subcommand {
    pub name: &'static str,
    pub usage: &'static str,
    pub summary: &'static str,
    pub details: &'static str,
}

pub const SUBCOMMANDS: &[Subcommand] = &[
    Subcommand {
        name: "convert",
        usage: "convert IN OUT [--markup]",
        summary: "save a document in the format OUT's extension names",
        details: "\
Opens IN and saves it as OUT. The format is OUT's extension: .docx, .odt, .rtf, .html, .md, .tex,
.txt and .json keep the text; .pdf lays the pages out; .png draws the first page (render draws
others). IN may also be a .doc (Word 97-2003), read only.

--markup is Print Markup: the comments beside each page and the tracked changes in .pdf and
.png, a comment list in .html, .md and .txt.

  wordcraft-cli convert letter.docx letter.pdf",
    },
    Subcommand {
        name: "info",
        usage: "info FILE",
        summary: "print a document's pages, words, paragraphs and properties as JSON",
        details: "\
Opens FILE and prints one JSON object: pages, words, paragraphs, sections, comments, whether
changes are tracked, and the document properties (title, creator, dates…).

  wordcraft-cli info letter.docx",
    },
    Subcommand {
        name: "text",
        usage: "text FILE",
        summary: "print a document's text",
        details: "\
Prints the body text of FILE, one line per paragraph. Headers, footers, footnotes and comments
are left out; inspect shows them.

  wordcraft-cli text letter.docx",
    },
    Subcommand {
        name: "inspect",
        usage: "inspect FILE",
        summary: "print a document's structure as JSON",
        details: "\
Opens FILE and prints its structure as JSON: every top-level block (a paragraph with its style,
text, runs and objects, or a table with its cells), the headers, footers and other parts, the
sections with their page setup, the selection, and the counts info prints. It is the same answer
as running document.inspect.

  wordcraft-cli inspect letter.docx",
    },
    Subcommand {
        name: "render",
        usage: "render FILE OUT.png [--page N] [--scale S]",
        summary: "draw one page as a PNG image",
        details: "\
Lays FILE out and draws page N (1 = the first, the default) into OUT.png at S pixels per point
(default 2, so a Letter page is 1224 × 1584 pixels). Prints the image's path, size and byte count.

  wordcraft-cli render letter.docx page2.png --page 2",
    },
    Subcommand {
        name: "run",
        usage: "run [--file FILE | --template NAME] [--cmd COMMAND[=PARAMS]]... [--save OUT] [--page N] [--scale S] [--print] [--author NAME]",
        summary: "run commands on a document, then save it",
        details: "\
Runs commands, in order, on one document: FILE with --file, a built-in template with --template
(blank, sample, letter, resume or report), else a blank document. Then --save writes the document
as OUT; its extension picks the format, as convert does, and a .png takes --page and --scale as
render does.

Each --cmd is one command: COMMAND is a command id, then `=`, then the command's parameters as one
JSON object. Quote it for the shell. A command that needs no parameters is just its id.

  wordcraft-cli run \\
    --cmd 'text.insert={\"text\":\"Quarterly report\"}' \\
    --cmd select.paragraph \\
    --cmd 'format.font={\"name\":\"Georgia\"}' \\
    --cmd 'format.size={\"size\":24}' \\
    --cmd 'para.align={\"value\":\"center\"}' \\
    --save report.docx

`wordcraft-cli commands` lists the command ids; `wordcraft-cli describe COMMAND` explains one and its
parameters.

What the ids don't say:
- A document is a list of blocks, paragraphs and tables, numbered from 0. A position is
  {\"story\": \"body\", \"path\": [block], \"off\": N} where N is a byte offset into the paragraph's
  text; inside a table cell the path is [block, row, cell, paragraph]. {\"block\": 2, \"off\": 0}
  is a shorthand for a body paragraph. Headers, footers, footnotes and text boxes are parts:
  their story is {\"part\": id}.
- Typing and inserting happen at the caret, replacing the selected text if there is one.
  select.text {\"text\": …} selects the first occurrence of a text; select.paragraph, select.all and
  caret.set {\"pos\": …} move the selection without changing the document.
- format.* and para.* act on the selected text, else at the caret: format.* then sets what is
  typed next, para.* the caret's paragraph. styles.modify changes a style everywhere it is used.
- Sizes and distances are in points (72 pt = 1 inch); colours are \"RRGGBB\". A new document is a
  Letter page with 1-inch margins; layout.size and layout.margins change that.
- insert.header and insert.footer put text in a header or footer; insert.editHeader moves the
  caret into it for further commands, insert.closeHeader moves it back.
- text.newParagraph carries the formatting at the caret into the new paragraph, as Enter does in
  Word: after a formatted heading, format.resetChar before typing goes back to the style's.
- format.set, para.set and styles.modify take CharProps and ParaProps objects; `wordcraft-cli
  help types` lists their fields.
- Answers count the way positions do: `offset 10` in an answer is `\"off\": 10` in a position, in
  bytes, so a letter like ä counts 2. Text in an answer longer than 60 characters is shown by its
  ends, as in `start … end` (141 bytes); document.paragraph {\"path\": [n]} gives a paragraph whole.
- --author names who tracked changes and comments are by.

If a command fails, run stops there and saves nothing. Without --print, run ends with one line,
e.g. `OK: 6 commands ran, saved report.docx`; --print shows each command's answer first: what
it changed, or the JSON a command like document.inspect answers with.",
    },
    Subcommand {
        name: "commands",
        usage: "commands [FILTER] [--json]",
        summary: "list the commands run can use, one per line",
        details: "\
Prints every command as `id — Label: {parameters}`, or only those whose id or label contains
FILTER (case doesn't matter). --json prints the same list as JSON objects with id, label,
location, shortcut, params and mutates.

  wordcraft-cli commands header",
    },
    Subcommand {
        name: "describe",
        usage: "describe COMMAND",
        summary: "explain one command and its parameters",
        details: "\
Prints what COMMAND is, where it lives in the ribbon, each parameter with what it takes, what the
command answers with, and how to run it.

  wordcraft-cli describe insert.picture",
    },
    Subcommand {
        name: "parity",
        usage: "parity [--markdown]",
        summary: "report which features of the Word catalog have commands",
        details: "\
Compares the word-processor feature catalog with the command registry and prints the result as
JSON, or as the Markdown checklist `cargo xtask parity` writes with --markdown.",
    },
    Subcommand {
        name: "mcp",
        usage: "mcp [--connect HOST:PORT | --author NAME] [--automation-read-root DIR] [--automation-write-root DIR]",
        summary: "serve the commands to an agent over MCP on stdio",
        details: "\
Starts an MCP server (Model Context Protocol, JSON-RPC over stdin and stdout) whose tools run the
commands: headless on its own document, or with --connect inside the WordCraft app started with
`wordcraft --control PORT`, which also lets the agent click, type and take screenshots.

--author NAME names who the headless session's tracked changes and comments are by; with
--connect the app's own user name applies.

--automation-read-root and --automation-write-root confine the files the agent's commands read
(open, insert text or picture, mail-merge data, templates…) and write (save, export, merge
output…) to DIR, links followed; a root left out grants none of its access. They apply to the
headless server and are an error with --connect.

  wordcraft-cli mcp --automation-read-root ~/in --automation-write-root ~/out",
    },
    Subcommand {
        name: "zotero",
        usage: "zotero COMMAND FILE [--cmd COMMAND[=PARAMS]]... [--save OUT] [--trace] [--port P]",
        summary: "run a Zotero command on a document",
        details: "\
Opens FILE and runs the Zotero integration command COMMAND on it: addEditCitation,
addEditBibliography, addNote, refresh, removeCodes or setDocPrefs. Zotero must be running; --port
is its integration port (default 23116). Each --cmd runs first, as run runs it, e.g. --cmd
caret.docEnd to put the caret where the citation goes. --save writes the document afterwards;
--trace prints every call to Zotero and its reply.

  wordcraft-cli zotero refresh paper.docx --save paper.docx",
    },
    Subcommand { name: "version", usage: "--version", summary: "print the version", details: "Prints `wordcraft-cli` and its version." },
];

/// The help page for a subcommand, `help` itself or `--version`.
pub fn page(name: &str) -> Option<String> {
    let name = match name {
        "help" | "--help" | "-h" => return Some(overview()),
        "types" => return Some(TYPES.to_string()),
        "--version" | "-V" => "version",
        name => name,
    };
    SUBCOMMANDS.iter().find(|s| s.name == name).map(|s| format!("usage: wordcraft-cli {}\n\n{}", s.usage, s.details))
}

/// The overview `wordcraft-cli help` prints.
pub fn overview() -> String {
    let width = SUBCOMMANDS.iter().map(|s| first_word(s.usage).len()).max().unwrap_or_default();
    let list: Vec<String> = SUBCOMMANDS.iter().map(|s| format!("  {:<width$}  {}", first_word(s.usage), s.summary)).collect();
    format!(
        "wordcraft-cli works on WordCraft documents (.docx, .odt, .rtf, .md…) without a window.\n\n{}\n\n\
         A document is edited by running commands on it:\n\n  \
         wordcraft-cli run --cmd 'text.insert={{\"text\":\"Hello\"}}' --save hello.docx\n\n\
         `wordcraft-cli help SUBCOMMAND`, or `wordcraft-cli SUBCOMMAND --help`, explains one subcommand;\n\
         `wordcraft-cli help run` explains how to write a command; `wordcraft-cli help types` the objects some take.",
        list.join("\n")
    )
}

/// The objects some commands take, for `wordcraft-cli help types`.
pub const TYPES: &str = "\
The objects some commands take. Every field is optional: a field left out keeps what the style
chain gives. Lengths are points; `color` is \"RRGGBB\", the other colours [r, g, b] (0–255).

Pos, a position in the document (caret.set, select.range):
  {\"story\": \"body\" | {\"part\": id}, \"path\": [block] or [block, row, cell, paragraph], \"off\": byte offset}
  {\"block\": n, \"off\": n} is a shorthand for a body paragraph. Blocks and offsets count from 0;
  an offset is in bytes of the paragraph's text, and answers count the same way.
  A Story on its own (document.paragraph, select.text) is \"body\" or {\"part\": id}.

CharProps (format.set {\"props\": …}, styles.modify {\"chr\": …}):
  font, fontEa, fontCs      string, a font name
  size, sizeCs              pt
  bold, italic, strike, doubleStrike, caps, smallCaps, hidden, outline, shadow, emboss, engrave,
  noProof, rtl, boldCs, italicCs
                            bool
  underline                 \"none|single|words|double|thick|dotted|dash|dotDash|dotDotDash|wave|doubleWave\"
  underlineColor, shading   [r, g, b]
  color                     \"RRGGBB\" or \"auto\"
  highlight                 \"none|yellow|brightGreen|turquoise|pink|blue|red|darkBlue|teal|green|violet|darkRed|darkYellow|gray50|gray25|black|white\"
  border                    Border
  vertAlign                 \"baseline|superscript|subscript\"
  spacing, position         pt (character spacing; raised or lowered)
  scale                     percent, 100 = normal
  kern                      pt, kern fonts at or above this size; 0 = off
  style                     a character style id
  lang, langBidi            a BCP 47 tag, e.g. \"fi-FI\"
  link                      a URL, or \"#bookmark\"

ParaProps (para.set {\"props\": …}, styles.modify {\"para\": …}):
  align                     \"left|center|right|justify|distribute\"
  indentLeft, indentRight   pt
  indentFirst               pt; negative = hanging
  spaceBefore, spaceAfter   pt
  lineSpacing               {\"rule\": \"multiple\", \"value\": 1.15} | {\"rule\": \"atLeast\" | \"exactly\", \"value\": pt}
  contextualSpacing, mirrorIndents, keepNext, keepLines, pageBreakBefore, widowControl,
  suppressHyphens, suppressLineNumbers, bidi
                            bool
  outlineLevel              0–8 = Level 1–9, 9 = body text
  numbering                 {\"num\": n, \"level\": n}; num 0 removes numbering a style gives
  tabs                      [{\"pos\": pt, \"align\"?: \"left|center|right|decimal|bar|clear\", \"leader\"?: \"none|dot|hyphen|underscore|middleDot\"}]
  shading                   [r, g, b]
  borders                   {\"top\" | \"left\" | \"bottom\" | \"right\" | \"between\": Border}
  dropCap                   lines to drop, 0 = none
  style                     a paragraph style id

Border: {\"style\": \"none|single|double|dotted|dashed|thick|triple|dotDash|wave\", \"width\": pt, \"color\": [r, g, b], \"space\": pt}
  A border needs a width above 0 to show; space is the distance from the text.";

fn first_word(usage: &str) -> &str {
    usage.split(' ').next().unwrap_or_default()
}
