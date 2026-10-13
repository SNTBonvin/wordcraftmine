//! The editing session: document, selection, undo/redo, layout, view state.

use std::sync::{Arc, OnceLock};

/// The system locale (BCP 47 tag) the shell found at startup, for Numeral › System.
static SYSTEM_LOCALE: OnceLock<String> = OnceLock::new();

/// Tell the engine the system locale (the shell calls this once at startup; layout never reads
/// the environment). Headless sessions without it show Western digits for Numeral › System.
pub fn set_system_locale(tag: &str) {
    let _ = SYSTEM_LOCALE.set(tag.to_string());
}

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wordcraft_doc::edit::Fragment;
use wordcraft_doc::props::CharProps;
use wordcraft_doc::{Document, Path, Pos, StoryRef, Table};
use wordcraft_layout::{DocLayout, LayoutCache, LayoutOptions, NumeralMode, ViewMode};

use crate::{CmdError, Registry};

/// A selection: anchor (where it started) and focus (where the caret is).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Selection {
    pub anchor: Pos,
    pub focus: Pos,
}

impl Selection {
    pub fn caret(p: Pos) -> Self {
        Selection { anchor: p.clone(), focus: p }
    }
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }
    /// (start, end) in document order.
    pub fn ordered(&self) -> (Pos, Pos) {
        if self.anchor <= self.focus { (self.anchor.clone(), self.focus.clone()) } else { (self.focus.clone(), self.anchor.clone()) }
    }
}

/// A column (block) selection: the same x range on every line between two corners.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnBlock {
    /// The stream selection the block was made with (its corners). The block is current only
    /// while `Session::sel` still equals it, so any ordinary caret move or selection drops it.
    pub sel: Selection,
    /// Page x of the anchor and focus corners (points); the block spans the range between.
    pub anchor_x: f32,
    pub focus_x: f32,
    /// One `(start, end)` per line, in document order, each inside one paragraph.
    pub segments: Vec<(Pos, Pos)>,
    /// Set when the block is whole table cells (Select Cell, Row, Column or Table, or the mouse in
    /// a table's selection areas): one segment per paragraph of the cells.
    pub cells: Option<CellRange>,
}

/// Whole table cells selected: rows `rows.0..=rows.1` and every cell of them that reaches into
/// grid columns `cols.0..=cols.1`.
#[derive(Clone, Debug, PartialEq)]
pub struct CellRange {
    pub story: StoryRef,
    pub table: Path,
    pub rows: (usize, usize),
    pub cols: (usize, usize),
    /// The cells are one run of the text (cells of one row, or whole rows), so the stream
    /// selection between the corners holds just them and commands act on that as usual.
    /// Otherwise (part of a column) text edits and formatting go cell by cell.
    pub stream: bool,
}

impl CellRange {
    /// The selected cells of table `t` (row, cell index), in document order.
    pub fn cells(&self, t: &Table) -> Vec<(usize, usize)> {
        let ((r0, r1), (g0, g1)) = (self.rows, self.cols);
        let mut out = Vec::new();
        for (r, row) in t.rows.iter().enumerate().take(r1.saturating_add(1)).skip(r0) {
            let mut g = 0usize;
            for (c, cell) in row.cells.iter().enumerate() {
                let end = g.saturating_add(cell.span());
                if g <= g1 && end > g0 {
                    out.push((r, c));
                }
                g = end;
            }
        }
        out
    }
}

/// A selection Word's table rule widened to whole cells or rows (`cmd::table::extend_selection`),
/// and where it really runs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Widened {
    /// The selection it became; this is current while `Session::sel` still equals it.
    pub sel: Selection,
    /// Where the selection was really extended from and to: extending it again goes on from here.
    pub ends: Selection,
}

/// View state that commands can change (ribbon View tab, status bar).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewState {
    pub zoom: f32,
    pub mode: ViewMode,
    pub read_mode: bool,
    pub focus_mode: bool,
    pub marks: bool,
    pub ruler: bool,
    /// View › Show › Gridlines: the drawing grid over the page's text area (on screen only).
    pub gridlines: bool,
    /// Table Layout › View Gridlines: outlines of table cells (on screen only); on by default, as in Word.
    #[serde(default = "on")]
    pub table_gridlines: bool,
    pub nav_pane: bool,
    pub styles_pane: bool,
    /// Style Inspector pane (Home › Styles).
    #[serde(default)]
    pub style_inspector: bool,
    pub comments_pane: bool,
    /// The Clipboard pane (Home › Clipboard): items collected by Copy and Cut.
    #[serde(default)]
    pub clipboard_pane: bool,
    /// SmartArt Design › Text Pane: the selected SmartArt graphic's items as an editable outline.
    #[serde(default)]
    pub smart_art_pane: bool,
    /// The Accessibility pane (Review › Check Accessibility): the checker's findings.
    #[serde(default)]
    pub accessibility_pane: bool,
    /// The Alt Text pane (Picture/Shape Format › Alt Text): the selected object's or table's alt text.
    #[serde(default)]
    pub alt_text_pane: bool,
    pub multi_page: bool,
    /// One Page or Page Width was chosen last: keep one page per row when a typed or wheel zoom replaces the fit mode.
    #[serde(default)]
    pub single_column: bool,
    /// Zoom to fit: "pageWidth", "onePage", "multiplePages", or empty.
    pub fit: String,
    pub web_width: f32,
    /// Transient viewport metrics for reading pagination; never saved in a document or preferences.
    #[serde(skip, default = "default_read_width")]
    pub read_width: f32,
    #[serde(skip, default = "default_read_height")]
    pub read_height: f32,
    /// Reader-only layout controls; they are neither document formatting nor saved UI prefs.
    /// `None` chooses responsive column width and keeps the document's line spacing.
    #[serde(skip)]
    pub read_column_width: Option<f32>,
    #[serde(skip, default = "default_read_zoom")]
    pub read_zoom: f32,
    #[serde(skip)]
    pub read_line_spacing: Option<f32>,
    pub dark_mode: bool,
    pub show_markup: bool,
    pub track_changes_pane: bool,
    /// Check spelling and grammar as you type.
    pub proofing: bool,
    /// How digits look on screen (Word › File › Options › Advanced › Numeral).
    #[serde(default)]
    pub numeral: NumeralMode,
    /// Review › Hide Ink: ink strokes aren't shown on screen (they stay in the document).
    #[serde(default)]
    pub hide_ink: bool,
    /// Draw tab: the tool dragging on the page uses, and each pen's colour and thickness.
    #[serde(default)]
    pub draw: crate::cmd::draw::DrawState,
    /// Show field codes instead of their results (`view.fieldCodes`, Alt+F9).
    #[serde(default)]
    pub field_codes: bool,
    /// Print Layout › Hide White Space: pages run together, the top and bottom margins (with
    /// headers and footers) between pages collapse to a thin line. On screen only; the layout
    /// and page count are unchanged.
    #[serde(default)]
    pub hide_white_space: bool,
    /// Outline view: collapsed headings, Show Level, first lines, text formatting.
    #[serde(default)]
    pub outline: crate::cmd::outline::OutlineState,
    /// Developer › Design Mode: every content control shows its frame and title.
    #[serde(default)]
    pub design_mode: bool,
}

fn on() -> bool {
    true
}

fn default_read_width() -> f32 {
    300.0
}

fn default_read_height() -> f32 {
    500.0
}

fn default_read_zoom() -> f32 {
    1.0
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            zoom: 1.0,
            mode: ViewMode::Print,
            read_mode: false,
            focus_mode: false,
            marks: false,
            ruler: true,
            gridlines: false,
            table_gridlines: true,
            nav_pane: false,
            styles_pane: false,
            style_inspector: false,
            comments_pane: false,
            clipboard_pane: false,
            smart_art_pane: false,
            accessibility_pane: false,
            alt_text_pane: false,
            multi_page: false,
            single_column: false,
            fit: String::new(),
            web_width: 800.0,
            read_width: default_read_width(),
            read_height: default_read_height(),
            read_column_width: None,
            read_zoom: default_read_zoom(),
            read_line_spacing: None,
            dark_mode: false,
            show_markup: true,
            track_changes_pane: false,
            proofing: true,
            numeral: NumeralMode::default(),
            hide_ink: false,
            draw: Default::default(),
            field_codes: false,
            hide_white_space: false,
            outline: Default::default(),
            design_mode: false,
        }
    }
}

/// Find/replace state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FindState {
    pub query: String,
    pub replace: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    /// Word's "Use wildcards" (#543).
    pub wildcards: bool,
    /// Take Find what literally: no `^` codes (`special: false`).
    pub literal: bool,
    pub match_prefix: bool,
    pub match_suffix: bool,
    pub ignore_punct: bool,
    pub ignore_space: bool,
    /// Matches from the last search: (start, end).
    #[serde(skip)]
    pub results: Vec<(Pos, Pos)>,
    pub current: usize,
    /// Reading Highlight: mark every match of `query` in the body.
    pub highlight: bool,
    /// Highlighted matches, for the document revision they were found in.
    #[serde(skip)]
    pub highlights: Option<(u64, Vec<(Pos, Pos)>)>,
}

