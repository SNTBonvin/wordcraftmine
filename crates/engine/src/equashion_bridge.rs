//! The document's equations and the equaSHion editor's tree, through OMML.
//!
//! WordCraft keeps its own equation model ([`wordcraft_doc::math::Math`]), layout and DOCX
//! save; the equation editor widget and the MathML / LaTeX readers work on
//! [`equashion_math::Math`]. The two meet in OMML: the document side is written and read with
//! the DOCX equation writer and reader ([`wordcraft_docx::omml`]), the editor side with
//! equaSHion's own OMML export and import. So an equation crosses over exactly as it would
//! through a saved file, and nothing here duplicates either converter.

use wordcraft_doc::math::{Arg, LimLoc, MAX_DEPTH, MNode, Math, is_integral};
use wordcraft_docx::omml::{equation_from_omml, equation_to_omml};

pub use equashion_math as eq;

/// A document equation (its `linear` text, `display` flag and tree) as the editor's tree. A
/// display equation becomes an `m:oMathPara` (the editor's `para` flag).
pub fn to_editor(linear: &str, display: bool, math: &Math) -> eq::Math {
    eq::from_omml(&equation_to_omml(linear, display, math))
}

/// The editor's tree as a document equation and its display flag, or `None` when its OMML
/// does not read back as an equation (it always should). The source OMML is not kept: the
/// tree is the equation from here on.
pub fn from_editor(m: &eq::Math) -> Option<(Math, bool)> {
    let (mut math, display) = equation_from_omml(&eq::to_omml(m))?;
    math.omml.clear();
    default_lim_locs(&mut math.nodes, 0);
    Some((math, display))
}

/// equaSHion always writes an n-ary operator's limit location; where it is the usual one
/// (beside an integral, under and over the rest) the document model leaves it unset, as it
/// does for equations made here, so a save writes no `m:limLoc` Word would not.
fn default_lim_locs(a: &mut Arg, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let d = depth + 1;
    for n in a.iter_mut() {
        match n {
            MNode::Run(_) => {}
            MNode::Nary { chr, lim_loc, sub, sup, e, .. } => {
                let usual = if is_integral(*chr) { LimLoc::SubSup } else { LimLoc::UndOvr };
                if *lim_loc == Some(usual) {
                    *lim_loc = None;
                }
                for x in [sub, sup, e] {
                    default_lim_locs(x, d);
                }
            }
            MNode::Frac { num, den, .. } => {
                default_lim_locs(num, d);
                default_lim_locs(den, d);
            }
            MNode::Script { base, sub, sup, .. } => {
                for x in [base, sub, sup] {
                    default_lim_locs(x, d);
                }
            }
            MNode::Rad { deg, e, .. } => {
                default_lim_locs(deg, d);
                default_lim_locs(e, d);
            }
            MNode::Delim { elems, .. } => elems.iter_mut().for_each(|x| default_lim_locs(x, d)),
            MNode::Func { name, e } => {
                default_lim_locs(name, d);
                default_lim_locs(e, d);
            }
            MNode::Lim { e, lim, .. } => {
                default_lim_locs(e, d);
                default_lim_locs(lim, d);
            }
            MNode::Acc { e, .. }
            | MNode::Bar { e, .. }
            | MNode::BorderBox { e, .. }
            | MNode::Boxed { e }
            | MNode::GroupChr { e, .. }
            | MNode::Phant { e, .. } => default_lim_locs(e, d),
            MNode::EqArr { rows } => rows.iter_mut().for_each(|x| default_lim_locs(x, d)),
            MNode::Matrix { rows, .. } => rows.iter_mut().flatten().for_each(|x| default_lim_locs(x, d)),
        }
    }
}

/// The editor's tree as OMML: what equation commands take as their `omml` parameter.
pub fn editor_omml(m: &eq::Math) -> String {
    eq::to_omml(m)
}

/// Presentation MathML (one `<math>` element) as a document equation. `None` when it is not
/// MathML equaSHion reads, or holds nothing.
pub fn from_mathml(xml: &str) -> Option<(Math, bool)> {
    let m = eq::from_mathml(xml).ok()?;
    if m.is_empty() {
        return None;
    }
    from_editor(&m)
}

