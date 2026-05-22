use std::sync::LazyLock;

use anyhow::{Context, bail};
use regex::Regex;
use sha1::{Digest, Sha1};

const MAX_DIFFICULTY: u32 = 7;

static WORK_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"var\s+work\s*=\s*"([0-9a-fA-F-]+)""#).unwrap());

static DIFFICULTY_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+difficulty\s*=\s*(\d+)").unwrap());

pub fn compute(uuid: &str, difficulty: u32) -> anyhow::Result<String> {
    if difficulty > MAX_DIFFICULTY {
        bail!("o2 PoW difficulty {difficulty} exceeds cap of {MAX_DIFFICULTY}");
    }

    let target = "0".repeat(difficulty as usize);
    let mut nonce: u64 = 0;

    loop {
        let mut hasher = Sha1::new();
        hasher.update(uuid.as_bytes());
        hasher.update(nonce.to_string().as_bytes());
        let hash = hex::encode(hasher.finalize());

        if hash.starts_with(&target) {
            return Ok(nonce.to_string());
        }

        nonce += 1;
    }
}

pub fn solve(script: &str) -> anyhow::Result<String> {
    let work = WORK_PATTERN
        .captures(script)
        .and_then(|capture| capture.get(1))
        .context("Failed to extract work uuid from PoW script")?
        .as_str();

    let difficulty: u32 = DIFFICULTY_PATTERN
        .captures(script)
        .and_then(|capture| capture.get(1))
        .context("Failed to extract difficulty from PoW script")?
        .as_str()
        .parse()
        .context("Failed to parse PoW difficulty as integer")?;

    compute(work, difficulty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(uuid: &str, nonce: &str) -> String {
        let mut hasher = Sha1::new();
        hasher.update(uuid.as_bytes());
        hasher.update(nonce.as_bytes());
        hex::encode(hasher.finalize())
    }

    #[test]
    fn compute_produces_nonce_whose_hash_starts_with_difficulty_zeros() {
        let uuid = "abc-123";
        let nonce = compute(uuid, 3).unwrap();
        assert!(hash(uuid, &nonce).starts_with("000"));
    }

    #[test]
    fn compute_with_zero_difficulty_returns_first_nonce() {
        let nonce = compute("any-uuid", 0).unwrap();
        assert_eq!(nonce, "0");
    }

    #[test]
    fn compute_rejects_difficulty_above_cap() {
        let result = compute("abc", MAX_DIFFICULTY + 1);
        assert!(result.is_err());
    }

    #[test]
    fn solve_extracts_work_and_difficulty_from_script() {
        let script = r#"
            startProofOfWork();
            var work = "abc-123";
            var difficulty = 2;
        "#;
        let nonce = solve(script).unwrap();
        assert!(hash("abc-123", &nonce).starts_with("00"));
    }

    #[test]
    fn solve_rejects_script_missing_work() {
        let script = "var difficulty = 1;";
        assert!(solve(script).is_err());
    }

    #[test]
    fn solve_rejects_script_missing_difficulty() {
        let script = r#"var work = "abc";"#;
        assert!(solve(script).is_err());
    }
}