#[derive(Clone)]
struct Undo {
    label: String,
    doc: Document,
    sel: Selection,
    /// Formatting waiting at the caret (Bold with nothing selected), so Undo takes it back too.
    pending: Option<CharProps>,
    /// The column block or whole cells the selection was (undoing brings the selection back as
    /// it was).
    column: Option<ColumnBlock>,
}

/// What [`Session::restore`] needs to take commands back exactly: the document, the selection,
/// the undo and redo stacks (also the oldest undo steps the undo limit pushes out meanwhile), the
/// open typing group, a pending [`Session::join_next_undo`] and the dirty flag. See
/// [`Session::edit_snapshot`].
pub struct EditSnapshot {
    doc: Document,
    sel: Selection,
    history_len: usize,
    /// `Session::undo_evicted` when the snapshot was taken.
    evicted: u64,
    /// The oldest undo steps, kept only when the stack is near its limit (the commands that
    /// follow may push them out); at most [`SNAPSHOT_HEAD`].
    head: Vec<Arc<Undo>>,
    redo: Vec<Arc<Undo>>,
    typing_open: bool,
    join_next: bool,
    dirty: bool,
}

/// Undo steps that the commands run between [`Session::edit_snapshot`] and [`Session::restore`]
/// may push out of a full stack and still come back.
const SNAPSHOT_HEAD: usize = 4;

impl EditSnapshot {
    /// The document when the snapshot was taken.
    pub fn doc(&self) -> &Document {
        &self.doc
    }
    /// The dirty flag then.
    pub fn dirty(&self) -> bool {
        self.dirty
    }
    /// The number of undo steps then.
    pub fn undo_depth(&self) -> usize {
        self.history_len
    }
}

/// An editing session.
pub struct Session {
    pub doc: Document,
    pub sel: Selection,
    pub view: ViewState,
    /// Formatting picked with a collapsed caret, applied to the next typed text.
    pub pending: Option<CharProps>,
    pub path: Option<std::path::PathBuf>,
    pub dirty: bool,
    /// Where the custom dictionary is kept on disk, if the host wired one up. Only the desktop
    /// app does: headless sessions (MCP, `wordcraft-cli run`), tests and the web build leave
    /// `None`, so an agent's words stay in memory and never change the person's dictionary.
    /// Add to Dictionary and File › Options › Proofing save through it (#40).
    pub user_dict_path: Option<std::path::PathBuf>,
    pub clipboard: Option<Fragment>,
    /// A tracked cut the next paste can turn into a move (see `cmd::moves`).
    pub pending_move: Option<crate::cmd::moves::PendingMove>,
    /// Plain text mirror of the clipboard (for the system clipboard).
    pub clipboard_text: String,
    /// Items collected by Copy and Cut this session, for the Clipboard pane.
    pub clip_history: crate::cmd::edit::ClipHistory,
    pub find: FindState,
    /// Page x the caret tries to keep on Up/Down.
    pub goal_x: Option<f32>,
    /// Page the caret was last on (headers/footers repeat on many pages).
    pub page_hint: usize,
    pub registry: Arc<Registry>,
    /// Author name for comments and tracked changes.
    pub author: String,
    /// Initials for comments (File › Options › General); empty = the name's initials.
    pub initials: String,
    /// Format painter: copied formatting waiting to be applied (and whether it stays on).
    pub painter: Option<(CharProps, wordcraft_doc::props::ParaProps, bool)>,
    /// Last message for the status bar / agents.
    pub status: String,
    /// Undo and redo steps. Shared so `run` can snapshot both stacks cheaply (a pointer per
    /// step) and put them back exactly when a command fails.
    history: Vec<Arc<Undo>>,
    /// Undo steps dropped so far because of the undo limit.
    undo_evicted: u64,
    redo: Vec<Arc<Undo>>,
    /// Typing is coalesced into one undo step until something else happens.
    typing_open: bool,
    /// The next mutating command joins the previous undo step (later frames of a drag).
    join_next: bool,
    rev: u64,
    /// Which document this is: changes when [`Session::set_document`] replaces it (open, new,
    /// mail merge, recover), never on an edit.
    document_id: u64,
    cache: LayoutCache,
    /// (revision, width, read height, view, read line spacing, layout, proofing, outline,
    /// (deletions left out, body deletions in balloons), numeral display).
    #[allow(clippy::type_complexity)]
    layout: Option<(
        u64,
        f32,
        f32,
        ViewMode,
        Option<u32>,
        Arc<DocLayout>,
        bool,
        Option<wordcraft_layout::outline::OutlineView>,
        (bool, bool),
        NumeralMode,
    )>,
    /// Picture edits: edited media key → original media key (Reset Picture).
    pub originals: std::collections::HashMap<String, String>,
    /// Last mutating command (Repeat).
    pub last_command: Option<(String, Value)>,
    /// Macro being recorded: (name, steps).
    pub recording: Option<(String, Vec<(String, Value)>)>,
    pub macros: std::collections::BTreeMap<String, Vec<(String, Value)>>,
    /// Saved versions: (label, date, document).
    pub versions: Vec<(String, String, Document)>,
    /// Quick Parts / AutoText entries.
    pub building_blocks: std::collections::BTreeMap<String, Fragment>,
    pub autosave: bool,
    /// Citation style: APA, MLA, Chicago, IEEE.
    pub bib_style: String,
    /// Mail merge data source and preview.
    pub merge: crate::cmd::mailings::MergeState,
    /// Requests from commands to the UI (open a dialog, scroll…), drained by the front end.
    pub ui_requests: Vec<Value>,
    /// Read Aloud player (Review › Speech).
    pub read_aloud: crate::speech::ReadAloud,
    /// Preferences the front end saves between runs.
    pub prefs: Prefs,
    /// The system region (#838): the paper a new blank document starts on, the default
    /// measurement unit and the decimal separator. The front end sets it from the platform (the
    /// browser's locale on the web); headless sessions keep `en-US`. A region the user chose
    /// ([`Prefs::region`]) wins: read it with [`Session::region`].
    pub system_region: wordcraft_geom::Region,
    /// Custom keyboard shortcuts (`tools.customizeKeyboard`); the front end saves them too.
    pub keymap: crate::KeyMap,
    /// Custom ribbon and Quick Access Toolbar (`tools.customizeRibbon`); the front end saves it too.
    pub ribbon: crate::RibbonLayout,
    /// The language new blank documents (and opened plain text) are written in, a BCP 47 tag;
    /// `None` is WordCraft's default, US English. The front end sets it from the interface
    /// language with [`Session::set_editing_language`] (a Polish interface writes Polish).
    pub editing_lang: Option<String>,
    /// Editing inside an equation: which one and the caret in it.
    pub math: Option<MathEdit>,
    /// Equations are typed in LaTeX rather than the linear format.
    pub math_latex: bool,
    /// Text typed into equations is normal (non-math) text.
    pub math_normal_text: bool,
    /// The password Word documents are saved with (File › Info › Protect Document › Encrypt with
    /// Password, `file.encrypt`); `None` saves them unencrypted. Kept from a password-protected
    /// file that was opened, cleared when the document is replaced.
    pub password: Option<crate::io::Password>,
    /// More pictures and shapes selected along with the one the selection holds (Shift+click),
    /// for Group. Cleared by any edit or selection change other than adding to it.
    pub also_selected: Vec<Pos>,
    /// Column (block) selection, if one was made (see [`Session::column_segments`]).
    pub column: Option<ColumnBlock>,
    /// Column selection mode (Ctrl+Shift+F8): caret movement extends the block.
    pub column_mode: bool,
    /// A selection widened to whole table cells or rows, and where it really runs (see
    /// [`Session::sel_ends`]).
    pub(crate) widened: Option<Widened>,
    /// Where the last edits happened, oldest first (at most [`EDIT_MARKS`]), for Go Back
    /// (Shift+F5). Positions aren't mapped through later edits; they're clamped when used.
    edit_marks: Vec<Pos>,
    /// How many Go Back presses since the last edit (which place comes next).
    go_back_step: usize,
}

/// The number of edit locations Go Back cycles through (Word: four).
const EDIT_MARKS: usize = 4;

/// The equation being edited.
#[derive(Clone, Debug, PartialEq)]
pub struct MathEdit {
    /// The equation object (its U+FFFC).
    pub at: Pos,
    /// The caret inside it.
    pub pos: wordcraft_doc::math_edit::MathPos,
}

