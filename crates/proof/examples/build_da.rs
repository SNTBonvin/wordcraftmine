//! Build the Danish spelling lexicon (all sources CC0).
//!
//! ```text
//! curl -LO https://ordregister.dk/files/cor1.5.1.0.tsv     # COR₁ (Retskrivningsordbogen forms)
//! curl -LO https://ordregister.dk/files/corext1.0.tsv      # COR.EXT (Den Danske Ordbog lemmas)
//! # freq.tsv: `word<TAB>count` over the CC0 subsets of Danish Dynaword (ft, nordjyllandnews, nota)
//! # places.csv, given.csv, family.csv: Wikidata labels (header line, one label per line)
//! # pairs.tsv: `w1 w2<TAB>…` lowercase adjacent pairs in the same corpus whose join is a listed form
//! #   of class ≥ 6 (from a first build) and that occur at least max(20, join count / 20) times
//! cargo run --release -p wordcraft-proof --example build_da -- \
//!     cor1.5.1.0.tsv corext1.0.tsv freq.tsv places.csv given.csv family.csv pairs.tsv assets/proofing/da.lex
//! ```
//!
//! COR₁ columns: id, lemma, gloss, tag, form, norm (`N` normed, `K` probably correct, `U` not normed).
//! COR.EXT columns: id, lemma, gloss, class, DDO id, flag, tag, form.
//! Flags: every form is a [`WORD`]; noun, adjective and verb forms are compound [`HEAD`]s;
//! compound-first forms ([`FIRST`]) are the listed *sammensætningsformer* (`hus-`, `arbejds-`), else
//! the noun's singular indefinite, plus verb infinitives, adjective base forms, prefixes, proper
//! nouns and Danish place names. Danish words missing from COR (stressed *én*, *ét*, *dets*,
//! *oftest*, *kong*) and compound-starting adverbs and ordinals are added here. Each form gets the frequency class of its corpus count; compounds seen
//! at least [`MIN_COMPOUND_COUNT`] times and given and family names seen at least
//! [`MIN_NAME_COUNT`] times are listed too. Word pairs from `pairs.tsv` ("over for") are stored with no
//! flags: [`wordcraft_proof::da::split_compound`] leaves them alone. Forms that are only ever a
//! present-tense verb are stored as `~form` (no flags) for
//! [`wordcraft_proof::da::present_for_infinitive`].

use std::collections::{HashMap, HashSet};
use wordcraft_proof::da::is_compound;
use wordcraft_proof::lexicon::{FIRST, HEAD, Lexicon, UNNORMED, WORD, freq_class};

