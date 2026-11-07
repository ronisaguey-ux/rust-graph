//! The hand-written JSON writer: strings are the only thing that needs care.

pub fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

pub fn push_list(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(out, value);
    }
    out.push(']');
}

pub fn push_optional(out: &mut String, value: Option<&str>) {
    match value {
        Some(text) => push_string(out, text),
        None => out.push_str("null"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_escape_quotes_and_controls() {
        let mut out = String::new();
        push_string(&mut out, "a\"b\\c\n\u{1}é");
        assert_eq!(out, "\"a\\\"b\\\\c\\n\\u0001é\"");
    }

    #[test]
    fn lists_and_nulls() {
        let mut out = String::new();
        push_list(&mut out, &["a".to_string(), "b".to_string()]);
        push_optional(&mut out, None);
        assert_eq!(out, "[\"a\",\"b\"]null");
    }
}