/// Editing preferences that persist between runs (the front end saves and restores them).
/// File › Options sets most of them (`file.options`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// Word Count includes text boxes, footnotes and endnotes (Word's default).
    pub count_notes: bool,
    /// Track Changes Options: what markup shows and how revisions are drawn (per user, as in
    /// Word).
    pub markup: wordcraft_layout::display::MarkupOptions,
    /// File › Print / Export: Print Markup. PDFs, prints and page images show comments in
    /// balloons beside each page and tracked changes as marked up; HTML, Markdown and text
    /// copies end with the comments (`file.printMarkup`).
    pub print_markup: bool,
    /// Language › Detect language automatically (per user, as in Word).
    pub detect_language: bool,
    /// Typing over a selection replaces it; off, the typed text goes in front of it.
    pub typing_replaces_selection: bool,
    /// The Insert key switches overtype on and off.
    pub insert_key_overtype: bool,
    /// Overtype: typed characters replace the ones after the caret.
    pub overtype: bool,
    /// The unit rulers and dialogs show lengths in, when the user chose one (File › Options ›
    /// Advanced › Measurement units); `None` follows the region ([`Session::units`]). Saved under
    /// a new key; an older `units` value other than inches (the old default) carries over on load.
    #[serde(rename = "measurementUnits")]
    pub units: Option<wordcraft_geom::Unit>,
    /// The region the user chose for new documents, units and the decimal separator (a locale
    /// tag like `pl-PL`, File › Options › Advanced › Region); `None` follows the system.
    pub region: Option<String>,
    /// Formatting marks shown even while Show/Hide ¶ is off.
    pub marks: wordcraft_layout::display::FormattingMarks,
    /// Mark grammar errors as you type (with Check spelling as you type on).
    pub mark_grammar: bool,
    pub ignore_uppercase: bool,
    pub ignore_numbers: bool,
    pub ignore_internet: bool,
    /// AutoCorrect Options: replacements, capitalization and AutoFormat As You Type (per user, as
    /// in Word). A damaged saved value reads as the defaults.
    #[serde(deserialize_with = "crate::cmd::tools::lenient_autocorrect")]
    pub autocorrect: crate::cmd::tools::AutoCorrectPrefs,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            count_notes: true,
            markup: Default::default(),
            print_markup: false,
            typing_replaces_selection: true,
            insert_key_overtype: false,
            overtype: false,
            units: Default::default(),
            region: None,
            marks: Default::default(),
            mark_grammar: true,
            ignore_uppercase: true,
            ignore_numbers: true,
            ignore_internet: true,
            detect_language: true,
            autocorrect: Default::default(),
        }
    }
}

impl Prefs {
    /// What the spelling check skips.
    pub fn spelling(&self) -> wordcraft_proof::Options {
        wordcraft_proof::Options {
            ignore_uppercase: self.ignore_uppercase,
            ignore_numbers: self.ignore_numbers,
            ignore_internet: self.ignore_internet,
        }
    }
}

/// The citation style a document records (References › Style), APA when it records none or
/// one we don't know.
fn doc_bib_style(doc: &Document) -> String {
    doc.settings.bib_style.as_deref().and_then(crate::cmd::citations::find_style).map_or("APA", |st| st.name).to_string()
}

/// Maximum undo depth.
const MAX_UNDO: usize = 500;
/// How many steps the Undo button's list shows (#701); `edit.undoTo` reaches no further.
pub const UNDO_LIST_MAX: usize = 100;

impl Session {
    /// File name of the persisted custom dictionary inside the settings folder.
    pub const USER_DICT_FILE: &str = "custom-dictionary.txt";

    /// Save the custom dictionary through [`Session::user_dict_path`], if the host wired one up,
    /// with `removed` (words just taken out) left out of the merge with the file. A failed save
    /// keeps the words in memory and never fails the command (#40).
    pub(crate) fn persist_user_dictionary(&self, removed: &[String]) {
        if let Some(path) = self.user_dict_path.as_deref()
            && let Err(e) = wordcraft_proof::save_user_dictionary(path, removed)
        {
            log::warn!("custom dictionary: save failed: {e}");
        }
    }

    pub fn new(doc: Document) -> Self {
        let start = doc.start_of(StoryRef::Body);
        let bib_style = doc_bib_style(&doc);
        Session {
            doc,
            sel: Selection::caret(start),
            view: ViewState::default(),
            pending: None,
            path: None,
            user_dict_path: None,
            dirty: false,
            clipboard: None,
            pending_move: None,
            clipboard_text: String::new(),
            clip_history: Default::default(),
            find: FindState::default(),
            goal_x: None,
            page_hint: 0,
            registry: Arc::new(crate::cmd::registry()),
            author: "WordCraft User".into(),
            initials: String::new(),
            painter: None,
            status: String::new(),
            history: Vec::new(),
            undo_evicted: 0,
            redo: Vec::new(),
            typing_open: false,
            join_next: false,
            rev: 1,
            document_id: 1,
            cache: LayoutCache::new(),
            layout: None,
            originals: Default::default(),
            last_command: None,
            recording: None,
            macros: Default::default(),
            versions: Vec::new(),
            building_blocks: Default::default(),
            autosave: true,
            bib_style,
            merge: Default::default(),
            ui_requests: Vec::new(),
            math: None,
            math_latex: false,
            math_normal_text: false,
            also_selected: Vec::new(),
            prefs: Prefs::default(),
            system_region: Default::default(),
            keymap: crate::KeyMap::default(),
            ribbon: crate::RibbonLayout::default(),
            editing_lang: None,
            read_aloud: Default::default(),
            password: None,
            column: None,
            column_mode: false,
            widened: None,
            edit_marks: Vec::new(),
            go_back_step: 0,
        }
    }

    /// Remember the caret as the place of an edit, for Go Back. Edits in the same paragraph as
    /// the last one (typing on) update that place rather than adding one.
    fn note_edit(&mut self) {
        let at = self.sel.focus.clone();
        match self.edit_marks.last_mut() {
            Some(last) if last.story == at.story && last.path == at.path => *last = at,
            _ => {
                self.edit_marks.push(at);
                if self.edit_marks.len() > EDIT_MARKS {
                    self.edit_marks.remove(0);
                }
            }
        }
        self.go_back_step = 0;
    }

    /// Go Back (Shift+F5): the next of the last edit places, most recent first, cycling; a place
    /// the caret is already at is skipped. `None` before any edit.
    pub fn go_back(&mut self) -> Option<Pos> {
        let n = self.edit_marks.len();
        for _ in 0..n {
            let i = n - 1 - self.go_back_step % n;
            self.go_back_step = self.go_back_step.wrapping_add(1);
            let Some(p) = self.edit_marks.get(i) else { continue };
            let p = self.doc.clamp(p);
            if p != self.sel.focus || n == 1 {
                return Some(p);
            }
        }
        self.edit_marks.last().map(|p| self.doc.clamp(p))
    }

    /// The line pieces of the current column selection, if there is one (the selection hasn't
    /// moved since the block was made).
    pub fn column_segments(&self) -> Option<&[(Pos, Pos)]> {
        // Cells that are one run of the text are an ordinary stream selection.
        self.column.as_ref().filter(|c| c.sel == self.sel && !c.cells.as_ref().is_some_and(|r| r.stream)).map(|c| c.segments.as_slice())
    }

    /// The whole table cells selected, if the selection is that (see [`CellRange`]).
    pub fn cell_range(&self) -> Option<&CellRange> {
        self.column.as_ref().filter(|c| c.sel == self.sel).and_then(|c| c.cells.as_ref())
    }

    /// The selection as it was made: where a drag, Shift+click or Shift+arrows started and where
    /// the caret went, even when Word's table rule widened it to whole cells or rows (`sel` holds
    /// those).
    pub fn sel_ends(&self) -> &Selection {
        self.widened.as_ref().filter(|w| w.sel == self.sel).map_or(&self.sel, |w| &w.ends)
    }