const MIN_COMPOUND_COUNT: u64 = 3;
const MIN_NAME_COUNT: u64 = 3;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, cor1, corext, freq, places, given, family, pairs, out] = args.as_slice() else {
        return Err("usage: build_da <cor1.tsv> <corext.tsv> <freq.tsv> <places.csv> <given.csv> <family.csv> <pairs.tsv> <out.lex>".into());
    };
    // Both COR files as [id, lemma, gloss, tag, form, norm]; COR.EXT forms count as probably correct.
    let cor1 = std::fs::read_to_string(cor1)?;
    let corext = std::fs::read_to_string(corext)?;
    let mut rows: Vec<[&str; 6]> = Vec::new();
    for l in cor1.lines() {
        if let [id, lemma, gloss, tag, form, norm, ..] = l.split('\t').collect::<Vec<_>>()[..] {
            rows.push([id, lemma, gloss, tag, form, norm]);
        }
    }
    for l in corext.lines() {
        if let [id, lemma, gloss, _, _, _, tag, form, ..] = l.split('\t').collect::<Vec<_>>()[..] {
            rows.push([id, lemma, gloss, tag, form, "K"]);
        }
    }
    // Lemma key: the COR id without its form and variant parts.
    let lemma = |id: &str| id.rsplitn(3, '.').last().unwrap_or("").to_string();
    let with_sms: HashSet<String> = rows.iter().filter(|c| c[3].ends_with(".sms")).map(|c| lemma(c[0])).collect();
    let mut flags: HashMap<String, u8> = HashMap::new();
    // Forms whose every reading is a present-tense verb ("bliver", not "læser" or "når") are also
    // stored as "~form" for the present-for-infinitive check.
    let mut present_only: HashMap<&str, bool> = HashMap::new();
    for c in &rows {
        let p = c[3].starts_with("vb.præs");
        *present_only.entry(c[4]).or_insert(p) &= p;
    }
    for (form, _) in present_only.iter().filter(|(f, p)| **p && f.ends_with('r')) {
        flags.insert(format!("~{form}"), 0);
    }
    for c in &rows {
        let (tag, form, norm) = (c[3], c[4], c[5]);
        if form.contains(' ') || form.starts_with('-') || form.is_empty() {
            continue;
        }
        let mut add = |w: &str, f: u8| *flags.entry(w.to_string()).or_insert(0) |= f;
        if let Some(stem) = form.strip_suffix('-') {
            if tag.ends_with(".sms") || tag == "præfiks" {
                add(stem, FIRST);
            }
            continue;
        }
        let class = tag.split('.').next().unwrap_or("");
        let mut f = WORD;
        if norm == "U" {
            f |= UNNORMED;
        }
        if matches!(class, "sb" | "adj" | "vb") {
            f |= HEAD;
        }
        let noun_base = class == "sb" && tag.ends_with(".sg.ubest") && !with_sms.contains(&lemma(c[0]));
        // Proper nouns start compounds with their genitive: "Københavnsområdet".
        // Two-letter first parts only when COR lists them ("el-", "ur-") or they are productive.
        let short_ok = form.chars().count() > 2 || matches!(form, "by" | "ny" | "is");
        if short_ok && (noun_base || tag == "vb.inf.akt" || tag == "adj.sg.ubest.fk" || class == "prop") {
            f |= FIRST;
        }
        add(form, f);
        // The word splitter drops full stops: "bl.a." is checked as "bl" and "a".
        if form.contains('.') {
            for piece in form.split('.').filter(|p| !p.is_empty()) {
                add(piece, WORD);
            }
        }
    }
    for w in ["én", "ét", "Én", "Ét", "dets", "oftest", "kong"] {
        *flags.entry(w.to_string()).or_insert(0) |= WORD;
    }
    // Adverbs and ordinals that start compounds: "hjemmeundervisning", "tredjehøjeste".
    for w in ["hjemme", "netto", "brutto", "anden", "tredje", "fjerde", "femte", "sjette", "syvende", "ottende", "niende", "tiende"] {
        *flags.entry(w.to_string()).or_insert(0) |= FIRST;
    }

    let freq = std::fs::read_to_string(freq)?;
    let count: HashMap<&str, u64> = freq.lines().filter_map(|l| l.split_once('\t')).filter_map(|(w, n)| Some((w, n.parse().ok()?))).collect();
    let name_like = |w: &str| {
        let mut c = w.chars();
        w.chars().count() >= 3 && c.next().is_some_and(char::is_uppercase) && c.all(char::is_lowercase)
    };
    // Danish places: every capitalised word of a Danish label ("Esrum", "Nørre", "Hedehusene").
    for t in std::fs::read_to_string(places)?.lines().skip(1).flat_map(|l| l.split([' ', ',', '(', ')', '"'])) {
        if t.split('-').all(name_like) {
            *flags.entry(t.to_string()).or_insert(0) |= WORD | FIRST;
        }
    }
    // Given and family names that Danish text actually uses.
    for path in [given, family] {
        for t in std::fs::read_to_string(path)?.lines().skip(1).map(|l| l.trim_matches('"')) {
            if name_like(t) && count.get(t).copied().unwrap_or(0) >= MIN_NAME_COUNT {
                *flags.entry(t.to_string()).or_insert(0) |= WORD;
            }
        }
    }
    // Frequent compounds become listed forms, so they get a frequency class.
    let base = Lexicon::from_entries(flags.iter().map(|(w, f)| (w.clone(), *f)).collect());
    for (w, &n) in &count {
        if n >= MIN_COMPOUND_COUNT && w.chars().all(char::is_lowercase) && !flags.contains_key(*w) && is_compound(&base, w) {
            flags.insert(w.to_string(), WORD);
        }
    }

    for pair in std::fs::read_to_string(pairs)?.lines().filter_map(|l| l.split('\t').next()) {
        flags.entry(pair.to_string()).or_insert(0);
    }

    // A lowercase form also counts its sentence-start capitalisations.
    let class = |w: &str| {
        let mut n = count.get(w).copied().unwrap_or(0);
        if w.chars().next().is_some_and(char::is_lowercase) {
            let mut c = w.chars();
            let cap: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
            n += count.get(cap.as_str()).copied().unwrap_or(0);
        }
        freq_class(n)
    };
    let lex = Lexicon::from_entries(
        flags
            .into_iter()
            .map(|(w, f)| {
                let c = class(&w);
                (w, f | c << 4)
            })
            .collect(),
    );
    let bytes = lex.to_bytes();
    std::fs::write(out, &bytes)?;
    println!("{} forms, {} bytes", lex.len(), bytes.len());
    Ok(())
}
