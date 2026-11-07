//! The only thing held in memory for the whole tree: paths and a compact row
//! per symbol. Everything else stays in the pack and is read a file at a time.

use std::collections::HashMap;

use crate::facts::{Facts, KIND_CLASS, KIND_MODULE};

pub fn hash(text: &str) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        value = (value ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    value
}

#[derive(Clone, Copy)]
pub struct Row {
    pub name: u64,
    pub owner: u64,
    pub file: u32,
    pub key_at: u32,
    pub key_len: u16,
    pub kind: u8,
}

/// Languages whose files can call each other.
pub fn family(tag: &str) -> u8 {
    match tag {
        "C" | "CPP" | "OBJC" => 1,
        "JS" | "TS" => 2,
        "JAVA" | "KT" | "SCALA" | "GROOVY" => 3,
        other => 16u8.wrapping_add(other.bytes().fold(0, |sum, byte| sum.wrapping_mul(31).wrapping_add(byte))),
    }
}

#[derive(Default)]
pub struct Index {
    pub paths: Vec<String>,
    pub families: Vec<u8>,
    /// File ids by the last component of their module path.
    stems: HashMap<u64, Vec<u32>>,
    keys: String,
    rows: Vec<Row>,
    /// Row numbers sorted by name, for tree-wide lookups.
    by_name: Vec<u32>,
    /// First row of each file; rows are appended file by file.
    file_start: Vec<u32>,
}

fn stem_of(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.split('.').next().unwrap_or(file);
    if matches!(stem, "__init__" | "index" | "mod" | "lib" | "main") {
        let mut parts = path.rsplit('/');
        parts.next();
        return parts.next().unwrap_or(stem);
    }
    stem
}

impl Index {
    pub fn add_file(&mut self, path: &str, facts: &Facts) {
        let file = self.paths.len() as u32;
        self.paths.push(path.to_string());
        self.families.push(family(&facts.tag));
        self.stems.entry(hash(stem_of(path))).or_default().push(file);
        let plain = path.rsplit('/').next().unwrap_or(path).split('.').next().unwrap_or("");
        if plain != stem_of(path) {
            self.stems.entry(hash(plain)).or_default().push(file);
        }
        self.file_start.push(self.rows.len() as u32);
        for symbol in &facts.symbols {
            if symbol.kind == KIND_MODULE {
                continue;
            }
            let key_at = self.keys.len() as u32;
            self.keys.push_str(&symbol.key);
            self.rows.push(Row {
                name: hash(&symbol.name),
                owner: if symbol.owner.is_empty() { 0 } else { hash(&symbol.owner) },
                file,
                key_at,
                key_len: symbol.key.len().min(u16::MAX as usize) as u16,
                kind: symbol.kind,
            });
        }
    }

    pub fn seal(&mut self) {
        self.file_start.push(self.rows.len() as u32);
        self.by_name = (0..self.rows.len() as u32).collect();
        let rows = &self.rows;
        self.by_name.sort_unstable_by_key(|row| (rows[*row as usize].name, *row));
        self.keys.shrink_to_fit();
    }

    pub fn id(&self, row: &Row) -> String {
        let start = row.key_at as usize;
        format!("{}:{}", self.paths[row.file as usize], &self.keys[start..start + row.key_len as usize])
    }

    pub fn rows_in(&self, file: u32) -> &[Row] {
        let start = self.file_start[file as usize] as usize;
        let end = self.file_start[file as usize + 1] as usize;
        &self.rows[start..end]
    }

    /// Top-level definitions of `name` in a file, functions before classes.
    pub fn top_level(&self, file: u32, name: u64) -> Option<Row> {
        let rows = self.rows_in(file);
        rows.iter().find(|row| row.name == name && row.owner == 0 && row.kind != KIND_CLASS).or_else(|| {
            rows.iter().find(|row| row.name == name && row.owner == 0)
        }).copied()
    }

    pub fn member(&self, file: u32, owner: u64, name: u64) -> Option<Row> {
        self.rows_in(file).iter().find(|row| row.name == name && row.owner == owner).copied()
    }

    pub fn class_in(&self, file: u32, name: u64) -> bool {
        self.rows_in(file).iter().any(|row| row.name == name && row.kind == KIND_CLASS)
    }

    /// Every top-level definition of `name` across the tree, in file order.
    pub fn named(&self, name: u64) -> Vec<Row> {
        let start = self.by_name.partition_point(|row| self.rows[*row as usize].name < name);
        self.by_name[start..]
            .iter()
            .map(|row| self.rows[*row as usize])
            .take_while(|row| row.name == name)
            .filter(|row| row.owner == 0)
            .collect()
    }

    pub fn candidates(&self, stem: &str) -> &[u32] {
        self.stems.get(&hash(stem)).map_or(&[], Vec::as_slice)
    }
}