    /// Document revision (bumped by every change).
    pub fn rev(&self) -> u64 {
        self.rev
    }
    /// Which document is open (see `document_id`): lets a caller tell an edit from a replacement.
    pub fn document_id(&self) -> u64 {
        self.document_id
    }
    pub fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
        self.dirty = true;
    }

    /// Reading Highlight ranges for the current document (searched again after edits); empty when it's off.
    pub fn find_highlights(&mut self) -> &[(Pos, Pos)] {
        if !self.find.highlight {
            return &[];
        }
        if self.find.highlights.as_ref().is_none_or(|(r, _)| *r != self.rev) {
            let found = crate::cmd::edit::search(self, StoryRef::Body).unwrap_or_default();
            self.find.highlights = Some((self.rev, found));
        }
        self.find.highlights.as_ref().map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    /// Whether the markup area beside the page can show balloons: markup shown in Print Layout
    /// (not Read Mode or pages side by side), comments not listed in the Comments pane instead,
    /// and Track Changes Options not set to show every revision inline.
    pub fn balloons_shown(&self) -> bool {
        let v = &self.view;
        v.show_markup
            && !v.comments_pane
            && !v.read_mode
            && !v.multi_page
            && v.mode == ViewMode::Print
            && self.prefs.markup.balloons != wordcraft_layout::display::BalloonMode::Inline
    }

    /// Show revisions in balloons, with the balloons on screen: tracked deletions in the body leave
    /// the text for balloons in the markup area. Where no balloons can be shown they stay inline,
    /// as in Word; so do deletions in headers, footers, notes and text boxes, which get no balloons.
    pub fn revisions_in_balloons(&self) -> bool {
        let m = &self.prefs.markup;
        self.balloons_shown() && m.balloons == wordcraft_layout::display::BalloonMode::Revisions && m.insertions_deletions
    }

    /// Initials for new comments: the ones set in File › Options, else the user name's.
    pub fn user_initials(&self) -> String {
        let set = self.initials.trim();
        if set.is_empty() { self.author.split_whitespace().filter_map(|w| w.chars().next()).collect() } else { set.to_string() }
    }

    /// The current layout (recomputed when the document or view changed).
    pub fn layout(&mut self) -> Arc<DocLayout> {
        let mode = if self.view.read_mode { ViewMode::Read } else { self.view.mode };
        let width = if self.view.read_mode { self.view.read_width } else { self.view.web_width };
        let height = if self.view.read_mode { self.view.read_height } else { 0.0 };
        let spacing = if self.view.read_mode { self.view.read_line_spacing.map(f32::to_bits) } else { None };
        // Deletions in balloons depend on the view (the Comments pane, Read Mode, side by side), so
        // a view change that moves them in or out of the text lays it out again.
        let hide_deleted = !self.view.show_markup || self.prefs.markup.hides_deletions();
        let deleted = (hide_deleted, self.revisions_in_balloons());
        let outline = (mode == ViewMode::Outline).then(|| self.view.outline.view());
        if let Some((r, w, h, m, sp, l, pf, o, hd, n)) = &self.layout
            && *r == self.rev
            && (*w == width || mode == ViewMode::Print)
            && *h == height
            && *m == mode
            && *sp == spacing
            && *pf == self.view.proofing
            && *n == self.view.numeral
            && *hd == deleted
            && *o == outline
        {
            return l.clone();
        }
        let opts = LayoutOptions {
            view: mode,
            web_width: width,
            read_height: height,
            read_line_spacing: spacing.map(f32::from_bits),
            show_hidden: self.view.marks || self.prefs.marks.hidden,
            hide_deleted,
            deletions_in_balloons: deleted.1,
            proofing: self.view.proofing,
            numeral: self.view.numeral.resolve(SYSTEM_LOCALE.get().map(String::as_str)),
            comment_balloons: false,
            field_codes: self.view.field_codes,
            outline: outline.clone(),
            proof: wordcraft_layout::ProofOptions { grammar: self.prefs.mark_grammar, spelling: self.prefs.spelling() },
        };
        let l = Arc::new(wordcraft_layout::layout(&self.doc, &mut self.cache, &opts));
        self.layout = Some((self.rev, width, height, mode, spacing, l.clone(), self.view.proofing, outline, deleted, self.view.numeral));
        l
    }
    /// A layout for output (PDF, images, print): no proofing marks, print view.
    pub fn export_layout(&self) -> Arc<DocLayout> {
        self.export_layout_with(false)
    }

    /// [`Session::export_layout`]; with `markup` (Print Markup) each page also has the markup
    /// area with the comments in balloons.
    pub fn export_layout_with(&self, markup: bool) -> Arc<DocLayout> {
        let opts = LayoutOptions { view: ViewMode::Print, comment_balloons: markup && self.prefs.markup.comments, ..Default::default() };
        Arc::new(wordcraft_layout::layout(&self.doc, &mut LayoutCache::new(), &opts))
    }

    /// Invalidate the cached layout (fonts changed etc.).
    pub fn relayout(&mut self) {
        self.layout = None;
        self.cache.clear();
    }

    /// Set the language new documents are written in ([`Session::editing_lang`]; `None` or a
    /// malformed tag: US English). A document nobody has touched yet — untitled, unchanged, no
    /// undo steps, no text — switches too, so the blank document a session starts with follows
    /// the interface language. Nothing else changes and no undo step is added.
    pub fn set_editing_language(&mut self, tag: Option<&str>) {
        let tag = tag.and_then(wordcraft_proof::lang::normalize_tag);
        if self.editing_lang == tag {
            return;
        }
        self.editing_lang = tag;
        let untouched = self.path.is_none() && !self.dirty && self.history.is_empty() && self.redo.is_empty();
        if untouched && self.doc.plain_text(StoryRef::Body).trim().is_empty() && self.doc.paragraph_count() <= 1 {
            self.doc.styles.default_chr.lang = Some(self.editing_lang.clone().unwrap_or_else(|| "en-US".into()));
            self.rev = self.rev.wrapping_add(1);
            self.relayout();
        }
    }

    /// Snapshot for undo before a change.
    pub fn checkpoint(&mut self, label: &str) {
        if label == "Typing" && self.typing_open {
            return;
        }
        self.typing_open = label == "Typing";
        self.history.push(Arc::new(Undo {
            label: label.to_string(),
            doc: self.doc.clone(),
            sel: self.sel.clone(),
            pending: self.pending.clone(),
            column: self.column.clone(),
        }));
        if self.history.len() > MAX_UNDO {
            self.history.remove(0);
            self.undo_evicted += 1;
        }
        self.redo.clear();
    }
    /// Record `doc`/`sel` as their own undo step after the fact and close the typing group, so
    /// Undo right after an automatic change (AutoFormat, AutoCorrect) reverts only that change.
    pub fn push_undo(&mut self, label: &str, doc: Document, sel: Selection) {
        self.history.push(Arc::new(Undo { label: label.to_string(), doc, sel, pending: self.pending.clone(), column: None }));
        if self.history.len() > MAX_UNDO {
            self.history.remove(0);
            self.undo_evicted += 1;
        }
        self.typing_open = false;
    }
    /// Make the next mutating command part of the previous undo step instead of a new one, so a
    /// drag that runs a command every frame is a single Undo. Call it on every frame of the drag
    /// except the first; it is consumed by the next `run`.
    pub fn join_next_undo(&mut self) {
        self.join_next = true;
    }
    /// Stamp the document's save metadata (modified time, last modified by, revision; created
    /// when it has none) the way Word does on every save. `file.save` calls it, and so does any
    /// front end that writes the bytes itself (the web build's download, #262).
    pub fn stamp_save(&mut self) {
        let core = &mut self.doc.core;
        core.modified = crate::cmd::now_iso();
        if core.created.is_empty() {
            core.created = core.modified.clone();
        }
        core.last_modified_by = self.author.clone();
        core.revision = core.revision.saturating_add(1);
    }
    /// Close an open typing group (caret moved, other command).
    pub fn close_typing(&mut self) {
        self.typing_open = false;
    }
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.history.last().map(|u| u.label.as_str())
    }
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|u| u.label.as_str())
    }
    /// The number of undo steps.
    pub fn undo_depth(&self) -> usize {
        self.history.len()
    }
    pub fn undo_labels(&self) -> Vec<String> {
        self.history.iter().rev().map(|u| u.label.clone()).collect()
    }
    /// The Undo button's list (#701): the undo steps' labels (English, as commands name them),
    /// newest first, at most [`UNDO_LIST_MAX`].
    pub fn undo_history(&self) -> Vec<&str> {
        self.history.iter().rev().take(UNDO_LIST_MAX).map(|u| u.label.as_str()).collect()
    }
    /// Undo the newest `count` steps in one go, as picking an entry in the Undo list does (#701).
    /// More than the undo depth undoes everything; a step that would take the document's
    /// protection away stops it (only Stop Protection lifts it). Returns the steps undone.
    pub fn undo_steps(&mut self, count: usize) -> usize {
        let mut done = 0;
        while done < count && !self.step_lifts_protection(false) && self.undo() {
            done += 1;
        }
        done
    }
    pub fn undo(&mut self) -> bool {
        self.typing_open = false;
        let Some(u) = self.history.pop().map(Arc::unwrap_or_clone) else { return false };
        let cur = Undo {
            label: u.label.clone(),
            doc: std::mem::replace(&mut self.doc, u.doc),
            sel: std::mem::replace(&mut self.sel, u.sel),
            pending: std::mem::replace(&mut self.pending, u.pending),
            column: std::mem::replace(&mut self.column, u.column),
        };
        self.redo.push(Arc::new(cur));
        self.touch();
        true
    }
    /// Would Undo (or Redo) take the document's protection away? Only Stop Protection lifts it.
    pub fn step_lifts_protection(&self, redo: bool) -> bool {
        let next = if redo { self.redo.last() } else { self.history.last() };
        next.is_some_and(|u| crate::cmd::protect::protection_lifted(&self.doc, &u.doc))
    }
    pub fn redo(&mut self) -> bool {
        self.typing_open = false;
        let Some(u) = self.redo.pop().map(Arc::unwrap_or_clone) else { return false };
        let cur = Undo {
            label: u.label.clone(),
            doc: std::mem::replace(&mut self.doc, u.doc),
            sel: std::mem::replace(&mut self.sel, u.sel),
            pending: std::mem::replace(&mut self.pending, u.pending),
            column: std::mem::replace(&mut self.column, u.column),
        };
        self.history.push(Arc::new(cur));
        self.touch();
        true
    }
    /// Drop history (after open / new).
    pub fn reset_history(&mut self) {
        self.history.clear();
        self.redo.clear();
        self.typing_open = false;
    }

    /// The region in effect (#838): the one the user chose ([`Prefs::region`]), else the
    /// system's.
    pub fn region(&self) -> wordcraft_geom::Region {
        match self.prefs.region.as_deref() {
            Some(tag) => wordcraft_geom::Region::from_tag(tag),
            None => self.system_region.clone(),
        }
    }

    /// The unit rulers and dialogs show lengths in: the user's choice, else the region's.
    pub fn units(&self) -> wordcraft_geom::Unit {
        self.prefs.units.unwrap_or_else(|| self.region().units())
    }

    /// `pt` in [`Session::units`], written the region's way (`2,54 cm` in Poland).
    pub fn format_length(&self, pt: f32) -> String {
        self.region().format(self.units(), pt)
    }

    /// A new blank document (the Normal template) on the region's paper: Letter in the US and
    /// other Letter regions, A4 elsewhere (#838). Margins stay 2.54 cm (1") all round.
    pub fn blank_document(&self) -> Document {
        blank_document(&self.region())
    }

    /// Replace the document (open/new).
    pub fn set_document(&mut self, doc: Document) {
        self.document_id = self.document_id.wrapping_add(1);
        self.bib_style = doc_bib_style(&doc);
        self.doc = doc;
        self.doc.ensure_nonempty();
        self.password = None;
        self.sel = Selection::caret(self.doc.start_of(StoryRef::Body));
        self.pending = None;
        self.also_selected.clear();
        self.column = None;
        self.column_mode = false;
        self.widened = None;
        self.edit_marks.clear();
        self.go_back_step = 0;
        self.view.outline.collapsed.clear();
        self.reset_history();
        self.touch();
        self.dirty = false;
        self.relayout();
    }

    /// A snapshot to put back with [`Session::restore`], for a caller that runs a command and
    /// may want to take it back without a trace (cheap: the document's blocks are shared).
    ///
    /// Covered: `doc`, `sel`, the undo stack (its length, and its oldest steps when it is near
    /// the limit), `redo`, the open typing group, a pending `join_next_undo` and `dirty`. `rev`,
    /// `cache` and `layout` are derived (`restore` bumps `rev`). The other fields are not
    /// covered: a caller that lets the command change them (the view, the find state, the
    /// clipboard, macros, the file path…) saves and puts them back itself.
    ///
    /// `restore` is exact when the commands in between only add undo steps (every mutating
    /// command adds at most one; up to [`SNAPSHOT_HEAD`] may push old steps out of a full
    /// stack). Taking steps off the stack (`edit.undo`) or clearing it (`file.new`,
    /// `file.open`) in between is not covered.
    ///
    /// The destructuring lists every field, so a new field does not build until someone decides
    /// whether a snapshot must cover it.
    pub fn edit_snapshot(&self) -> EditSnapshot {
        let Session {
            doc,
            sel,
            view: _,
            pending: _,
            path: _,
            // The custom dictionary file, like the file path: host wiring, not an edit.
            user_dict_path: _,
            dirty,
            clipboard: _,
            // Like the clipboard: what the next paste can make a move of.
            pending_move: _,
            clipboard_text: _,
            clip_history: _,
            find: _,
            goal_x: _,
            page_hint: _,
            registry: _,
            author: _,
            initials: _,
            painter: _,
            status: _,
            history,
            undo_evicted,
            redo,
            typing_open,
            join_next,
            rev: _,
            cache: _,
            layout: _,
            originals: _,
            last_command: _,
            recording: _,
            macros: _,
            versions: _,
            building_blocks: _,
            autosave: _,
            bib_style: _,
            merge: _,
            ui_requests: _,
            document_id: _,
            read_aloud: _,
            prefs: _,
            system_region: _,
            keymap: _,
            ribbon: _,
            // A preference for new documents, like `prefs`: not part of this document.
            editing_lang: _,
            // Equation editing mode, like `view`: not part of the document or its history.
            math: _,
            math_latex: _,
            math_normal_text: _,
            // The save password, like the file path: not an edit to the document.
            password: _,
            also_selected: _,
            // Column selection and a widened selection's ends, like `sel`'s shape: valid only while
            // `sel` matches them.
            column: _,
            column_mode: _,
            widened: _,
            // Go Back's places, like the caret history: not part of the document.
            edit_marks: _,
            go_back_step: _,
        } = self;
        let head = if history.len() + SNAPSHOT_HEAD > MAX_UNDO { history.iter().take(SNAPSHOT_HEAD).cloned().collect() } else { Vec::new() };
        EditSnapshot {
            doc: doc.clone(),
            sel: sel.clone(),
            history_len: history.len(),
            evicted: *undo_evicted,
            head,
            redo: redo.clone(),
            typing_open: *typing_open,
            join_next: *join_next,
            dirty: *dirty,
        }
    }

    /// Put a snapshot back: the commands run since leave no trace in the document, the
    /// selection or undo/redo (undo steps the limit pushed out meanwhile come back). The layout
    /// is recomputed.
    pub fn restore(&mut self, snap: EditSnapshot) {
        let EditSnapshot { doc, sel, history_len, evicted, head, redo, typing_open, join_next, dirty } = snap;
        let gone = usize::try_from(self.undo_evicted.saturating_sub(evicted)).unwrap_or(usize::MAX);
        if gone > 0 {
            let back = gone.min(head.len());
            self.history.splice(0..0, head.into_iter().take(back));
        }
        self.undo_evicted = evicted;
        self.doc = doc;
        self.sel = sel;
        self.also_selected.clear();
        self.history.truncate(history_len);
        self.redo = redo;
        self.typing_open = typing_open;
        self.join_next = join_next;
        self.touch();
        self.dirty = dirty;
    }

    /// Make the selection valid for the current document.
    pub fn clamp_selection(&mut self) {
        self.sel.anchor = self.doc.clamp(&self.sel.anchor);
        self.sel.focus = self.doc.clamp(&self.sel.focus);
        // The equation being edited must still be there (undo, edits elsewhere).
        if let Some(m) = &self.math {
            let eq = self.doc.para_at(&m.at).and_then(|p| match p.object_at(m.at.off) {
                Some(wordcraft_doc::InlineObject::Equation { math, .. }) => Some(wordcraft_doc::math_edit::clamp(&math.nodes, &m.pos)),
                _ => None,
            });
            match eq {
                Some(pos) => {
                    if let Some(m) = self.math.as_mut() {
                        m.pos = pos;
                    }
                }
                None => self.math = None,
            }
        }
    }

    /// Run a command by id with JSON params (the single entry point for the UI, CLI, MCP and
    /// control channel). Mutating commands are undoable; a failed command leaves the document as
    /// it was; a panic inside a command becomes an error.
    pub fn run(&mut self, id: &str, params: &Value) -> Result<Value, CmdError> {
        let join = std::mem::take(&mut self.join_next);
        let reg = self.registry.clone();
        let Some(spec) = reg.get(id) else { return Err(CmdError::Unknown(id.to_string())) };
        if let Some(why) = (spec.enabled)(self) {
            return Err(CmdError::Disabled(format!("{id}: {why}")));
        }
        // Restrict Editing: refused outright, or allowed as long as it only edits the exceptions.
        let guard = if spec.mutates { crate::cmd::protect::check_before(self, id, params)? } else { false };
        if spec.mutates && self.doc.settings.protection.as_deref() == Some("trackedChanges") {
            self.doc.settings.track_changes = true;
        }
        // Macro recording and Repeat.
        let record = !matches!(id, "tools.recordMacro" | "tools.macros" | "edit.undo" | "edit.redo" | "edit.repeat")
            && (spec.mutates || id.starts_with("caret.") || id.starts_with("select."));
        // A password never lands in a recorded macro or Repeat.
        let kept = || {
            let mut p = params.clone();
            if let Some(m) = p.as_object_mut() {
                m.remove("password");
            }
            p
        };
        if record && let Some((_, steps)) = self.recording.as_mut() {
            steps.push((id.to_string(), kept()));
        }
        if spec.mutates && !matches!(id, "edit.undo" | "edit.redo" | "edit.repeat") {
            self.last_command = Some((id.to_string(), kept()));
        }
        // Typing, caret movement and selection outside the equation commands leave the equation.
        if self.math.is_some()
            && (id.starts_with("text.") || id.starts_with("caret.") || id.starts_with("select.") || id.starts_with("edit.paste"))
            && let Some(m) = self.math.take()
        {
            let off = m.at.off + wordcraft_doc::para::OBJ.len_utf8();
            self.sel = Selection::caret(Pos { off, ..m.at });
        }
        // The undo stacks are snapshotted whole, not as a length: commands run nested commands,
        // whose checkpoints, undos and redos (and evictions at the limit) must be put back too.
        let before_doc = if spec.mutates {
            Some((
                self.doc.clone(),
                self.sel.clone(),
                self.history.clone(),
                self.redo.clone(),
                self.typing_open,
                self.undo_evicted,
                self.column.clone(),
            ))
        } else {
            None
        };
        let typing = spec.id == "text.insert" || spec.id == "equation.type";
        if spec.mutates && !typing {
            self.typing_open = false;
        }
        if spec.mutates {
            let label = if typing { "Typing" } else { spec.label };
            if !join || self.history.is_empty() {
                self.checkpoint(label);
            }
        } else if !spec.id.starts_with("caret.") && !spec.id.starts_with("view.") {
            // Non-mutating commands other than caret movement keep the typing group.
        } else if spec.id.starts_with("caret.") {
            self.typing_open = false;
        }
        // Only the paste right after a tracked cut makes it a move.
        if (spec.mutates || matches!(id, "edit.undo" | "edit.undoTo" | "edit.redo")) && !matches!(id, "edit.cut" | "edit.paste" | "edit.pasteSpecial")
        {
            self.pending_move = None;
        }
        let sel_before = self.sel.clone();
        let run = spec.run;
        let mutates = spec.mutates;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::cmd::column::dispatch(self, id, mutates, run, params)));
        let mut result = match result {
            Ok(r) => r,
            Err(p) => {
                let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_default();
                Err(CmdError::Failed(format!("{id} panicked: {msg}")))
            }
        };
        // Restrict Editing: outside the exceptions nothing may change.
        if result.is_ok()
            && guard
            && let Some((before, ..)) = &before_doc
            && !crate::cmd::protect::outside_unchanged(before, &self.doc, &self.author)
        {
            result = Err(CmdError::Disabled(format!("{id}: only the editable regions of this protected document can change")));
        }
        match &result {
            Ok(_) => {
                if spec.mutates {
                    // Formatting changed while tracking becomes a formatting revision.
                    if let Some((before, ..)) = &before_doc {
                        crate::cmd::fmt_revisions::record(self, id, before);
                    }
                    self.touch();
                    self.doc.ensure_nonempty();
                    self.doc.prune_text_boxes();
                    // Glued connectors follow the shapes they join.
                    if crate::cmd::connectors::reroutes_after(id) && crate::cmd::connectors::reroute(self) {
                        self.touch();
                    }
                }
                self.clamp_selection();
                // A selection command that moves the insertion point ends the typing group, like
                // caret movement: typing, Select, typing are two undo steps (#419).
                if !spec.mutates && self.sel != sel_before {
                    self.typing_open = false;
                }
                if spec.mutates && !matches!(id, "edit.undo" | "edit.redo") {
                    self.note_edit();
                }
                // Outline view: collapsed headings follow edits; the caret stays where it shows.
                crate::cmd::outline::after_command(self, &sel_before);
                // Extra selected objects last until something else is edited or selected.
                if !matches!(id, "select.addObject" | "arrange.ungroup") && (spec.mutates || self.sel != sel_before) {
                    self.also_selected.clear();
                }
            }
            Err(e) => {
                if let Some((d, s, h, r, t, ev, c)) = before_doc {
                    self.doc = d;
                    self.sel = s;
                    self.history = h;
                    self.redo = r;
                    self.typing_open = t;
                    // The steps the command pushed out are back, so they no longer count as gone.
                    self.undo_evicted = ev;
                    self.column = c;
                }
                self.status = e.to_string();
            }
        }
        result
    }

    /// Reserve an app-controlled revision author distinct from the owner and existing
    /// document authors. `unique_suffix` comes from the host, never from the client label.
    /// Keep the returned name for this attachment; its subsequent revisions are its own.
    pub fn agent_author(&self, label: &str, unique_suffix: &str) -> Result<String, String> {
        let label = label.trim();
        if label.is_empty() || label.len() > 128 || !label.chars().all(|c| c.is_ascii_alphanumeric() || " _.-@".contains(c)) {
            return Err("agent label must contain 1-128 ASCII letters, numbers, spaces, _, ., - or @".into());
        }
        if unique_suffix.is_empty()
            || unique_suffix.len() > 128
            || !unique_suffix.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("agent author suffix must be a bounded host-generated identifier".into());
        }
        let in_use = |candidate: &str| {
            candidate.trim() == self.author.trim()
                || self.doc.revisions.iter().any(|revision| revision.author.trim() == candidate.trim())
                || self.doc.comments.values().any(|comment| comment.author.trim() == candidate.trim())
        };
        for serial in 0..32 {
            let suffix = if serial == 0 { unique_suffix.to_string() } else { format!("{unique_suffix}-{serial}") };
            let candidate = format!("{label} (WordCraft agent {suffix})");
            if !in_use(&candidate) {
                return Ok(candidate);
            }
        }
        Err("cannot reserve a distinct agent author; disable access and request a new invitation".into())
    }

    /// Execute a permitted live-agent command with its own author, selection and typing
    /// properties. The host supplies the command policy; this transaction isolates owner UI
    /// state, forces tracked edits, and keeps agent undo separate from owner typing.
    #[allow(clippy::type_complexity)]
    pub fn run_as_agent(
        &mut self,
        author: &str,
        selection: Selection,
        pending: Option<CharProps>,
        id: &str,
        params: &Value,
    ) -> Result<(Value, Selection, Option<CharProps>), CmdError> {
        if author.trim().is_empty() || author.len() > 512 || author.chars().any(char::is_control) || author.trim() == self.author.trim() {
            return Err(CmdError::Failed("agent revision author must be distinct from the document owner's author".into()));
        }
        let mutates = self.registry.get(id).ok_or_else(|| CmdError::Unknown(id.to_string()))?.mutates;
        crate::agent_selection::validate_selection(&self.doc, &selection).map_err(CmdError::Failed)?;
        let edited_start = selection.ordered().0;
        let before = self.doc.clone();
        let history = self.history.clone();
        let redo = self.redo.clone();
        let evicted = self.undo_evicted;
        let rev = self.rev;
        let dirty = self.dirty;
        let typing_open = self.typing_open;
        let join_next = self.join_next;
        let mut owner = AgentOwnerState::take(self);
        self.sel = selection;
        self.pending = pending;
        self.author = author.to_string();
        self.initials.clear();
        self.doc.settings.track_changes = true;
        self.prefs.markup.track_formatting = true;
        self.prefs.autocorrect.enabled = false;
        self.prefs.overtype = false;
        self.prefs.typing_replaces_selection = true;
        self.typing_open = false;
        self.join_next = false;
        let mut result = self.run(id, params);
        let agent_selection = self.sel.clone();
        let agent_pending = self.pending.clone();
        if result.is_ok()
            && let Err(error) = crate::agent_selection::validate_selection(&self.doc, &agent_selection)
        {
            result = Err(CmdError::Failed(format!("agent_postcondition_failed: {error}; operation was rolled back")));
        }
        self.doc.settings.track_changes = before.settings.track_changes;
        let mut owner_relocated = None;
        let owner_selection = match &result {
            Ok(_) => match crate::shift_agent_selection(&before, &self.doc, &owner.sel).or_else(|error| {
                // These audited commands change formatting, never paragraph paths or text.
                // Generic mapping stays identity-first; only its failure can reach this
                // bounded certificate, so duplicated paragraphs do not block plain bold.
                if matches!(
                    id,
                    "format.bold"
                        | "format.italic"
                        | "format.underline"
                        | "format.strikethrough"
                        | "format.font"
                        | "format.size"
                        | "format.color"
                        | "format.highlight"
                        | "format.subscript"
                        | "format.superscript"
                        | "para.style"
                        | "para.align"
                        | "para.alignLeft"
                        | "para.alignCenter"
                        | "para.alignRight"
                        | "para.justify"
                        | "para.spacing"
                        | "para.indents"
                ) {
                    crate::agent_selection::preserve_formatting_selection(&before, &self.doc, &owner.sel)
                } else {
                    Err(error)
                }
            }) {
                Ok(selection) => selection,
                Err(error) => match crate::agent_selection::relocate_owner_selection(&self.doc, &edited_start, &agent_selection) {
                    Ok(selection) => {
                        owner_relocated = Some(error);
                        selection
                    }
                    Err(recovery) => {
                        result = Err(CmdError::Failed(format!("agent_postcondition_failed: {recovery}; operation was rolled back")));
                        owner.sel.clone()
                    }
                },
            },
            Err(_) => owner.sel.clone(),
        };
        if result.is_err() {
            self.doc = before;
            self.history = history;
            self.redo = redo;
            self.undo_evicted = evicted;
            self.rev = rev;
            self.dirty = dirty;
            self.typing_open = typing_open;
            self.join_next = join_next;
            self.relayout();
        } else {
            // The snapshot belongs to the owner's Undo button: restoring it must also
            // restore the owner's caret, pending formatting and Track Changes preference.
            let gone = usize::try_from(self.undo_evicted.saturating_sub(evicted)).unwrap_or(usize::MAX);
            let first_new = history.len().saturating_sub(gone);
            for undo in self.history.iter_mut().skip(first_new) {
                let undo = Arc::make_mut(undo);
                undo.doc.settings.track_changes = before.settings.track_changes;
                undo.sel = owner.sel.clone();
                undo.pending = owner.pending.clone();
                undo.column = owner.column.clone();
                undo.label = format!("Agent {author}: {}", undo.label);
            }
            if mutates {
                owner.after_edit(&before, &self.doc);
            }
            if owner_relocated.is_some() {
                // Undo snapshots above retain the original owner's complete selection state.
                // At the recovered caret, old typing props/geometry must not drive a new edit.
                owner.pending = None;
                owner.goal_x = None;
                owner.page_hint = 0;
                owner.column = None;
                owner.column_mode = false;
                owner.widened = None;
                owner.also_selected.clear();
                owner.pending_move = None;
                owner.math = None;
                owner.math_latex = false;
                owner.math_normal_text = false;
            }
            // A mutating agent operation ends the owner's typing group on both sides.
            // Pure agent reads and selection changes must not split the owner's typing.
            self.typing_open = if mutates || owner_relocated.is_some() { false } else { typing_open };
            self.join_next = if mutates || owner_relocated.is_some() { false } else { join_next };
        }
        let warning = owner_relocated.map(|reason| {
            serde_json::json!({
                "code":"owner_selection_relocated",
                "reason":reason,
                "ownerSelection":owner_selection,
                "nextStep":"The edit succeeded. Inspect the document; do not retry this operation."
            })
        });
        owner.restore(self, owner_selection);
        result.map(|mut value| {
            if let Some(warning) = warning {
                match value.as_object_mut() {
                    Some(fields) => {
                        fields.insert("agentWarnings".into(), serde_json::json!([warning]));
                    }
                    None => value = serde_json::json!({"value":value,"agentWarnings":[warning]}),
                }
            }
            (value, agent_selection, agent_pending)
        })
    }

    /// The paragraph path the caret is in.
    pub fn caret_path(&self) -> &Path {
        &self.sel.focus.path
    }
    pub fn story(&self) -> StoryRef {
        self.sel.focus.story
    }

    /// Formatting typing would use at the caret.
    pub fn typing_props(&self) -> CharProps {
        if let Some(p) = &self.pending {
            return p.clone();
        }
        let f = &self.sel.focus;
        let Some(p) = self.doc.para_at(f) else { return CharProps::default() };
        let props = p.props_at(f.off);
        // As in Word, text typed at either end of a hyperlink isn't part of it; only typing
        // inside the link extends it.
        let inside_link = props.link.is_some() && f.off > 0 && f.off < p.len() && p.props_of_char(f.off).link == props.link;
        if props.link.is_some() && !inside_link { props.unlinked() } else { props.clone() }
    }

    /// Words in the document, as the status bar shows them (see [`Prefs::count_notes`]).
    pub fn word_count(&self) -> usize {
        if self.prefs.count_notes { self.doc.word_count_including_notes() } else { self.doc.word_count() }
    }

    /// Selected plain text.
    pub fn selected_text(&self) -> String {
        // A column block or whole cells that aren't one run of the text: each piece's own text,
        // a line each, so the status bar and Word Count count those cells only.
        if let Some(segs) = self.column_segments() {
            return segs.iter().map(|(a, b)| self.doc.copy_range(a.min(b), a.max(b)).plain_text()).collect::<Vec<_>>().join("\n");
        }
        let (a, b) = self.sel.ordered();
        self.doc.copy_range(&a, &b).plain_text()
    }
}

