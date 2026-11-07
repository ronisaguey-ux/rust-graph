//! What one file says about itself. A record depends only on that file's bytes,
//! which is what makes the SHA-256 cache sound: a change elsewhere re-resolves a
//! file's calls but never re-parses it.

pub const KIND_MODULE: u8 = 0;
pub const KIND_FUNCTION: u8 = 1;
pub const KIND_CLASS: u8 = 2;

pub const IMPORT_MODULE: u8 = 0;
pub const IMPORT_STAR: u8 = 1;

#[derive(Default, Debug, Clone)]
pub struct Import {
    pub line: u32,
    /// As written: `app.db`, `./db`, `run.h`, `..pkg`.
    pub module: String,
    /// The member taken from it, if any.
    pub name: String,
    /// The local name. May be dotted: `import a.b` binds `a.b`.
    pub alias: String,
    pub kind: u8,
}

#[derive(Default, Debug, Clone)]
pub struct Call {
    pub path: String,
    pub line: u32,
}

#[derive(Default, Debug, Clone)]
pub struct Symbol {
    /// Unique within the file; the id is `file:key`.
    pub key: String,
    pub name: String,
    pub owner: String,
    pub kind: u8,
    pub line: u32,
    pub end_line: u32,
    pub params: Vec<String>,
    pub bases: Vec<String>,
    /// JSON array elements, see README "flows". Calls are indexes into `calls`.
    pub events: String,
    pub calls: Vec<Call>,
}

#[derive(Default, Debug, Clone)]
pub struct Facts {
    pub tag: String,
    pub lines: u32,
    pub imports: Vec<Import>,
    pub symbols: Vec<Symbol>,
    /// Variables with a known class: `repo = new Repo()`.
    pub vars: Vec<(String, String)>,
}

fn put_u32(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_str(out: &mut Vec<u8>, value: &str) {
    put_u32(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

fn put_list(out: &mut Vec<u8>, values: &[String]) {
    put_u32(out, values.len() as u32);
    for value in values {
        put_str(out, value);
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    fn u32(&mut self) -> Option<u32> {
        let mut value = 0u32;
        let mut shift = 0;
        loop {
            let byte = *self.bytes.get(self.pos)?;
            self.pos += 1;
            value |= u32::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Some(value);
            }
            shift += 7;
            if shift > 28 {
                return None;
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        let length = self.u32()? as usize;
        let slice = self.bytes.get(self.pos..self.pos + length)?;
        self.pos += length;
        String::from_utf8(slice.to_vec()).ok()
    }

    fn list(&mut self) -> Option<Vec<String>> {
        let count = self.u32()?;
        (0..count).map(|_| self.string()).collect()
    }
}

impl Facts {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1024);
        put_str(&mut out, &self.tag);
        put_u32(&mut out, self.lines);
        put_u32(&mut out, self.imports.len() as u32);
        for import in &self.imports {
            put_u32(&mut out, import.line);
            put_str(&mut out, &import.module);
            put_str(&mut out, &import.name);
            put_str(&mut out, &import.alias);
            out.push(import.kind);
        }
        put_u32(&mut out, self.vars.len() as u32);
        for (name, class) in &self.vars {
            put_str(&mut out, name);
            put_str(&mut out, class);
        }
        put_u32(&mut out, self.symbols.len() as u32);
        for symbol in &self.symbols {
            put_str(&mut out, &symbol.key);
            put_str(&mut out, &symbol.name);
            put_str(&mut out, &symbol.owner);
            out.push(symbol.kind);
            put_u32(&mut out, symbol.line);
            put_u32(&mut out, symbol.end_line);
            put_list(&mut out, &symbol.params);
            put_list(&mut out, &symbol.bases);
            put_str(&mut out, &symbol.events);
            put_u32(&mut out, symbol.calls.len() as u32);
            for call in &symbol.calls {
                put_str(&mut out, &call.path);
                put_u32(&mut out, call.line);
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Option<Facts> {
        let mut reader = Reader::new(bytes);
        let mut facts = Facts { tag: reader.string()?, lines: reader.u32()?, ..Facts::default() };
        for _ in 0..reader.u32()? {
            let line = reader.u32()?;
            let module = reader.string()?;
            let name = reader.string()?;
            let alias = reader.string()?;
            let kind = *reader.bytes.get(reader.pos)?;
            reader.pos += 1;
            facts.imports.push(Import { line, module, name, alias, kind });
        }
        for _ in 0..reader.u32()? {
            facts.vars.push((reader.string()?, reader.string()?));
        }
        for _ in 0..reader.u32()? {
            let key = reader.string()?;
            let name = reader.string()?;
            let owner = reader.string()?;
            let kind = *reader.bytes.get(reader.pos)?;
            reader.pos += 1;
            let line = reader.u32()?;
            let end_line = reader.u32()?;
            let params = reader.list()?;
            let bases = reader.list()?;
            let events = reader.string()?;
            let mut calls = Vec::new();
            for _ in 0..reader.u32()? {
                calls.push(Call { path: reader.string()?, line: reader.u32()? });
            }
            facts.symbols.push(Symbol { key, name, owner, kind, line, end_line, params, bases, events, calls });
        }
        Some(facts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_survives_the_round_trip() {
        let facts = Facts {
            tag: "PY".into(),
            lines: 300,
            imports: vec![Import { line: 1, module: "a.b".into(), name: "c".into(), alias: "d".into(), kind: IMPORT_STAR }],
            vars: vec![("r".into(), "Repo".into())],
            symbols: vec![Symbol {
                key: "A.f".into(),
                name: "f".into(),
                owner: "A".into(),
                kind: KIND_FUNCTION,
                line: 2,
                end_line: 200,
                params: vec!["x".into()],
                bases: vec!["B".into()],
                events: "[\"x\",1,0]".into(),
                calls: vec![Call { path: "os.system".into(), line: 3 }],
            }],
        };
        let back = Facts::decode(&facts.encode()).unwrap();
        assert_eq!(format!("{facts:?}"), format!("{back:?}"));
    }

    #[test]
    fn a_truncated_record_is_rejected_not_misread() {
        let facts = Facts { tag: "PY".into(), lines: 1, ..Facts::default() };
        let bytes = facts.encode();
        assert!(Facts::decode(&bytes[..bytes.len() - 1]).is_none());
    }
}
