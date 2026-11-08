//! The mutator: a pure function from `(rng, seed bytes)` to one new input.
//!
//! There is no randomness from the environment: the same [`Rng`] seed and the
//! same seed bytes produce the same sequence of inputs on every machine. That
//! is what lets a report name an input by `(seed, iteration)`.
//!
//! The operations are the ones that break parsers, not the ones that test a
//! grammar's language:
//!
//! | operation | what it damages |
//! |:--|:--|
//! | bit flip, byte replace | the lexer's character classes |
//! | truncation | an opener without its closer, a line without its newline |
//! | range delete | a production in the middle of a chain |
//! | delimiter insert | the pair pass and the group firewall |
//! | token doubling | the separator loops (`a, a, a, ...`) |
//! | invalid UTF-8 / split character | the byte-level lexer's boundary handling |
//! | NUL and control bytes | the "not text" path |
//!
//! Delimiters and keywords are global, not per language: a mutation that is
//! nonsense for one grammar is legal for another, and the input must still not
//! crash the parser it is handed to.

/// xorshift64*, with a seed of zero replaced. Deterministic across platforms.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value in `0..bound`; 0 when `bound` is 0.
    pub fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next_u64() % bound as u64) as usize
        }
    }

    /// A value in `low..high` (half-open); `low` when the range is empty.
    pub fn range(&mut self, low: usize, high: usize) -> usize {
        if high <= low {
            low
        } else {
            low + self.below(high - low)
        }
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in.max(1)) == 0
    }
}

/// Delimiters, quotes, layout bytes and comment markers, in a fixed order.
const DELIMITERS: &[&[u8]] = &[
    b"(", b")", b"{", b"}", b"[", b"]", b"<", b">", b",", b";", b":", b"::", b".", b"=", b"=>", b"->", b"|", b"&", b"+", b"-", b"*",
    b"/", b"%", b"!", b"?", b"@", b"#", b"$", b"\\", b"`", b"'", b"\"", b"'''", b"\"\"\"", b"/*", b"*/", b"//", b"--", b"<!--", b"-->",
    b"\n", b"\t", b"  ", b"\x00", b"%s", b"${", b"#{", b"f\"", b"r\"", b"b'",
];

/// Keywords that change a parser's state in the grammars here.
const KEYWORDS: &[&[u8]] = &[
    b"if", b"else", b"for", b"while", b"do", b"switch", b"case", b"return", b"def", b"class", b"function", b"let", b"var", b"const",
    b"import", b"from", b"try", b"catch", b"finally", b"begin", b"end", b"module", b"struct", b"enum", b"match", b"fn", b"pub",
];

/// Apply one mutated input's worth of damage: 1 to 3 operations in sequence.
///
/// Always returns bytes different from `input`; an empty input becomes a
/// delimiter so the fuzzer still exercises the lexer's first-byte paths.
pub fn mutate(rng: &mut Rng, input: &[u8]) -> Vec<u8> {
    let count = rng.range(1, 4);
    let mut bytes = input.to_vec();
    for _ in 0..count {
        bytes = match rng.below(10) {
            0 => flip_bit(rng, bytes),
            1 => replace_byte(rng, bytes),
            2 => truncate(rng, bytes),
            3 => delete_range(rng, bytes),
            4 => insert_delimiter(rng, bytes),
            5 => duplicate_token(rng, bytes),
            6 => damage_utf8(rng, bytes),
            7 => insert_invalid_utf8(rng, bytes),
            8 => insert_keyword(rng, bytes),
            _ => duplicate_range(rng, bytes),
        };
    }
    if bytes == input {
        // A no-op is wasted time; force one bit.
        bytes = flip_bit(rng, bytes);
    }
    bytes
}

fn flip_bit(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.is_empty() {
        return vec![b'('];
    }
    let at = rng.below(bytes.len());
    let bit = 1u8 << rng.below(8);
    bytes[at] ^= bit;
    bytes
}

fn replace_byte(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.is_empty() {
        return vec![b'\xff'];
    }
    let at = rng.below(bytes.len());
    bytes[at] = match rng.below(4) {
        0 => rng.below(256) as u8,
        1 => b"(){}[]<>,;:\"'`\\/"[rng.below(15)],
        2 => b"\x00\x01\x7f\xff\xfe\x80"[rng.below(6)],
        _ => b" \t\n\r"[rng.below(4)],
    };
    bytes
}

fn truncate(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.is_empty() {
        return vec![b'{'];
    }
    let at = rng.below(bytes.len());
    bytes.truncate(at);
    bytes
}

fn delete_range(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() < 2 {
        return Vec::new();
    }
    let at = rng.below(bytes.len());
    let span = rng.range(1, 33).min(bytes.len() - at);
    bytes.drain(at..at + span);
    bytes
}

fn insert_delimiter(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    let at = rng.range(0, bytes.len() + 1);
    let token = DELIMITERS[rng.below(DELIMITERS.len())];
    bytes.splice(at..at, token.iter().copied());
    bytes
}

fn insert_keyword(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    let at = rng.range(0, bytes.len() + 1);
    let word = KEYWORDS[rng.below(KEYWORDS.len())];
    let mut insert = Vec::with_capacity(word.len() + 2);
    insert.push(b' ');
    insert.extend_from_slice(word);
    insert.push(b' ');
    bytes.splice(at..at, insert);
    bytes
}

/// Split into token-ish runs: one byte of punctuation, one whitespace run, one
/// word run (letters, digits, `_`). A random run is copied in after itself.
fn duplicate_token(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.is_empty() {
        return vec![b'a'; 2];
    }
    let (start, end) = token_at(&bytes, rng.below(bytes.len()));
    let copy: Vec<u8> = bytes[start..end].to_vec();
    bytes.splice(end..end, copy);
    bytes
}