/// A new blank document on `region`'s paper (see [`Session::blank_document`]).
pub fn blank_document(region: &wordcraft_geom::Region) -> Document {
    let mut d = Document::new();
    let (_, w, h) = region.paper();
    d.last_section.page_w = w;
    d.last_section.page_h = h;
    d
}

/// Transient state owned by the person, never by a live agent command.
struct AgentOwnerState {
    sel: Selection,
    pending: Option<CharProps>,
    author: String,
    initials: String,
    view: ViewState,
    prefs: Prefs,
    find: FindState,
    goal_x: Option<f32>,
    page_hint: usize,
    painter: Option<(CharProps, wordcraft_doc::ParaProps, bool)>,
    math: Option<MathEdit>,
    math_latex: bool,
    math_normal_text: bool,
    column: Option<ColumnBlock>,
    column_mode: bool,
    widened: Option<Widened>,
    also_selected: Vec<Pos>,
    pending_move: Option<crate::cmd::moves::PendingMove>,
    last_command: Option<(String, Value)>,
    recording: Option<(String, Vec<(String, Value)>)>,
    edit_marks: Vec<Pos>,
    go_back_step: usize,
    ui_requests: Vec<Value>,
    status: String,
}

impl AgentOwnerState {
    fn take(s: &mut Session) -> Self {
        // Exhaustive like edit_snapshot: each new Session field requires an explicit
        // decision about owner isolation here. Document/history/cache are handled by the
        // transaction; host wiring and unrelated persistent data remain in the session.
        let Session {
            doc: _,
            sel,
            view,
            pending,
            path: _,
            user_dict_path: _,
            dirty: _,
            clipboard: _,
            pending_move,
            clipboard_text: _,
            clip_history: _,
            find,
            goal_x,
            page_hint,
            registry: _,
            author,
            initials,
            painter,
            status,
            history: _,
            undo_evicted: _,
            redo: _,
            typing_open: _,
            join_next: _,
            rev: _,
            cache: _,
            layout: _,
            originals: _,
            last_command,
            recording,
            macros: _,
            versions: _,
            building_blocks: _,
            autosave: _,
            bib_style: _,
            merge: _,
            ui_requests,
            document_id: _,
            read_aloud: _,
            prefs,
            system_region: _,
            keymap: _,
            ribbon: _,
            editing_lang: _,
            math,
            math_latex,
            math_normal_text,
            password: _,
            also_selected,
            column,
            column_mode,
            widened,
            edit_marks,
            go_back_step,
        } = s;
        Self {
            sel: std::mem::take(sel),
            pending: pending.take(),
            author: author.clone(),
            initials: initials.clone(),
            view: view.clone(),
            prefs: prefs.clone(),
            find: std::mem::take(find),
            goal_x: goal_x.take(),
            page_hint: std::mem::replace(page_hint, 0),
            painter: painter.take(),
            math: math.take(),
            math_latex: std::mem::replace(math_latex, false),
            math_normal_text: std::mem::replace(math_normal_text, false),
            column: column.take(),
            column_mode: std::mem::replace(column_mode, false),
            widened: widened.take(),
            also_selected: std::mem::take(also_selected),
            pending_move: pending_move.take(),
            last_command: last_command.take(),
            recording: recording.take(),
            edit_marks: std::mem::take(edit_marks),
            go_back_step: std::mem::replace(go_back_step, 0),
            ui_requests: std::mem::take(ui_requests),
            status: std::mem::take(status),
        }
    }
    fn after_edit(&mut self, before: &Document, after: &Document) {
        let same_blocks =
            |a: &wordcraft_doc::Blocks, b: &wordcraft_doc::Blocks| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y));
        let changed = !same_blocks(&before.body, &after.body)
            || before.parts.len() != after.parts.len()
            || before.parts.iter().any(|(id, part)| after.parts.get(id).is_none_or(|p| !same_blocks(&part.blocks, &p.blocks)));
        if !changed {
            return;
        }
        // A block/cell selection can refer to obsolete rows or line geometry after an edit.
        // Keep the mapped ordinary selection, and leave these modes before the person types.
        self.column = None;
        self.column_mode = false;
        self.widened = None;
        self.also_selected.clear();
        self.pending_move = None;
        self.edit_marks = self
            .edit_marks
            .iter()
            .filter_map(|p| crate::shift_agent_selection(before, after, &Selection::caret(p.clone())).ok().map(|s| s.focus))
            .collect();
        self.go_back_step %= self.edit_marks.len().max(1);
        // Keep the person's search and view. Results are regenerated by Find Next;
        // stale match offsets must not drive an edit, and remapping all matches is unbounded.
        self.find.results.clear();
        self.find.current = 0;
        self.find.highlights = None;
        if let Some(math) = &mut self.math {
            let end = Pos { off: math.at.off.saturating_add(wordcraft_doc::para::OBJ.len_utf8()), ..math.at.clone() };
            let mapped = crate::shift_agent_selection(before, after, &Selection { anchor: math.at.clone(), focus: end }).ok();
            let next = mapped.and_then(|s| {
                after.para_at(&s.anchor).and_then(|p| match p.object_at(s.anchor.off) {
                    Some(wordcraft_doc::InlineObject::Equation { math: equation, .. }) => {
                        Some((s.anchor.clone(), wordcraft_doc::math_edit::clamp(&equation.nodes, &math.pos)))
                    }
                    _ => None,
                })
            });
            match next {
                Some((at, pos)) => {
                    math.at = at;
                    math.pos = pos;
                }
                None => self.math = None,
            }
        }
    }
    fn restore(self, s: &mut Session, selection: Selection) {
        // Exhaustive like edit_snapshot: each new Session field requires an explicit
        // decision about owner isolation here. Document/history/cache are handled by the
        // transaction; host wiring and unrelated persistent data remain in the session.
        let Session {
            doc: _,
            sel,
            view,
            pending,
            path: _,
            user_dict_path: _,
            dirty: _,
            clipboard: _,
            pending_move,
            clipboard_text: _,
            clip_history: _,
            find,
            goal_x,
            page_hint,
            registry: _,
            author,
            initials,
            painter,
            status,
            history: _,
            undo_evicted: _,
            redo: _,
            typing_open: _,
            join_next: _,
            rev: _,
            cache: _,
            layout: _,
            originals: _,
            last_command,
            recording,
            macros: _,
            versions: _,
            building_blocks: _,
            autosave: _,
            bib_style: _,
            merge: _,
            ui_requests,
            document_id: _,
            read_aloud: _,
            prefs,
            system_region: _,
            keymap: _,
            ribbon: _,
            editing_lang: _,
            math,
            math_latex,
            math_normal_text,
            password: _,
            also_selected,
            column,
            column_mode,
            widened,
            edit_marks,
            go_back_step,
        } = s;
        *sel = selection;
        *pending = self.pending;
        *author = self.author;
        *initials = self.initials;
        *view = self.view;
        *prefs = self.prefs;
        *find = self.find;
        *goal_x = self.goal_x;
        *page_hint = self.page_hint;
        *painter = self.painter;
        *math = self.math;
        *math_latex = self.math_latex;
        *math_normal_text = self.math_normal_text;
        *column = self.column;
        *column_mode = self.column_mode;
        *widened = self.widened;
        *also_selected = self.also_selected;
        *pending_move = self.pending_move;
        *last_command = self.last_command;
        *recording = self.recording;
        *edit_marks = self.edit_marks;
        *go_back_step = self.go_back_step;
        *ui_requests = self.ui_requests;
        *status = self.status;
    }
}

