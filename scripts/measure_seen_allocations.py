#!/usr/bin/env python3
"""Count Rust allocations in selected production workloads using a temporary harness."""

import argparse
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--toolchain", default="1.92.0")
    parser.add_argument(
        "--benchmark", choices=["seen_requests", "url_params", "csv_rows"], default="seen_requests"
    )
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[1]
    workload = (repository / "benches" / f"{args.benchmark}.rs").as_posix()
    # A raw Rust string preserves paths with spaces, quotes, Unicode or backslashes.
    delimiter = "#"
    while '"' + delimiter in workload:
        delimiter += "#"
    rust_path = f'r{delimiter}"{workload}"{delimiter}'
    with tempfile.TemporaryDirectory(prefix="silkworm-seen-alloc-") as directory:
        root = Path(directory)
        (root / "src").mkdir()
        project_dependency = ""
        project_imports = ""
        if args.benchmark == "url_params":
            repository_path = json.dumps(repository.as_posix(), ensure_ascii=False)
            project_dependency = f"silkworm-rs = {{ path = {repository_path} }}\n"
            lock = (repository / "Cargo.lock").read_text()
            match = re.search(
                r'\[\[package\]\]\nname = "url"\nversion = "([^"\n]+)"', lock
            )
            if match is None:
                raise RuntimeError("Locked url dependency not found")
            url_version = match.group(1)
            project_dependency += f'url = "={url_version}"\n'
            project_imports = "pub use silkworm::{errors, types};\n"
            shutil.copyfile(repository / "Cargo.lock", root / "Cargo.lock")
        (root / "Cargo.toml").write_text(
            '[package]\nname = "silkworm-seen-alloc"\nversion = "0.0.0"\n'
            'edition = "2024"\n[dependencies]\nstats_alloc = "=0.1.10"\n'
            + project_dependency + '[profile.release]\nlto = "fat"\ncodegen-units = 1\n',
            encoding="utf-8",
        )
        (root / "src/main.rs").write_text(
            project_imports + '#[allow(dead_code)]\n'
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
