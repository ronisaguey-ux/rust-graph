//! A small JSON reader and writer, enough for the conformance report.
//!
//! The report has to survive a round trip through a file so `summary`, `gates`
//! and `diff` can run on a scan that happened in another process, and the
//! project's rule is std only. The writer escapes every control byte and the
//! quote and backslash; the reader accepts any valid JSON value and hands back
//! an object tree. Unknown keys are kept, so a report written by a newer tool
//! can still be read by an older one.

use std::fmt::Write as _;

/// One JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(fields) => fields.iter().find(|(name, _)| name == key).map(|(_, value)| value),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(number) => Some(*number),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        let number = self.as_f64()?;
        if number.is_finite() && number >= 0.0 {
            Some(number as u64)
        } else {
            None
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(items) => Some(items),
            _ => None,
        }
    }
}

/// Parse one JSON document. Trailing bytes other than whitespace are an error.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut reader = Reader { bytes: text.as_bytes(), at: 0 };
    reader.space();
    let value = reader.value()?;
    reader.space();
    if reader.at != reader.bytes.len() {
        return Err(format!("trailing bytes at offset {}: {:?}", reader.at, &text[reader.at..]));
    }
    Ok(value)
}

/// Append `text` as a JSON string literal, quotes included.
pub fn push_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Write a number so it round trips and never prints `NaN`/`inf`, which JSON
/// has no spelling for. `None` becomes `null`, never `0`: a measurement that
/// did not happen is not a zero.
pub fn push_number(out: &mut String, number: Option<f64>) {
    match number {
        Some(number) if number.is_finite() => {
            if number.fract() == 0.0 && number.abs() < 1e15 {
                let _ = write!(out, "{}", number as i64);
            } else {
                let _ = write!(out, "{number}");
            }
        }
        _ => out.push_str("null"),
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while matches!(self.bytes.get(self.at), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.space();
        match self.bytes.get(self.at).copied() {
            Some(b'n') => self.literal("null", Value::Null),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::Str),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(byte) => Err(format!("unexpected byte {:?} at offset {}", byte as char, self.at)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, String> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(format!("expected {word} at offset {}", self.at))
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1; // opening quote
        let mut out = String::new();
        loop {
            let byte = *self.bytes.get(self.at).ok_or("unterminated string")?;
            self.at += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = *self.bytes.get(self.at).ok_or("unterminated escape")?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let first = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&first) {
                                if self.bytes.get(self.at) != Some(&b'\\') || self.bytes.get(self.at + 1) != Some(&b'u') {
                                    return Err("unpaired surrogate".to_string());
                                }
                                self.at += 2;
                                let second = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&second) {
                                    return Err("bad low surrogate".to_string());
                                }
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                            } else {
                                first
                            };
                            out.push(char::from_u32(code).ok_or("invalid code point")?);
                        }
                        other => return Err(format!("bad escape \\{}", other as char)),
                    }
                }
                byte if byte < 0x20 => return Err("control byte in string".to_string()),
                _ => {
                    // Copy the UTF-8 sequence the byte starts.
                    let start = self.at - 1;
                    let width = utf8_width(byte);
                    let end = start + width;
                    let slice = self.bytes.get(start..end).ok_or("truncated UTF-8")?;
                    out.push_str(std::str::from_utf8(slice).map_err(|_| "invalid UTF-8 in string")?);
                    self.at = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.bytes.get(self.at..self.at + 4).ok_or("truncated \\u escape")?;
        let text = std::str::from_utf8(digits).map_err(|_| "bad \\u escape")?;
        self.at += 4;
        u32::from_str_radix(text, 16).map_err(|_| "bad \\u escape".to_string())
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.at;
        if self.bytes.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        while matches!(self.bytes.get(self.at), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        if self.bytes.get(self.at) == Some(&b'.') {
            self.at += 1;
            while matches!(self.bytes.get(self.at), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            while matches!(self.bytes.get(self.at), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| "bad number")?;
        text.parse::<f64>().map(Value::Num).map_err(|_| format!("bad number {text:?}"))
    }

    fn array(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(Value::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Arr(items));
                }
                _ => return Err(format!("expected , or ] at offset {}", self.at)),
            }
        }
    }

    fn object(&mut self) -> Result<Value, String> {
        self.at += 1;
        let mut fields = Vec::new();
        self.space();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Value::Obj(fields));
        }
        loop {
            self.space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return Err(format!("expected a key at offset {}", self.at));
            }
            let key = self.string()?;
            self.space();
            if self.bytes.get(self.at) != Some(&b':') {
                return Err(format!("expected : at offset {}", self.at));
            }
            self.at += 1;
            let value = self.value()?;
            fields.push((key, value));
            self.space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Obj(fields));
                }
                _ => return Err(format!("expected , or }} at offset {}", self.at)),
            }
        }
    }
}

fn utf8_width(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_escapes() {
        let text = "quote\" back\\ line\n tab\t snowman \u{2603}";
        let mut out = String::new();
        push_string(&mut out, text);
        match parse(&out).unwrap() {
            Value::Str(back) => assert_eq!(back, text),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reads_a_report_shaped_document() {
        let value = parse(r#"{"a":[1,2.5,null,true],"b":"x\"y","c":{"d":-3e2}}"#).unwrap();
        assert_eq!(value.get("b").unwrap().as_str(), Some("x\"y"));
        assert_eq!(value.get("c").unwrap().get("d").unwrap().as_f64(), Some(-300.0));
        assert_eq!(value.get("a").unwrap().as_array().unwrap().len(), 4);
        assert!(value.get("missing").is_none());
    }

    #[test]
    fn null_is_not_zero() {
        let mut out = String::new();
        push_number(&mut out, None);
        assert_eq!(out, "null");
        out.clear();
        push_number(&mut out, Some(f64::NAN));
        assert_eq!(out, "null");
    }

    #[test]
    fn rejects_trailing_garbage() {
        assert!(parse("{} x").is_err());
        assert!(parse("{\"a\":}").is_err());
    }
}
