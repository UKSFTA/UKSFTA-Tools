//! Local hot-path benchmarks.
//!
//! Run `cargo bench`. divan reports the allocation count for every
//! benchmark through its allocator hook, so a change that allocates per
//! iteration is visible without a profiler.

#![allow(dead_code)]

#[allow(unused_imports)]
#[path = "../src/pbo.rs"]
mod pbo;
#[allow(unused_imports)]
#[path = "../src/util.rs"]
mod util;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[global_allocator]
static ALLOC: divan::AllocProfiler = divan::AllocProfiler::system();

fn main() {
    divan::main();
}

/// A 64 MiB file, built once per bench process.
fn sample_file() -> &'static Path {
    static PATH: OnceLock<PathBuf> = OnceLock::new();
    PATH.get_or_init(|| {
        let path = std::env::temp_dir().join("uksfta-bench-64mib.bin");
        let needs_write = match std::fs::metadata(&path) {
            Ok(meta) => meta.len() != (64 << 20),
            Err(_) => true,
        };
        if needs_write {
            let block = vec![0x5au8; 1 << 20];
            use std::io::Write;
            let mut file = std::fs::File::create(&path).expect("create the sample");
            for _ in 0..64 {
                file.write_all(&block).expect("write the sample");
            }
        }
        path
    })
}

/// 4 MiB of config-like text with embedded binaries, as a PBO body holds.
fn pack_bytes() -> &'static [u8] {
    static DATA: OnceLock<Vec<u8>> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut data = Vec::with_capacity(4 << 20);
        while data.len() < (4 << 20) {
            data.extend_from_slice(
                b"class CfgPatches { class Demo { units[] = {}; requiredVersion = 1; }; };\0",
            );
            data.extend_from_slice(&[0x00, 0x01, 0x02, 0xff, 0xfe]);
        }
        data
    })
}

#[divan::bench(args = [8192, 1 << 20])]
fn hash_64mib(bencher: divan::Bencher, buffer: usize) {
    let path = sample_file();
    bencher.bench(|| util::file_sha256_with_buffer(path, buffer));
}

#[divan::bench]
fn readable_strings_4mib(bencher: divan::Bencher) {
    let data = pack_bytes();
    bencher.bench(|| pbo::readable_strings(divan::black_box(data)));
}

#[divan::bench]
fn keybind_scan_4mib(bencher: divan::Bencher) {
    let text = String::from_utf8_lossy(pack_bytes()).into_owned();
    bencher.bench(|| pbo::cba_keybind_names(divan::black_box(&text)));
}
