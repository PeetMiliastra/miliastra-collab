//! Encrypted metadata framing. The coordinator never holds decryption keys.
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub v: u32,
    pub k: String,
    pub p: String,
    pub c: String,
    pub n: String,
    pub d: String,
}
pub fn parse(value: &str) -> Result<Envelope> {
    ensure!(value.len() <= 90000, "Encrypted metadata is too large");
    let e: Envelope = serde_json::from_str(
        value
            .strip_prefix("m2:")
            .ok_or_else(|| anyhow::anyhow!("Encrypted metadata required"))?,
    )?;
    let hex = |s: &str, len: usize| s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit());
    ensure!(
        e.v == 2
            && hex(&e.k, 64)
            && hex(&e.c, 64)
            && hex(&e.n, 24)
            && e.p.len() <= 256
            && e.d.len() >= 544
            && e.d.len() <= 65568
            && e.d.len() % 2 == 0
            && e.d.bytes().all(|b| b.is_ascii_hexdigit())
            && (e.d.len() / 2 - 16).is_power_of_two(),
        "Invalid encrypted metadata"
    );
    Ok(e)
}
pub fn field(value: &str, purpose: &str) -> Result<()> {
    ensure!(
        parse(value)?.p == purpose,
        "Encrypted metadata belongs to a different field"
    );
    Ok(())
}
#[cfg(test)]
pub fn fixture(purpose: &str, value: &str) -> String {
    format!(
        "m2:{}",
        serde_json::to_string(&Envelope {
            v: 2,
            k: "a".repeat(64),
            p: purpose.into(),
            c: crate::hash(value.as_bytes()),
            n: "b".repeat(24),
            d: "c".repeat(544)
        })
        .unwrap()
    )
}

pub fn resource(value: &str) -> bool {
    value == "shared" || value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