#[cfg(test)]
#[path = "tests_agents.rs"]
mod agent_tests;

#[cfg(test)]
mod live_selection_review_tests {
    use super::*;
    use serde_json::json;
    use wordcraft_doc::RevisionKind;

    #[test]
    fn whole_document_edits_with_blank_duplicates_succeed_and_warn_without_losing_owner_undo() {
        let original = "Title\n\n\nPara one\n\nPara two";
        for command in ["text.insert", "text.delete"] {
            let mut session = Session::new(Document::from_text(original));
            session.author = "Owner".into();
            session.initials = "OW".into();
            session.view.zoom = 1.7;
            session.prefs.overtype = true;
            session.pending = Some(CharProps { italic: Some(true), ..Default::default() });
            session.sel = Selection { anchor: Pos::body(2, 0), focus: Pos::body(5, 3) };
            let owner = session.sel.clone();
            let pending = session.pending.clone();
            let prefs = session.prefs.clone();
            let before = serde_json::to_value(&session.doc).unwrap();
            let selected = session.run_as_agent("Review agent", Selection::caret(Pos::body(0, 0)), None, "select.all", &json!({})).unwrap().1;
            let (result, _, _) =
                session.run_as_agent("Review agent", selected, None, command, &json!({"text":"😀 replacement", "raw":true})).unwrap();
            assert_eq!(result["agentWarnings"][0]["code"], "owner_selection_relocated");
            assert!(result["agentWarnings"][0]["nextStep"].as_str().unwrap().contains("do not retry"));
            assert_eq!(result["agentWarnings"][0]["ownerSelection"], json!(session.sel));
            assert!(session.sel.is_collapsed());
            crate::agent_selection::validate_selection(&session.doc, &session.sel).unwrap();
            assert_eq!(session.author, "Owner");
            assert_eq!(session.initials, "OW");
            assert_eq!(session.view.zoom, 1.7);
            assert_eq!(session.prefs, prefs);
            assert!(!session.doc.settings.track_changes);
            assert!(session.doc.revisions.iter().any(|r| r.kind == RevisionKind::Delete && r.author == "Review agent"));
            if command == "text.insert" {
                assert!(session.doc.revisions.iter().any(|r| r.kind == RevisionKind::Insert && r.author == "Review agent"));
                assert!(session.doc.plain_text(StoryRef::Body).contains("😀 replacement"));
            }
            assert!(session.pending.is_none());
            assert_eq!(session.undo_depth(), 1);
            assert!(session.undo());
            assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
            assert_eq!(session.sel, owner);
            assert_eq!(session.pending, pending);
        }
    }

