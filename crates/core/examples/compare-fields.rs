//! Experiment helper: which top-level fields differ between versions of a slice (.gis)
//! or shared level settings (.gilh) file? Read-only.
//!
//! ```text
//! cargo run -p miliastra-core --example compare-fields -- <file> <file> [<file> ...]
//! ```
//!
//! Integers are printed with their value; other fields with their length and a short
//! hash. Fields the app ignores when comparing are marked.
use miliastra_core::binary::{fields, payload, Field};
use std::collections::BTreeMap;

fn describe(f: &Field) -> String {
    if f.wire == 0 {
        let (mut value, mut shift) = (0u64, 0);
        for b in &f.value {
            value |= u64::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        format!("int {value}")
    } else {
        let h = miliastra_core::hash(&f.value);
        format!("{} bytes {}", f.value.len(), &h[..10])
    }
}

fn main() -> anyhow::Result<()> {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(paths.len() >= 2, "Pass two or more files to compare");
    let mut table: BTreeMap<(u64, usize), Vec<String>> = BTreeMap::new();
    for (column, path) in paths.iter().enumerate() {
        let bytes = std::fs::read(path)?;
        let shared = path.ends_with(".gilh");
        let root = fields(payload(&bytes, if shared { 7 } else { 6 })?)?;
        // Slices keep their content in field 2; shared settings at the top level.
        let data = if shared {
            root
        } else {
            let inner = root
                .iter()
                .find(|f| f.number == 2)
                .ok_or_else(|| anyhow::anyhow!("{path}: no slice content"))?;
            fields(&inner.value)?
        };
        let mut seen: BTreeMap<u64, usize> = BTreeMap::new();
        for f in &data {
            let n = seen.entry(f.number).or_default();
            let row = table
                .entry((f.number, *n))
                .or_insert_with(|| vec!["—".into(); paths.len()]);
            row[column] = describe(f);
            *n += 1;
        }
    }
    for (i, p) in paths.iter().enumerate() {
        println!("[{}] {p}", i + 1);
    }
    let mut differing = 0;
    for ((number, index), values) in &table {
        if values.iter().all(|v| v == &values[0]) {
            continue;
        }
        differing += 1;
        let ignored = if [1, 40, 53].contains(number)
            || ([2, 21, 22, 41, 46].contains(number) && paths.iter().all(|p| p.ends_with(".gilh")))
        {
            "  (known metadata leaves ignored; unknown children still compared)"
        } else if [6, 10].contains(number) && paths.iter().all(|p| p.ends_with(".gilh")) {
            "  (compared selectively; derived children ignored)"
        } else {
            ""
        };
        let label = if *index == 0 {
            format!("field {number}")
        } else {
            format!("field {number} #{}", index + 1)
        };
        println!("{label}{ignored}");
        for (i, v) in values.iter().enumerate() {
            println!("    [{}] {v}", i + 1);
        }
    }
    println!(
        "{differing} differing field(s) out of {} compared.",
        table.len()
    );
    // What the app compares to decide whether a file changed. Equal means "no change".
    for (i, path) in paths.iter().enumerate() {
        let bytes = std::fs::read(path)?;
        let section = if path.ends_with(".gilh") {
            "shared"
        } else {
            "slice"
        };
        let fingerprint = if section == "shared" {
            miliastra_core::binary::import_comparison_hash(&bytes, "shared")?
        } else {
            let root = fields(payload(&bytes, 6)?)?;
            let id = root
                .iter()
                .find(|f| f.number == 1)
                .map(|f| f.value.first().copied().unwrap_or(0) as u64)
                .unwrap_or(0);
            miliastra_core::binary::import_comparison_hash(&bytes, &id.to_string())?
        };
        println!("app fingerprint [{}] {}", i + 1, &fingerprint[..12]);
    }
    Ok(())
}