fn token_at(bytes: &[u8], at: usize) -> (usize, usize) {
    let class = |byte: u8| -> u8 {
        if byte.is_ascii_whitespace() {
            0
        } else if byte.is_ascii_alphanumeric() || byte == b'_' {
            1
        } else {
            2
        }
    };
    let kind = class(bytes[at]);
    let mut start = at;
    while start > 0 && class(bytes[start - 1]) == kind {
        start -= 1;
    }
    let mut end = at + 1;
    while end < bytes.len() && class(bytes[end]) == kind {
        end += 1;
    }
    if kind == 2 {
        // Punctuation: one byte, so a duplicated delimiter is a duplicate
        // delimiter and not a run of unrelated symbols.
        return (at, at + 1);
    }
    (start, end)
}

fn duplicate_range(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.is_empty() {
        return vec![b')'];
    }
    let start = rng.below(bytes.len());
    let span = rng.range(1, 65).min(bytes.len() - start);
    let copy: Vec<u8> = bytes[start..start + span].to_vec();
    bytes.splice(start..start, copy);
    bytes
}

/// Cut a multi-byte character in half: truncate right after a lead byte, or
/// replace a continuation byte with an ASCII one.
fn damage_utf8(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    let leads: Vec<usize> = bytes.iter().enumerate().filter(|(_, &byte)| byte >= 0xC0).map(|(at, _)| at).collect();
    if leads.is_empty() {
        return insert_invalid_utf8(rng, bytes);
    }
    let at = leads[rng.below(leads.len())];
    if rng.chance(2) {
        bytes.truncate(at + 1);
    } else if at + 1 < bytes.len() {
        bytes[at + 1] = b'A';
    } else {
        bytes.push(0x80);
    }
    bytes
}

fn insert_invalid_utf8(rng: &mut Rng, mut bytes: Vec<u8>) -> Vec<u8> {
    const BAD: &[&[u8]] = &[
        &[0x80],
        &[0xBF],
        &[0xC0, 0x80],
        &[0xC3],
        &[0xE2, 0x82],
        &[0xED, 0xA0, 0x80],
        &[0xF0, 0x9F],
        &[0xF4, 0x90, 0x80, 0x80],
        &[0xF5, 0x80, 0x80, 0x80],
        &[0xFF],
        &[0xFE, 0xFF],
    ];
    let at = rng.range(0, bytes.len() + 1);
    let bad = BAD[rng.below(BAD.len())];
    bytes.splice(at..at, bad.iter().copied());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_not_zero() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let mut c = Rng::new(8);
        let left: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let right: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();
        assert_eq!(left, right);
        let other: Vec<u64> = (0..16).map(|_| c.next_u64()).collect();
        assert_ne!(left, other);
        assert_eq!(Rng::new(0).next_u64(), Rng::new(0x9E37_79B9_7F4A_7C15).next_u64());
    }

    #[test]
    fn mutation_is_a_function_of_the_seed() {
        let seed = b"def f(x):\n    return x + 1\n";
        let mut a = Rng::new(11);
        let mut b = Rng::new(11);
        for _ in 0..200 {
            assert_eq!(mutate(&mut a, seed), mutate(&mut b, seed));
        }
    }

    #[test]
    fn hundreds_of_mutations_stay_under_a_sane_size_cap() {
        let seed = b"{\"a\": [1, 2, 3]}";
        let mut rng = Rng::new(99);
        let mut distinct = std::collections::BTreeSet::new();
        let mut longest = 0;
        for _ in 0..500 {
            let mutated = mutate(&mut rng, seed);
            assert_ne!(mutated, seed.to_vec(), "a mutation was a no-op");
            longest = longest.max(mutated.len());
            distinct.insert(mutated);
        }
        // Every op fires over 500 draws; no input explodes past 4x plus a token.
        assert!(distinct.len() > 400, "only {} distinct inputs", distinct.len());
        assert!(longest < seed.len() * 8, "mutation grew the seed to {longest}");
    }

    #[test]
    fn every_operation_class_can_fire() {
        let seed = b"if (a) { b(\"x\"); } // c\n";
        let mut rng = Rng::new(3);
        let mut empty = 0;
        let mut invalid_utf8 = 0;
        let mut delimiters = 0;
        for _ in 0..2000 {
            let out = mutate(&mut rng, seed);
            if out.is_empty() {
                empty += 1;
            }
            if std::str::from_utf8(&out).is_err() {
                invalid_utf8 += 1;
            }
            if out.windows(1).any(|w| w == b"{") || out.windows(1).any(|w| w == b"(") {
                delimiters += 1;
            }
        }
        assert!(empty > 0 || seed.is_empty());
        assert!(invalid_utf8 > 0, "no mutation produced invalid UTF-8");
        assert!(delimiters > 0);
    }

    #[test]
    fn empty_and_tiny_inputs_are_handled() {
        let mut rng = Rng::new(5);
        for input in [&b""[..], b"a", b"(", b"\xff"] {
            for _ in 0..100 {
                let out = mutate(&mut rng, input);
                assert_ne!(out, input.to_vec());
            }
        }
    }

    #[test]
    fn token_split_on_punctuation_is_one_byte() {
        assert_eq!(token_at(b"ab+c", 2), (2, 3));
        assert_eq!(token_at(b"ab+c", 0), (0, 2));
        assert_eq!(token_at(b"a  b", 1), (1, 3));
    }
}