    #[test]
    fn local_agent_insert_and_formatting_work_in_more_than_65536_paragraphs() {
        let mut session = Session::new(Document::from_text(&"x\n".repeat(65_538)));
        session.sel = Selection::caret(Pos::body(65_536, 1));
        let owner = session.sel.clone();
        let (insert, _, _) =
            session.run_as_agent("Review agent", Selection::caret(Pos::body(0, 0)), None, "text.insert", &json!({"text":"new ","raw":true})).unwrap();
        assert!(insert.get("agentWarnings").is_none());
        assert_eq!(session.sel, owner);
        session.sel = Selection::caret(Pos::body(0, 5));
        let formatted_owner = session.sel.clone();
        let range = Selection { anchor: Pos::body(0, 0), focus: Pos::body(0, 4) };
        let (format, _, _) = session.run_as_agent("Review agent", range, None, "format.bold", &json!({"value":true})).unwrap();
        assert!(format.get("agentWarnings").is_none());
        assert_eq!(session.sel, formatted_owner);
        assert_eq!(session.doc.para_at(&Pos::body(0, 0)).unwrap().props_at(0).bold, Some(true));
        assert!(session.undo());
        assert_eq!(session.sel, formatted_owner);
        assert!(session.undo());
        assert_eq!(session.sel, owner);
        assert_eq!(session.doc.para_at(&Pos::body(0, 0)).unwrap().text, "x");
    }

