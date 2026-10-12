//! Danish false-flag check: reads clean Danish text on stdin, prints the flag rate and the
//! most frequent flagged lowercase words (capitalised ones are mostly names) and split-compound
//! flags.
//!
//! ```text
//! cargo run --release -p wordcraft-proof --example da_check < clean.txt
//! ```

use std::collections::HashMap;
use std::io::Read;
use wordcraft_proof::{IssueKind, ProofLang, check_text, words};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let (mut total, mut flagged, mut split) = (0usize, HashMap::<String, usize>::new(), HashMap::<String, usize>::new());
    for line in text.lines() {
        total += words(line).len();
        for i in check_text(line, &[(0..line.len(), ProofLang::Da)]) {
            let map = if i.kind == IssueKind::Grammar { &mut split } else { &mut flagged };
            *map.entry(line.get(i.start..i.end).unwrap_or("").to_string()).or_default() += 1;
        }
    }
    let n: usize = flagged.values().sum();
    let lower: usize = flagged.iter().filter(|(w, _)| w.chars().next().is_some_and(char::is_lowercase)).map(|(_, c)| c).sum();
    let per = |k: usize| k as f64 * 1000.0 / total.max(1) as f64;
    println!("{total} words, {n} flagged = {:.1} per 1,000 ({:.1} lowercase, {:.1} capitalised)", per(n), per(lower), per(n - lower));
    let mut v: Vec<_> = flagged.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (w, c) in v.iter().filter(|(w, _)| w.chars().next().is_some_and(char::is_lowercase)).take(60) {
        println!("{c:5} {w}");
    }
    println!("split compounds: {}", split.values().sum::<usize>());
    for (w, c) in &split {
        println!("{c:5} {w}");
    }
    Ok(())
}
