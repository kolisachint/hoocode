//! Stable-toolchain driver for the fuzz targets in `fuzz/targets/`.
//!
//! The owning crates' test suites include this file with `#[path]`, so the stable
//! `cargo test` run executes every target body on: the empty input, every prefix of
//! each checked-in seed (`fuzz/corpus/<target>/`), and a fixed set of deterministic
//! mutations of each seed. Nightly `cargo fuzz` explores far beyond this.

use std::path::PathBuf;

const MUTATIONS_PER_SEED: u64 = 256;
const INTERESTING: &[u8] = b"\0\n\r\t\"'\\{}[],:/ *#`-=\x1b\x7f\x80\xc2\xe2\xff";

/// Run `body` over the target's seeds, their prefixes and their mutations.
pub fn run(target: &str, body: fn(&[u8])) {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../fuzz/corpus")).join(target);
    let mut seeds: Vec<Vec<u8>> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading seeds in {}: {e}", dir.display()))
        .map(|entry| std::fs::read(entry.expect("seed entry").path()).expect("seed file"))
        .collect();
    seeds.sort();
    assert!(!seeds.is_empty(), "no seeds in {}", dir.display());

    body(b"");
    for (i, seed) in seeds.iter().enumerate() {
        for end in 0..=seed.len() {
            body(&seed[..end]);
        }
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (i as u64 + 1));
        for _ in 0..MUTATIONS_PER_SEED {
            body(&mutate(seed, &mut rng));
        }
    }
}

/// xorshift64: deterministic, so a failure reproduces on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = seed.to_vec();
    for _ in 0..=rng.below(4) {
        let pos = rng.below(out.len() + 1);
        let byte = INTERESTING[rng.below(INTERESTING.len())];
        match rng.below(4) {
            0 if pos < out.len() => out[pos] = byte,
            1 => out.insert(pos, byte),
            2 if pos < out.len() => {
                let end = (pos + 1 + rng.below(8)).min(out.len());
                out.drain(pos..end);
            }
            _ => out.insert(pos, rng.next() as u8),
        }
    }
    out
}
