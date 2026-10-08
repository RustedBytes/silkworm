#!/usr/bin/env python3
"""Count allocations in the production seen-cache workloads without repo dependencies."""

import argparse
import subprocess
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", default="1.92.0")
    args = parser.parse_args()
    workload = (Path(__file__).resolve().parents[1] / "benches/seen_requests.rs").as_posix()
    # A raw Rust string preserves paths with spaces, quotes, Unicode or backslashes.
    delimiter = "#"
    while '"' + delimiter in workload:
        delimiter += "#"
    rust_path = f'r{delimiter}"{workload}"{delimiter}'
    with tempfile.TemporaryDirectory(prefix="silkworm-seen-alloc-") as directory:
        root = Path(directory)
        (root / "src").mkdir()
        (root / "Cargo.toml").write_text(
            '[package]\nname = "silkworm-seen-alloc"\nversion = "0.0.0"\n'
            'edition = "2024"\n[dependencies]\nstats_alloc = "=0.1.10"\n'
            '[profile.release]\nlto = "fat"\ncodegen-units = 1\n',
            encoding="utf-8",
        )
        (root / "src/main.rs").write_text(
            '#[allow(dead_code)]\n'
            f'#[path = {rust_path}]\nmod workload;\n'
            'use std::alloc::System;\n'
            'use stats_alloc::{StatsAlloc, Region, INSTRUMENTED_SYSTEM};\n'
            '#[global_allocator]\n'
            'static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;\n'
            'fn main() {\n'
            '    workload::scenarios(|name, work| {\n'
            '        let region = Region::new(GLOBAL);\n'
            '        work();\n'
            '        let s = region.change();\n'
            '        println!("case={name} operations=20000 allocations={} '
            'reallocations={} deallocations={} bytes_allocated={} '
            'bytes_deallocated={}", s.allocations, s.reallocations, '
            's.deallocations, s.bytes_allocated, s.bytes_deallocated);\n'
            '    });\n}\n',
            encoding="utf-8",
        )
        subprocess.run(
            ["cargo", f"+{args.toolchain}", "run", "--release", "--manifest-path",
             str(root / "Cargo.toml")],
            check=True,
        )


if __name__ == "__main__":
    main()
