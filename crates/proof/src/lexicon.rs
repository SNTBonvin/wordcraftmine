//! Spelling lexicon: case-sensitive word forms with flag bits, binary-searched.
//!
//! Each form carries flag bits (low nibble) and a frequency class (high nibble, [`freq_class`]).
//!
//! On-disk format (`assets/proofing/da.lex`): raw-deflate of `"DCLEX2\n"` followed by one line per
//! form, sorted by bytes. Each line is front-coded against the previous one: a byte with the
//! shared prefix length, `b'0' + flags`, `b'0' + frequency class` (never `\n`), the rest of the
//! form, `\n`.

/// A standalone word form.
pub const WORD: u8 = 1;
/// May start or continue a compound (Danish *sammensætningsform*, noun base, verb infinitive…).
pub const FIRST: u8 = 2;
/// May end a compound (noun, adjective and verb forms).
pub const HEAD: u8 = 4;
/// Listed but not normed (COR flag `U`).
pub const UNNORMED: u8 = 8;

/// Frequency class of a corpus count: 0 unseen, else `⌊log2 count⌋ + 1`, capped at 15. Shifted
/// into the high nibble of an entry byte by [`Lexicon::from_entries`] callers (`flags | class << 4`).
pub fn freq_class(count: u64) -> u8 {
    if count == 0 { 0 } else { (count.ilog2() + 1).min(15) as u8 }
}

#[derive(Clone, Debug, Default)]
pub struct Lexicon {
    words: String,
    offs: Vec<u32>,
    flags: Vec<u8>,
}

const MAGIC: &[u8] = b"DCLEX2\n";

impl Lexicon {
    /// Build from `(form, flags | class << 4)` pairs; duplicates merge flags and keep the higher class.
    pub fn from_entries(mut entries: Vec<(String, u8)>) -> Lexicon {
        entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
        let mut d = Lexicon { words: String::new(), offs: vec![0], flags: Vec::with_capacity(entries.len()) };
        for (w, f) in entries {
            if w.is_empty() || w.contains('\n') {
                continue;
            }
            if !d.is_empty() && d.word(d.len() - 1) == w {
                if let Some(last) = d.flags.last_mut() {
                    *last = (*last | f) & 0x0f | (*last & 0xf0).max(f & 0xf0);
                }
                continue;
            }
            d.words.push_str(&w);
            d.offs.push(d.words.len() as u32);
            d.flags.push(f);
        }
        d
    }

    pub fn len(&self) -> usize {
        self.flags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.flags.is_empty()
    }

    fn word(&self, i: usize) -> &str {
        let a = self.offs.get(i).copied().unwrap_or(0) as usize;
        let b = self.offs.get(i + 1).copied().unwrap_or(0) as usize;
        self.words.get(a..b).unwrap_or("")
    }

    /// Entry byte (`flags | class << 4`) of an exact (case-sensitive) form.
    fn find(&self, form: &str) -> Option<u8> {
        let (mut lo, mut hi) = (0usize, self.len());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.word(mid).as_bytes().cmp(form.as_bytes()) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return self.flags.get(mid).copied(),
            }
        }
        None
    }

    /// Flags of an exact (case-sensitive) form.
    pub fn get(&self, form: &str) -> Option<u8> {
        self.find(form).map(|f| f & 0x0f)
    }

    /// Frequency class of an exact form (0 when unlisted or unseen).
    pub fn freq(&self, form: &str) -> u8 {
        self.find(form).map_or(0, |f| f >> 4)
    }

    /// Does `form` exist with any of the `mask` flags?
    pub fn has(&self, form: &str, mask: u8) -> bool {
        self.get(form).is_some_and(|f| f & mask != 0)
    }

    /// Iterate `(form, flags | class << 4)`.
    pub fn iter(&self) -> impl Iterator<Item = (&str, u8)> + '_ {
        (0..self.len()).map(|i| (self.word(i), self.flags.get(i).copied().unwrap_or(0)))
    }

    /// Serialize (deflate-compressed).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut raw = MAGIC.to_vec();
        let mut prev: &[u8] = &[];
        for (w, f) in self.iter() {
            let h = w.as_bytes();
            let p = prev.iter().zip(h).take_while(|(a, b)| a == b).count().min(255);
            raw.push(p as u8);
            raw.push(b'0' + (f & 0x0f));
            raw.push(b'0' + (f >> 4));
            raw.extend_from_slice(h.get(p..).unwrap_or(&[]));
            raw.push(b'\n');
            prev = h;
        }
        miniz_oxide::deflate::compress_to_vec(&raw, 10)
    }

    /// Deserialize data written by [`Lexicon::to_bytes`].
    pub fn from_bytes(data: &[u8]) -> Result<Lexicon, String> {
        let raw = miniz_oxide::inflate::decompress_to_vec(data).map_err(|e| format!("lexicon: inflate failed: {e:?}"))?;
        let body = raw.strip_prefix(MAGIC).ok_or("lexicon: bad header")?;
        let mut d = Lexicon { words: String::with_capacity(body.len()), offs: vec![0], flags: Vec::new() };
        let mut cur: Vec<u8> = Vec::new();
        let mut i = 0;
        while i < body.len() {
            let (Some(&p), Some(&f), Some(&c)) = (body.get(i), body.get(i + 1), body.get(i + 2)) else {
                return Err("lexicon: truncated".into());
            };
            let rest = body.get(i + 3..).ok_or("lexicon: truncated")?;
            let n = rest.iter().position(|&b| b == b'\n').ok_or("lexicon: truncated")?;
            if p as usize > cur.len() {
                return Err("lexicon: bad prefix".into());
            }
            cur.truncate(p as usize);
            cur.extend_from_slice(rest.get(..n).unwrap_or(&[]));
            i += 3 + n + 1;
            let s = std::str::from_utf8(&cur).map_err(|_| "lexicon: bad utf-8")?;
            d.words.push_str(s);
            d.offs.push(d.words.len() as u32);
            d.flags.push(f.wrapping_sub(b'0') & 0x0f | (c.wrapping_sub(b'0') & 0x0f) << 4);
        }
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let d = Lexicon::from_entries(vec![
            ("hus".into(), WORD | HEAD | freq_class(300) << 4),
            ("hus".into(), FIRST | freq_class(2) << 4),
            ("e-mail".into(), WORD),
            ("København".into(), WORD),
            ("huse".into(), WORD),
        ]);
        let back = Lexicon::from_bytes(&d.to_bytes()).unwrap();
        assert_eq!(back.len(), 4);
        assert_eq!(back.get("hus"), Some(WORD | HEAD | FIRST));
        assert_eq!(back.freq("hus"), 9);
        assert_eq!(back.freq("huse"), 0);
        assert!(back.has("e-mail", WORD));
        assert!(back.get("email").is_none());
        assert!(back.get("københavn").is_none());
        assert!(back.has("København", WORD));
    }

    #[test]
    fn junk_never_panics() {
        for data in [&b""[..], b"\x00\x01", b"garbage bytes here"] {
            let _ = Lexicon::from_bytes(data);
        }
        let mut raw = MAGIC.to_vec();
        raw.extend_from_slice(&[200, b'1', b'0', b'x']);
        assert!(Lexicon::from_bytes(&miniz_oxide::deflate::compress_to_vec(&raw, 1)).is_err());
    }
}
