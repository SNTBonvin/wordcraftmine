//! Spanish hyphenation by rule ([`ProofLang::Es`](crate::ProofLang::Es)).
//!
//! Spanish syllable rules are highly regular and syllable-based:
//! - Vowels:
//!   - Strong (open) vowels: `a`, `e`, `o`, `á`, `é`, `ó`.
//!   - Weak (closed) vowels: `i`, `u`, `ü`.
//!   - Accented weak vowels (`í`, `ú`) act as strong vowels, breaking diphthongs into hiatus.
//! - Two strong vowels in contact form a hiatus and split into separate syllables (`ca-os`, `le-er`, `te-a-tro`, `tí-o`, `ba-úl`).
//! - Diphthongs (strong+weak, weak+strong, weak+weak) and triphthongs (weak+strong+weak) stay united in one syllable (`ciel-o`, `bue-no`, `cui-da-do`).
//! - Consonant clusters between vowels:
//!   - Single consonant goes to the next syllable (`ca-sa`).
//!   - Inseparable groups (`ch`, `ll`, `rr`, and obstruent+liquid `pr`, `br`, `tr`, `dr`, `cr`, `gr`, `fr`, `pl`, `bl`, `cl`, `gl`, `fl`, `tl`) go to the next syllable (`o-cho`, `ca-lle`, `pe-rro`, `a-bra-zo`, `a-tle-ta`).
//!   - Two consonants otherwise split between syllables (`car-ta`, `in-ter-no`).
//!   - Three consonants: if the last two form an inseparable group, split after the first (`com-prar`, `es-cri-bir`); otherwise after the second (`cons-tan-te`).
//!   - Four consonants: split two and two (`cons-truir`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VowelKind {
    Strong,
    Weak,
    AccentedWeak,
}

fn vowel_kind(c: char) -> Option<VowelKind> {
    match c.to_ascii_lowercase() {
        'a' | 'e' | 'o' => Some(VowelKind::Strong),
        'i' | 'u' => Some(VowelKind::Weak),
        _ => match c {
            'á' | 'é' | 'ó' | 'Á' | 'É' | 'Ó' => Some(VowelKind::Strong),
            'í' | 'ú' | 'Í' | 'Ú' => Some(VowelKind::AccentedWeak),
            'ü' | 'Ü' => Some(VowelKind::Weak),
            _ => None,
        },
    }
}

pub(crate) fn is_vowel(c: char) -> bool {
    vowel_kind(c).is_some()
}

pub(crate) fn is_spanish_letter(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c, 'á' | 'é' | 'í' | 'ó' | 'ú' | 'ü' | 'ñ' | 'Á' | 'É' | 'Í' | 'Ó' | 'Ú' | 'Ü' | 'Ñ')
}

/// Is the 2-character pair an inseparable consonant onset group?
fn is_inseparable(c1: char, c2: char) -> bool {
    let (c1, c2) = (c1.to_ascii_lowercase(), c2.to_ascii_lowercase());
    // Digraphs: ch, ll, rr
    if (c1 == 'c' && c2 == 'h') || (c1 == 'l' && c2 == 'l') || (c1 == 'r' && c2 == 'r') {
        return true;
    }
    // Obstruent + liquid: b, c, d, f, g, p, t followed by r or l
    if c2 == 'r' && matches!(c1, 'b' | 'c' | 'd' | 'f' | 'g' | 'p' | 't') {
        return true;
    }
    if c2 == 'l' && matches!(c1, 'b' | 'c' | 'f' | 'g' | 'p' | 't') {
        return true;
    }
    false
}

