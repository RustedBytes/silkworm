#!/usr/bin/env python3
"""Compile and run the bundled offline tests against a framework checkout."""
import argparse
from pathlib import Path
import subprocess
import tempfile
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, required=True)
    args = parser.parse_args()
    root = args.repo.resolve()
    package = tomllib.loads((root / 'Cargo.toml').read_text())['package']
    if package['name'] != 'silkworm-rs':
        parser.error('--repo must be a silkworm-rs framework checkout')
    print(f"Checking resolved checkout API: silkworm-rs {package['version']}", flush=True)
    asset = Path(__file__).resolve().parents[1] / 'assets/crawler.rs'
    tests = root / 'tests'
    tests.mkdir(exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', prefix='skill_examples_', suffix='.rs', dir=tests) as target:
        target.write(asset.read_text())
        target.flush()
        for flags in ([], ['--features', 'xpath'], ['--no-default-features']):
            subprocess.run(['cargo', 'test', '--locked', '--test', Path(target.name).stem, *flags], cwd=root, check=True)
    print('Skill example passed in three feature configurations.')


if __name__ == '__main__':
    main()
