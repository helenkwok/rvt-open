//! `report.json`: what was read, what was written, and everything skipped or not drawn, with a
//! reason. Deterministic for the same input apart from `timings_ms`.

use crate::{Drawing2d, Space};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// The report of one conversion.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub input: Input,
    pub reader: String,
    pub version: String,
    pub units: i16,
    pub code_page: String,
    pub counts: Counts,
    /// Reader notes: skipped records, recovery, diagnostics.
    pub notes: Vec<String>,
    /// What an output left out, per output.
    pub skipped: BTreeMap<String, Vec<String>>,
    /// Entities not drawn in the SVG, by type.
    pub not_drawn: BTreeMap<String, usize>,
    /// Files written, by name.
    pub outputs: Vec<String>,
    pub timings_ms: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Input {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Counts {
    pub entities: usize,
    pub model_space: usize,
    pub paper_space: usize,
    pub in_blocks: usize,
    pub by_type: BTreeMap<String, usize>,
    pub layers: usize,
    pub blocks: usize,
    pub xrefs: usize,
    pub layouts: Vec<String>,
}

impl Report {
    /// A report for `bytes` named `name`, read into `d`.
    pub fn new(name: &str, bytes: &[u8], d: &Drawing2d, notes: Vec<String>) -> Report {
        let mut counts = Counts {
            entities: d.entities.len(),
            layers: d.layers.len(),
            blocks: d
                .blocks
                .iter()
                .filter(|b| b.layout.is_none() && Space::of(&b.name) == Space::Block)
                .count(),
            xrefs: d.blocks.iter().filter(|b| b.xref_path.is_some()).count(),
            layouts: d.layouts.iter().map(|l| l.name.clone()).collect(),
            ..Default::default()
        };
        for e in &d.entities {
            *counts.by_type.entry(e.kind.clone()).or_default() += 1;
            match Space::of(&e.owner) {
                Space::Model => counts.model_space += 1,
                Space::Paper => counts.paper_space += 1,
                Space::Block => counts.in_blocks += 1,
            }
        }
        Report {
            input: Input {
                name: name.to_string(),
                bytes: bytes.len() as u64,
                sha256: Sha256::digest(bytes)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect(),
            },
            reader: crate::ENGINE.to_string(),
            version: d.version.clone(),
            units: d.units,
            code_page: d.code_page.clone(),
            counts,
            notes,
            ..Default::default()
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default() + "\n"
    }
}