    #[test]
    fn duplicate_unicode_paragraphs_in_table_cell_can_be_replaced_with_safe_owner_recovery() {
        let mut session = Session::new(Document::new());
        session.run("insert.table", &json!({"rows":1,"cols":1})).unwrap();
        session.run("text.insert", &json!({"text":"Текст\r\r\r尾段","raw":true})).unwrap();
        let table = session.sel.focus.path.0.get(..3).unwrap().to_vec();
        let position = |paragraph: u32, off: usize| {
            let mut path = table.clone();
            path.push(paragraph);
            Pos { story: StoryRef::Body, path: Path(path), off }
        };
        session.sel = Selection::caret(position(2, 0));
        let owner = session.sel.clone();
        let before = serde_json::to_value(&session.doc).unwrap();
        let selection = Selection { anchor: position(0, 0), focus: position(3, "尾段".len()) };
        assert_eq!(session.doc.para_at(&selection.focus).unwrap().text, "尾段");
        let (result, _, _) = session.run_as_agent("Review agent", selection, None, "text.insert", &json!({"text":"é😀", "raw":true})).unwrap();
        assert_eq!(result["agentWarnings"][0]["code"], "owner_selection_relocated");
        assert!(session.sel.is_collapsed());
        assert_eq!(session.sel.focus.path.0.get(..3), Some(table.as_slice()));
        crate::agent_selection::validate_selection(&session.doc, &session.sel).unwrap();
        assert!(session.undo());
        assert_eq!(serde_json::to_value(&session.doc).unwrap(), before);
        assert_eq!(session.sel, owner);
    }
}