/// Break points of one lowercase Spanish core word (char indices; a hyphen goes before that char).
pub fn points(word: &[char]) -> Vec<usize> {
    if word.len() < 2 || !word.iter().all(|&c| is_spanish_letter(c)) {
        return Vec::new();
    }
    let lower: Vec<char> = word.iter().map(|&c| c.to_lowercase().next().unwrap_or(c)).collect();

    // Group characters into vowel nucleus ranges and consonant ranges.
    let mut vowel_ranges: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < lower.len() {
        if is_vowel(lower[i]) {
            let start = i;
            while i < lower.len() && is_vowel(lower[i]) {
                i += 1;
            }
            vowel_ranges.push((start, i));
        } else {
            i += 1;
        }
    }

    if vowel_ranges.len() <= 1 && vowel_ranges.first().map_or(0, |(s, e)| e - s) <= 1 {
        return Vec::new();
    }

    let mut out = Vec::new();

    // 1. Check hiatus WITHIN each consecutive vowel run (e.g. "caos", "leer", "tío", "baúl", "aéreo")
    for &(vs, ve) in &vowel_ranges {
        if ve - vs >= 2 {
            for j in vs..(ve - 1) {
                let k1 = vowel_kind(lower[j]);
                let k2 = vowel_kind(lower[j + 1]);
                let hiatus = match (k1, k2) {
                    (Some(VowelKind::Strong), Some(VowelKind::Strong)) => true,
                    (Some(VowelKind::AccentedWeak), _) => true,
                    (_, Some(VowelKind::AccentedWeak)) => true,
                    // Identical weak vowels: e.g. "chiita", "duunvirato"
                    (Some(VowelKind::Weak), Some(VowelKind::Weak)) if lower[j] == lower[j + 1] => true,
                    _ => false,
                };
                if hiatus && j + 1 > 0 && j + 1 < lower.len() {
                    out.push(j + 1);
                }
            }
        }
    }

    // 2. Check between consecutive vowel nuclei separated by consonants.
    for pair in vowel_ranges.windows(2) {
        let (_v1_start, v1_end) = pair[0];
        let (v2_start, _v2_end) = pair[1];
        let cons_count = v2_start - v1_end;

        match cons_count {
            0 => {
                // Adjacent vowels already handled in hiatus step above.
            }
            1 => {
                // One consonant goes to the following syllable: V-CV
                out.push(v1_end);
            }
            2 => {
                // Two consonants: VC-CV or V-CCV if inseparable
                if is_inseparable(lower[v1_end], lower[v1_end + 1]) {
                    out.push(v1_end);
                } else {
                    out.push(v1_end + 1);
                }
            }
            3 => {
                // Three consonants:
                // If last two form inseparable group (e.g., com-prar, es-cri-bir): V-C-CCV -> break after 1st
                if is_inseparable(lower[v1_end + 1], lower[v1_end + 2]) {
                    out.push(v1_end + 1);
                } else {
                    // Otherwise break after 2nd (e.g., cons-tan-te, ins-ti-tu-to)
                    out.push(v1_end + 2);
                }
            }
            4 => {
                // Four consonants: VC-C-CCV or VCC-CCV (e.g., cons-truir, obs-truc-ción)
                if is_inseparable(lower[v1_end + 2], lower[v1_end + 3]) {
                    out.push(v1_end + 2);
                } else {
                    out.push(v1_end + 2);
                }
            }
            _ => {
                // 5 or more consonants (rare/foreign): break before the last two if inseparable, else before last
                if cons_count >= 2 && is_inseparable(lower[v2_start - 2], lower[v2_start - 1]) {
                    out.push(v2_start - 2);
                } else {
                    out.push(v2_start - 1);
                }
            }
        }
    }

    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(w: &str) -> String {
        let chars: Vec<char> = w.chars().collect();
        let pts = points(&chars);
        let mut s = String::new();
        for (i, c) in chars.iter().enumerate() {
            if pts.contains(&i) {
                s.push('-');
            }
            s.push(*c);
        }
        s
    }

    #[test]
    fn spanish_syllabification_rules() {
        for (word, expected) in [
            ("casa", "ca-sa"),
            ("mama", "ma-ma"),
            ("mamá", "ma-má"),
            ("mexico", "me-xi-co"),
            ("méxico", "mé-xi-co"),
            ("español", "es-pa-ñol"),
            ("perro", "pe-rro"),
            ("calle", "ca-lle"),
            ("chico", "chi-co"),
            ("llave", "lla-ve"),
            ("hablar", "ha-blar"),
            ("abrazar", "a-bra-zar"),
            ("atleta", "a-tle-ta"),
            ("escribir", "es-cri-bir"),
            ("comprar", "com-prar"),
            ("constante", "cons-tan-te"),
            ("instituto", "ins-ti-tu-to"),
            ("construir", "cons-truir"),
            ("obstruccion", "obs-truc-cion"),
            ("teatro", "te-a-tro"),
            ("caos", "ca-os"),
            ("leer", "le-er"),
            ("poeta", "po-e-ta"),
            ("tío", "tí-o"),
            ("baúl", "ba-úl"),
            ("país", "pa-ís"),
            ("aéreo", "a-é-re-o"),
            ("abuelo", "a-bue-lo"),
            ("cielo", "cie-lo"),
            ("bueno", "bue-no"),
            ("cuidado", "cui-da-do"),
            ("computadora", "com-pu-ta-do-ra"),
            ("desarrollo", "de-sa-rro-llo"),
        ] {
            assert_eq!(split(word), expected, "{word}");
        }
    }
}
