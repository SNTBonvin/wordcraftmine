//! Map independent agent and owner selections across document edits.
//! Adapted from lpgn's WordCraft PR #178 `chat_shift.rs` (MIT OR Apache-2.0),
//! commit 4f375f3c81d4dde11d644b3e40312808bce1db80, with bounded traversal,
//! allocation limits, and explicit failures instead of silently guessing a position.

use wordcraft_doc::{Block, Document, Path, Pos, StoryRef};

use crate::Selection;

const MAX_PARAGRAPHS: usize = 65_536;
const MAX_NODES: usize = 262_144;
const MAX_CHANGED_BYTES: usize = 16 << 20;
const SEP: char = '\u{1}';

/// Preserve the selected text across edits, including paragraph splits and table cells.
/// A caret stays before text inserted exactly there; a selection's start moves after it.
/// Refuse overly large or invalid mappings so callers can roll back the edit.
pub fn shift_agent_selection(before: &Document, after: &Document, sel: &Selection) -> Result<Selection, String> {
    if sel.is_collapsed() {
        let p = shift(before, after, &sel.focus, false)?;
        return Ok(Selection::caret(p));
    }
    let anchor_is_start = sel.ordered().0 == sel.anchor;
    Ok(Selection { anchor: shift(before, after, &sel.anchor, anchor_is_start)?, focus: shift(before, after, &sel.focus, !anchor_is_start)? })
}

/// Preserve exact positions only after a caller has certified a formatting-only command.
/// Paragraph paths and text must be unchanged, and any retained paragraph identity must
/// still occupy its original path. This is deliberately not a generic mapping shortcut:
/// cloned duplicate paragraphs cannot establish identity across arbitrary document edits.
pub(crate) fn preserve_formatting_selection(before: &Document, after: &Document, sel: &Selection) -> Result<Selection, String> {
    valid(before, sel.anchor.clone())?;
    valid(before, sel.focus.clone())?;
    for (index, story) in [sel.anchor.story, sel.focus.story].into_iter().enumerate() {
        if index == 1 && story == sel.anchor.story {
            continue;
        }
        let old = paths(before, story)?;
        let new = paths(after, story)?;
        if old != new {
            return Err("formatting selection mapping: paragraph structure changed".into());
        }
        let mut destinations = std::collections::HashMap::new();
        let mut bytes = old.len().saturating_sub(1);
        for path in &new {
            let para = after.para(story, path).ok_or("formatting selection mapping: paragraph missing")?;
            if destinations.insert(std::ptr::from_ref(para), path).is_some() {
                return Err("formatting selection mapping: paragraph identity is ambiguous".into());
            }
        }
        for path in &old {
            let a = before.para(story, path).ok_or("formatting selection mapping: paragraph missing")?;
            let b = after.para(story, path).ok_or("formatting selection mapping: paragraph missing")?;
            bytes = bytes.checked_add(a.len()).ok_or("formatting selection mapping: length overflow")?;
            if bytes > MAX_CHANGED_BYTES {
                return Err("formatting selection mapping: text exceeds 16 MiB".into());
            }
            if a.text != b.text {
                return Err("formatting selection mapping: paragraph text changed".into());
            }
            if destinations.get(&std::ptr::from_ref(a)).is_some_and(|destination| **destination != *path) {
                return Err("formatting selection mapping: retained paragraph identity moved".into());
            }
        }
    }
    Ok(Selection { anchor: valid(after, sel.anchor.clone())?, focus: valid(after, sel.focus.clone())? })
}

fn paths(doc: &Document, story: StoryRef) -> Result<Vec<Path>, String> {
    let blocks = doc.story(story).ok_or("selection_mapping_invalid: story missing; reread and select an existing target")?;
    region_paths(blocks, 0)
}