/// LaTeX (or equaSHion's typed linear form) as a document equation, read by equaSHion's
/// parser (`\frac{a}{b}`, `\sqrt{x}`, `\sum_{i=1}^{n}`, `x^2`). `None` when nothing is left.
pub fn from_latex(src: &str, display: bool) -> Option<(Math, bool)> {
    let src = src.trim();
    if src.is_empty() {
        return None;
    }
    let mut m = eq::from_linear(src);
    if m.is_empty() {
        return None;
    }
    m.para = display;
    from_editor(&m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wordcraft_doc::math::{MNode, merge_runs, parse_linear, to_linear};

    /// The equations the round trips are pinned on: (name, linear format).
    const EQUATIONS: &[(&str, &str)] = &[
        ("fraction", "a/b"),
        ("sqrt", "√(x+1)"),
        ("root with degree", "√(3&x)"),
        ("sum with limits", "∑_(i=1)^n▒i^2"),
        ("matrix", "[■(1&0@0&1)]"),
        ("accent", "x̂+A⃗"),
        ("quadratic formula", "x=(-b±√(b^2-4ac))/2a"),
        ("scripts", "x_i^2+e^(iπ)"),
        ("function", "sin⁡θ"),
        ("integral", "∫_0^1▒〖x dx〗"),
    ];

    fn doc_math(linear: &str) -> Math {
        Math { nodes: parse_linear(linear), ..Default::default() }
    }

    #[test]
    fn document_equations_round_trip_through_the_editor_tree() {
        for (name, lin) in EQUATIONS {
            let m = doc_math(lin);
            for display in [false, true] {
                let e = to_editor(lin, display, &m);
                assert_eq!(e.para, display, "{name}: display flag");
                let (back, d) = from_editor(&e).unwrap_or_else(|| panic!("{name}: no equation back"));
                assert_eq!(d, display, "{name}");
                // Exactly what a DOCX save and reopen gives (that pass fills in the matrix
                // column alignments the writer spells out, nothing else).
                let (saved, _) = equation_from_omml(&equation_to_omml(lin, display, &m)).unwrap_or_default();
                assert_eq!(merge_runs(back.nodes.clone()), merge_runs(saved.nodes), "{name}: {lin} -> {}", to_linear(&back.nodes));
                assert_eq!(to_linear(&back.nodes), to_linear(&m.nodes), "{name}");
                // And the editor's tree survives the document model unchanged.
                let e2 = to_editor(&to_linear(&back.nodes), d, &Math { omml: String::new(), ..back });
                assert_eq!(eq::to_linear(&e2), eq::to_linear(&e), "{name}");
            }
        }
    }

    #[test]
    fn editor_trees_round_trip_through_the_document_model() {
        for src in [r"\frac{a}{b}", r"\sqrt{x}", r"\sum_{i=1}^{n} i", r"\hat{x}", r"x=\frac{-b\pm\sqrt{b^2-4ac}}{2a}"] {
            let e = eq::from_linear(src);
            let (m, _) = from_editor(&e).unwrap_or_else(|| panic!("{src}"));
            let back = to_editor(&to_linear(&m.nodes), false, &m);
            assert_eq!(eq::to_linear(&back), eq::to_linear(&e), "{src}");
        }
    }

    #[test]
    fn structures_land_in_the_document_model() {
        let structure = |lin: &str| from_editor(&to_editor(lin, false, &doc_math(lin))).map(|(m, _)| m.nodes).unwrap_or_default();
        assert!(matches!(structure("a/b").as_slice(), [MNode::Frac { .. }]));
        assert!(matches!(structure("√(x+1)").as_slice(), [MNode::Rad { .. }]));
        assert!(matches!(structure("∑_(i=1)^n▒i").as_slice(), [MNode::Nary { chr: '∑', .. }]));
        assert!(matches!(structure("x̂").as_slice(), [MNode::Acc { .. }]));
        assert!(
            structure("[■(1&0@0&1)]")
                .iter()
                .any(|n| matches!(n, MNode::Delim { elems, .. } if matches!(elems.first().and_then(|e| e.first()), Some(MNode::Matrix { .. }))))
        );
    }

    #[test]
    fn source_omml_is_what_the_editor_gets() {
        // An imported equation keeps Word's XML; the editor reads that, not a rewrite.
        let xml = "<m:oMath><m:sSup><m:e><m:r><m:t>E</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath>";
        let (m, _) = equation_from_omml(xml).unwrap();
        assert!(!m.omml.is_empty());
        assert_eq!(eq::to_linear(&to_editor(&to_linear(&m.nodes), false, &m)), "E^2");
    }

    #[test]
    fn mathml_and_latex_become_document_equations() {
        let (m, display) = from_mathml(r#"<math display="block"><mfrac><mi>a</mi><mi>b</mi></mfrac></math>"#).unwrap();
        assert!(display);
        assert!(matches!(m.nodes.as_slice(), [MNode::Frac { .. }]), "{:?}", m.nodes);
        let (m, display) = from_latex(r"\sqrt{x^2+1}", false).unwrap();
        assert!(!display);
        assert!(matches!(m.nodes.as_slice(), [MNode::Rad { .. }]), "{:?}", m.nodes);
        assert!(from_mathml("<p>not math</p>").is_none());
        assert!(from_latex("   ", false).is_none());
    }
}