/// Allocate only for the changed root-block slice. Unchanged prefixes and suffixes
/// are mapped by their retained Arc identities without flattening the whole story.
fn region_paths(blocks: &[std::sync::Arc<Block>], first_root: usize) -> Result<Vec<Path>, String> {
    fn walk(
        blocks: &[std::sync::Arc<Block>],
        first: usize,
        prefix: &mut Vec<u32>,
        out: &mut Vec<Path>,
        nodes: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 16 {
            return Err("agent selection mapping: table nesting limit exceeded".into());
        }
        for (i, block) in blocks.iter().enumerate() {
            *nodes = nodes.saturating_add(1);
            if *nodes > MAX_NODES {
                return Err("selection_mapping_region_limit: changed region is too complex; use a smaller target".into());
            }
            let index = first.checked_add(i).ok_or("selection_mapping_invalid: block index overflow")?;
            prefix.push(u32::try_from(index).map_err(|_| "selection_mapping_invalid: block index overflow")?);
            match block.as_ref() {
                Block::Para(_) => {
                    if out.len() >= MAX_PARAGRAPHS {
                        return Err("selection_mapping_region_limit: paragraph limit exceeded in changed region; use a smaller target".into());
                    }
                    out.push(Path(prefix.clone()));
                }
                Block::Table(table) => {
                    for (r, row) in table.rows.iter().enumerate() {
                        *nodes = nodes.saturating_add(1);
                        if *nodes > MAX_NODES {
                            return Err("selection_mapping_region_limit: row limit exceeded in changed region; use a smaller target".into());
                        }
                        for (c, cell) in row.cells.iter().enumerate() {
                            *nodes = nodes.saturating_add(1);
                            if *nodes > MAX_NODES {
                                return Err("selection_mapping_region_limit: cell limit exceeded in changed region; use a smaller target".into());
                            }
                            prefix.push(u32::try_from(r).map_err(|_| "agent selection mapping: row index overflow")?);
                            prefix.push(u32::try_from(c).map_err(|_| "agent selection mapping: cell index overflow")?);
                            walk(&cell.blocks, 0, prefix, out, nodes, depth.saturating_add(1))?;
                            prefix.pop();
                            prefix.pop();
                        }
                    }
                }
            }
            prefix.pop();
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(blocks, first_root, &mut Vec::new(), &mut out, &mut 0, 0)?;
    Ok(out)
}

fn valid(doc: &Document, p: Pos) -> Result<Pos, String> {
    if p.path.0.len() > 49 {
        return Err("selection_mapping_region_limit: table nesting limit exceeded; select a shallower target".into());
    }
    let para = doc.para_at(&p).ok_or("agent selection mapping: paragraph missing")?;
    if p.off > para.len() || !para.text.is_char_boundary(p.off) {
        return Err("agent selection mapping: invalid character boundary".into());
    }
    Ok(p)
}

pub(crate) fn validate_selection(doc: &Document, sel: &Selection) -> Result<Selection, String> {
    Ok(Selection { anchor: valid(doc, sel.anchor.clone())?, focus: valid(doc, sel.focus.clone())? })
}

/// Owner UI selection recovery is advisory; never use this to retarget an agent.
/// Prefer the start of the requested edit, clamped to a UTF-8 boundary. When its
/// paragraph was removed, use the verified post-command agent caret instead.
pub(crate) fn relocate_owner_selection(after: &Document, edited_start: &Pos, post_command: &Selection) -> Result<Selection, String> {
    validate_selection(after, post_command)?;
    if let Some(para) = after.para_at(edited_start) {
        let mut off = edited_start.off.min(para.len());
        while off > 0 && !para.text.is_char_boundary(off) {
            off = off.saturating_sub(1);
        }
        let next = Pos { off, ..edited_start.clone() };
        if let Ok(next) = valid(after, next) {
            return Ok(Selection::caret(next));
        }
    }
    Ok(Selection::caret(post_command.focus.clone()))
}

fn joined(doc: &Document, story: StoryRef, region: &[Path]) -> Result<String, String> {
    let mut bytes = region.len().saturating_sub(1);
    for path in region {
        let para = doc.para(story, path).ok_or("agent selection mapping: paragraph missing")?;
        if para.text.contains(SEP) {
            return Err("agent selection mapping: paragraph text contains the mapping separator".into());
        }
        bytes = bytes.checked_add(para.len()).ok_or("agent selection mapping: length overflow")?;
        if bytes > MAX_CHANGED_BYTES {
            return Err("selection_mapping_region_limit: changed region exceeds 16 MiB; use a smaller target".into());
        }
    }
    let mut text = String::new();
    text.try_reserve_exact(bytes).map_err(|e| format!("agent selection mapping: {e}"))?;
    for (i, path) in region.iter().enumerate() {
        if i != 0 {
            text.push(SEP);
        }
        text.push_str(&doc.para(story, path).ok_or("agent selection mapping: paragraph missing")?.text);
    }
    Ok(text)
}

fn shift(before: &Document, after: &Document, p: &Pos, start: bool) -> Result<Pos, String> {
    valid(before, p.clone())?;
    let sb = before.story(p.story).ok_or("agent selection mapping: story missing")?;
    let sa = after.story(p.story).ok_or("agent selection mapping: story removed")?;
    if sb.len() == sa.len() && sb.iter().zip(sa.iter()).all(|(x, y)| std::sync::Arc::ptr_eq(x, y)) {
        return valid(after, p.clone());
    }
    let root = usize::try_from(*p.path.0.first().ok_or("selection_mapping_invalid: paragraph path is empty")?)
        .map_err(|_| "selection_mapping_invalid: root index overflow")?;
    let mut root_head = 0usize;
    let shared = sb.len().min(sa.len());
    let same_root = |old: usize, new: usize| sb.get(old).zip(sa.get(new)).is_some_and(|(a, b)| std::sync::Arc::ptr_eq(a, b));
    while root_head < shared && same_root(root_head, root_head) {
        root_head = root_head.saturating_add(1);
    }
    if root < root_head {
        return valid(after, p.clone());
    }
    let mut root_tail = 0usize;
    while root_tail < shared.saturating_sub(root_head)
        && same_root(sb.len().saturating_sub(root_tail.saturating_add(1)), sa.len().saturating_sub(root_tail.saturating_add(1)))
    {
        root_tail = root_tail.saturating_add(1);
    }
    let old_end = sb.len().saturating_sub(root_tail);
    let new_end = sa.len().saturating_sub(root_tail);
    if root >= old_end {
        let new_root = root.checked_add(sa.len()).and_then(|n| n.checked_sub(sb.len())).ok_or("selection_mapping_invalid: root index overflow")?;
        let mut mapped = p.clone();
        let first = mapped.path.0.first_mut().ok_or("selection_mapping_invalid: paragraph path is empty")?;
        *first = u32::try_from(new_root).map_err(|_| "selection_mapping_invalid: root index overflow")?;
        return valid(after, mapped);
    }
    let old_region = sb.get(root_head..old_end).ok_or("selection_mapping_invalid: old changed region is invalid")?;
    let new_region = sa.get(root_head..new_end).ok_or("selection_mapping_invalid: new changed region is invalid")?;
    let (bp, ap) = (region_paths(old_region, root_head)?, region_paths(new_region, root_head)?);
    let i = bp.iter().position(|q| *q == p.path).ok_or("agent selection mapping: position not found")?;
    let pb = |k: usize| bp.get(k).and_then(|q| before.para(p.story, q));
    let pa = |k: usize| ap.get(k).and_then(|q| after.para(p.story, q));
    let original = before.para_at(p).ok_or("agent selection mapping: paragraph missing")?;
    // Paragraphs live in Arc<Block>s, including paragraphs inside copied table containers.
    // An unchanged paragraph's identity survives inserting/deleting equal-content neighbors.
    // Never let textual equality move the caret into the wrong identical/blank paragraph.
    let mut identity = ap.iter().filter(|path| after.para(p.story, path).is_some_and(|para| std::ptr::eq(original, para)));
    if let Some(path) = identity.next() {
        if identity.next().is_some() {
            return Err(
                "selection_mapping_ambiguous: shared paragraph has multiple destinations; reread and explicitly select the intended target".into()
            );
        }
        return valid(after, Pos { story: p.story, path: path.clone(), off: p.off });
    }
    let same_identity = |old: usize, new: usize| match (pb(old), pa(new)) {
        (Some(a), Some(b)) => std::ptr::eq(a, b),
        _ => false,
    };
    let n = bp.len().min(ap.len());
    let mut head = 0usize;
    while head < n && same_identity(head, head) {
        head = head.saturating_add(1);
    }
    let mut tail = 0usize;
    while tail < n.saturating_sub(head) && same_identity(bp.len().saturating_sub(tail + 1), ap.len().saturating_sub(tail + 1)) {
        tail = tail.saturating_add(1);
    }
    let at = |path: &Path, off: usize| Pos { story: p.story, path: path.clone(), off };
    if i < head {
        return valid(after, at(ap.get(i).ok_or("agent selection mapping: prefix missing")?, p.off));
    }
    let (b_end, a_end) = (bp.len().saturating_sub(tail), ap.len().saturating_sub(tail));
    if i >= b_end {
        let j = i.checked_add(ap.len()).and_then(|v| v.checked_sub(bp.len())).ok_or("agent selection mapping: index overflow")?;
        return valid(after, at(ap.get(j).ok_or("agent selection mapping: suffix missing")?, p.off));
    }
    let rb = bp.get(head..b_end).ok_or("agent selection mapping: invalid old range")?;
    let ra = ap.get(head..a_end).ok_or("agent selection mapping: invalid new range")?;
    let (tb, ta) = (joined(before, p.story, rb)?, joined(after, p.story, ra)?);
    // Text differences cannot establish paragraph identity among duplicate paragraphs.
    // Refuse ambiguous changed regions instead of guessing; callers can roll back an edit
    // or ask the agent to select its target again. Allocation stays bounded by paths().
    for (doc, region) in [(before, rb), (after, ra)] {
        let mut texts = std::collections::HashSet::new();
        for path in region {
            let para = doc.para(p.story, path).ok_or("agent selection mapping: paragraph missing")?;
            if !texts.insert(para.text.as_str()) {
                return Err("selection_mapping_ambiguous: duplicate paragraphs make the changed region ambiguous; reread and explicitly select the intended target".into());
            }
        }
    }
    let g = rb
        .iter()
        .take(i.saturating_sub(head))
        .fold(p.off, |g, q| g.saturating_add(before.para(p.story, q).map(|x| x.len()).unwrap_or(0).saturating_add(1)));
    let cp = common_prefix(&tb, &ta);
    let cs = common_suffix(&tb, &ta).min(tb.len().min(ta.len()).saturating_sub(cp));
    let moved = |g: usize| g.saturating_add(ta.len()).saturating_sub(tb.len());
    let changed_end = tb.len().saturating_sub(cs);
    let g2 = if start && g >= changed_end {
        moved(g)
    } else if g <= cp {
        g
    } else if g >= changed_end {
        moved(g)
    } else {
        cp
    };
    if ra.is_empty() {
        let next = match ap.get(head) {
            Some(q) => at(q, 0),
            None => {
                if let Some(q) = ap.last() {
                    at(q, after.para(p.story, q).map(|x| x.len()).unwrap_or(0))
                } else {
                    let neighbor = root_head.min(sa.len().saturating_sub(1));
                    let blocks = sa.get(neighbor..neighbor.saturating_add(1)).ok_or("selection_mapping_invalid: no remaining paragraph")?;
                    let nearby = region_paths(blocks, neighbor)?;
                    let q = if root_head < sa.len() { nearby.first() } else { nearby.last() }
                        .ok_or("selection_mapping_invalid: no remaining paragraph")?;
                    let off = if root_head < sa.len() { 0 } else { after.para(p.story, q).map(|x| x.len()).unwrap_or(0) };
                    at(q, off)
                }
            }
        };
        return valid(after, next);
    }
    let mut left = g2;
    for q in ra {
        let len = after.para(p.story, q).map(|x| x.len()).unwrap_or(0);
        if left <= len {
            return valid(after, at(q, left));
        }
        left = left.saturating_sub(len.saturating_add(1));
    }
    let q = ra.last().ok_or("agent selection mapping: empty changed range")?;
    valid(after, at(q, after.para(p.story, q).map(|x| x.len()).unwrap_or(0)))
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.char_indices().zip(b.chars()).find(|((_, x), y)| x != y).map(|((i, _), _)| i).unwrap_or(a.len().min(b.len()))
}

fn common_suffix(a: &str, b: &str) -> usize {
    let mut n = 0usize;
    for (x, y) in a.chars().rev().zip(b.chars().rev()) {
        if x != y {
            break;
        }
        n = n.saturating_add(x.len_utf8());
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use wordcraft_doc::CharProps;

    #[test]
    fn insertion_at_selection_start_preserves_selected_text_in_both_directions() {
        let before = Document::from_text("Term: 12 months.");
        let mut after = before.clone();
        after.insert_text(&Pos::body(0, 6), "24 months", &CharProps::default()).unwrap();
        for (a, f, want_a, want_f) in [(6, 15, 15, 24), (15, 6, 24, 15)] {
            let sel = Selection { anchor: Pos::body(0, a), focus: Pos::body(0, f) };
            assert_eq!(
                shift_agent_selection(&before, &after, &sel).unwrap(),
                Selection { anchor: Pos::body(0, want_a), focus: Pos::body(0, want_f) }
            );
        }
        let caret = Selection::caret(Pos::body(0, 6));
        assert_eq!(shift_agent_selection(&before, &after, &caret).unwrap(), caret);
    }

    #[test]
    fn deleted_paragraph_and_unicode_replacement_map_safely() {
        let before = Document::from_text("One.\nTwo.\nThree café.");
        let mut after = before.clone();
        after.remove_block(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(2, 2))).unwrap(), Selection::caret(Pos::body(1, 2)));
        assert_eq!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(0, 2))).unwrap(), Selection::caret(Pos::body(0, 0)));
        let before = Document::from_text("éab");
        let after = Document::from_text("😀ab");
        assert_eq!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(0, 2))).unwrap(), Selection::caret(Pos::body(0, 4)));
        assert!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(0, 1))).is_err());
    }

    #[test]
    fn changed_region_over_limit_and_excess_paragraphs_are_refused() {
        let before = Document::from_text(&"x".repeat(MAX_CHANGED_BYTES));
        let mut after = before.clone();
        after.insert_text(&Pos::body(0, 0), "extra", &CharProps::default()).unwrap();
        assert!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(0, 10))).is_err());
        let before = Document::from_text(&"x\n".repeat(MAX_PARAGRAPHS));
        let after = Document::from_text(&"y\n".repeat(MAX_PARAGRAPHS));
        assert!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(0, 0))).is_err());
    }

    #[test]
    fn identical_paragraph_insertions_and_deletions_keep_the_original_identity() {
        let before = Document::from_text("Same\nSame\nSame");
        let mut inserted = before.clone();
        inserted.insert_block(StoryRef::Body, &Path::top(0), before.block(StoryRef::Body, &Path::top(0)).unwrap().clone()).unwrap();
        for row in 0..3 {
            let old = Selection::caret(Pos::body(row, 2));
            assert_eq!(shift_agent_selection(&before, &inserted, &old).unwrap(), Selection::caret(Pos::body(row + 1, 2)));
        }
        let mut removed = before.clone();
        removed.remove_block(StoryRef::Body, &Path::top(0)).unwrap();
        assert_eq!(shift_agent_selection(&before, &removed, &Selection::caret(Pos::body(2, 2))).unwrap(), Selection::caret(Pos::body(1, 2)));
    }

    #[test]
    fn blank_paragraph_identity_survives_identical_neighbor_insertion() {
        let before = Document::from_text("\n\nEnd");
        let mut after = before.clone();
        after.insert_block(StoryRef::Body, &Path::top(0), before.block(StoryRef::Body, &Path::top(0)).unwrap().clone()).unwrap();
        assert_eq!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(1, 0))).unwrap(), Selection::caret(Pos::body(2, 0)));
    }

    #[test]
    fn detached_duplicate_paragraphs_are_refused_rather_than_guessed() {
        let before = Document::from_text("Same\nSame");
        let after = Document::from_text("Same\nSame\nSame");
        assert!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(1, 2))).unwrap_err().contains("ambiguous"));
    }

    #[test]
    fn literal_mapping_separator_cannot_be_mistaken_for_a_paragraph_boundary() {
        let before = Document::from_text("a\u{1}b");
        let mut after = before.clone();
        after.insert_text(&Pos::body(0, 0), "x", &CharProps::default()).unwrap();
        assert!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(0, 2))).unwrap_err().contains("separator"));
    }

    #[test]
    fn certified_formatting_preserves_positions_in_duplicate_paragraphs() {
        let before = Document::from_text("same\nsame\n\n");
        let mut after = before.clone();
        after.format_range(&before.start_of(StoryRef::Body), &before.end_of(StoryRef::Body), &|props| props.bold = Some(true)).unwrap();
        let sel = Selection::caret(Pos::body(1, 2));
        assert!(shift_agent_selection(&before, &after, &sel).is_err());
        assert_eq!(preserve_formatting_selection(&before, &after, &sel).unwrap(), sel);
    }

    #[test]
    fn certified_formatting_rejects_reordering_path_changes_and_text_changes() {
        let before = Document::from_text("same\nsame\n\n");
        let sel = Selection::caret(Pos::body(0, 2));
        let mut reordered = before.clone();
        reordered.body.swap(0, 1);
        assert_eq!(shift_agent_selection(&before, &reordered, &sel).unwrap(), Selection::caret(Pos::body(1, 2)));
        assert!(preserve_formatting_selection(&before, &reordered, &sel).unwrap_err().contains("identity moved"));
        let mut added = before.clone();
        added.insert_block(StoryRef::Body, &Path::top(0), before.block(StoryRef::Body, &Path::top(0)).unwrap().clone()).unwrap();
        assert!(preserve_formatting_selection(&before, &added, &sel).unwrap_err().contains("structure changed"));
        let mut edited = before.clone();
        edited.insert_text(&Pos::body(0, 0), "x", &CharProps::default()).unwrap();
        assert!(preserve_formatting_selection(&before, &edited, &sel).unwrap_err().contains("text changed"));
    }

    #[test]
    fn certified_formatting_does_not_bypass_mapping_limits_or_utf8_checks() {
        let huge = Document::from_text(&"x".repeat(MAX_CHANGED_BYTES + 1));
        assert!(preserve_formatting_selection(&huge, &huge, &Selection::caret(Pos::body(0, 0))).unwrap_err().contains("16 MiB"));
        let too_many = Document::from_text(&"x\n".repeat(MAX_PARAGRAPHS));
        assert!(preserve_formatting_selection(&too_many, &too_many, &Selection::caret(Pos::body(0, 0))).unwrap_err().contains("paragraph limit"));
        let unicode = Document::from_text("é");
        assert!(preserve_formatting_selection(&unicode, &unicode, &Selection::caret(Pos::body(0, 1))).is_err());
    }

    #[test]
    fn large_story_small_changed_region_preserves_retained_positions() {
        let before = Document::from_text(&"x\n".repeat(MAX_PARAGRAPHS + 2));
        let last = Selection::caret(Pos::body(MAX_PARAGRAPHS, 1));
        let mut after = before.clone();
        after.insert_text(&Pos::body(0, 0), "new ", &CharProps::default()).unwrap();
        assert_eq!(shift_agent_selection(&before, &after, &last).unwrap(), last);
        assert_eq!(shift_agent_selection(&before, &after, &Selection::caret(Pos::body(0, 1))).unwrap(), Selection::caret(Pos::body(0, 5)));
        let mut inserted = before.clone();
        inserted.insert_block(StoryRef::Body, &Path::top(0), before.block(StoryRef::Body, &Path::top(0)).unwrap().clone()).unwrap();
        assert_eq!(shift_agent_selection(&before, &inserted, &last).unwrap(), Selection::caret(Pos::body(MAX_PARAGRAPHS + 1, 1)));
    }

    #[test]
    fn owner_recovery_collapses_at_utf8_edit_start_or_verified_post_caret() {
        let after = Document::from_text("😀é");
        let post = Selection::caret(Pos::body(0, 6));
        assert_eq!(relocate_owner_selection(&after, &Pos::body(0, 3), &post).unwrap(), Selection::caret(Pos::body(0, 0)));
        assert_eq!(relocate_owner_selection(&after, &Pos::body(99, 0), &post).unwrap(), post);
        assert!(relocate_owner_selection(&after, &Pos::body(0, 0), &Selection::caret(Pos::body(0, 2))).is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(96))]
        #[test]
        fn mapping_always_returns_existing_utf8_boundaries_or_error(
            paragraphs in proptest::collection::vec(proptest::collection::vec(prop_oneof![Just('a'), Just('é'), Just('😀'), Just('\u{1}')], 0..16), 1..12),
            selected_row in any::<usize>(),
            anchor_char in any::<usize>(),
            focus_char in any::<usize>(),
            edit_row in any::<usize>(),
            operation in 0u8..4,
        ) {
            let strings: Vec<String> = paragraphs.into_iter().map(|chars| chars.into_iter().collect()).collect();
            let before = Document::from_text(&strings.join("\n"));
            let row = selected_row % strings.len();
            let boundaries: Vec<usize> = strings[row].char_indices().map(|(off, _)| off).chain(std::iter::once(strings[row].len())).collect();
            let sel = Selection { anchor: Pos::body(row, boundaries[anchor_char % boundaries.len()]), focus: Pos::body(row, boundaries[focus_char % boundaries.len()]) };
            let mut after = before.clone();
            let edited = edit_row % strings.len();
            match operation {
                0 => { after.insert_text(&Pos::body(edited, 0), "😀", &CharProps::default()).unwrap(); }
                1 => { after.insert_block(StoryRef::Body, &Path::top(edited), before.block(StoryRef::Body, &Path::top(edited)).unwrap().clone()).unwrap(); }
                2 if strings.len() > 1 => { after.remove_block(StoryRef::Body, &Path::top(edited)).unwrap(); }
                _ => { after.split_paragraph(&Pos::body(edited, 0)).unwrap(); }
            }
            if let Ok(mapped) = shift_agent_selection(&before, &after, &sel) {
                for pos in [&mapped.anchor, &mapped.focus] {
                    let paragraph = after.para_at(pos);
                    prop_assert!(paragraph.is_some());
                    let paragraph = paragraph.unwrap();
                    prop_assert!(pos.off <= paragraph.len());
                    prop_assert!(paragraph.text.is_char_boundary(pos.off));
                }
            }
        }
    }
}
