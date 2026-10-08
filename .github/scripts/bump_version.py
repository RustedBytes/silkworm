"""Prepare a strictly increasing stable SemVer bump without changing dependencies."""
import argparse
import os
from pathlib import Path
import re
import tomllib

VERSION = re.compile(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", re.ASCII)


def version_parts(value):
    if not VERSION.fullmatch(value):
        raise ValueError("Version must be MAJOR.MINOR.PATCH, without v, leading zeros or prerelease suffixes")
    return tuple(map(int, value.split(".")))


def prepare(root, bump, explicit):
    manifest = root / "Cargo.toml"
    lockfile = root / "Cargo.lock"
    manifest_text = manifest.read_text(encoding="utf-8")
    lock_text = lockfile.read_text(encoding="utf-8")
    package = tomllib.loads(manifest_text)["package"]
    current = package["version"]
    parts = list(version_parts(current))
    if explicit:
        new = explicit
    else:
        index = {"major": 0, "minor": 1, "patch": 2}[bump]
        parts[index] += 1
        parts[index + 1:] = [0] * (2 - index)
        new = ".".join(map(str, parts))
    if version_parts(new) <= version_parts(current):
        raise ValueError(f"Requested version {new} must be greater than {current}")

    matches = [p for p in tomllib.loads(lock_text)["package"]
               if p["name"] == package["name"] and "source" not in p]
    if len(matches) != 1 or matches[0]["version"] != current:
        raise ValueError("Cargo.lock must contain exactly one matching local package at the current version")

    def replace_block(text, block_pattern):
        matches = list(re.finditer(block_pattern, text, re.MULTILINE | re.DOTALL))
        if len(matches) != 1:
            raise ValueError("Expected exactly one package block")
        block = matches[0]
        updated, count = re.subn(r'^version\s*=\s*"[^"\n]+"', f'version = "{new}"',
                                 block.group(), flags=re.MULTILINE)
        if count != 1:
            raise ValueError("Expected exactly one package version")
        return text[:block.start()] + updated + text[block.end():]

    manifest_new = replace_block(manifest_text, r'^\[package\][^\n]*\n.*?(?=^\[|\Z)')
    # Restrict the lockfile change to this source-less root package.
    lock_pattern = (r'^\[\[package\]\]\n(?:(?!^\[\[package\]\]).)*?'
                    r'^name = "' + re.escape(package["name"]) + r'"\n'
                    r'(?:(?!^\[\[package\]\]).)*?(?=^\[\[package\]\]|\Z)')
    lock_new = replace_block(lock_text, lock_pattern)
    if tomllib.loads(manifest_new)["package"]["version"] != new:
        raise ValueError("Manifest verification failed")
    lock_packages = tomllib.loads(lock_new)["package"]
    if next(p for p in lock_packages if p["name"] == package["name"] and "source" not in p)["version"] != new:
        raise ValueError("Lockfile verification failed")
    # Complete validation before either file is changed.
    manifest.write_text(manifest_new, encoding="utf-8")
    lockfile.write_text(lock_new, encoding="utf-8")
    return current, new


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bump", choices=["patch", "minor", "major"], default="patch")
    parser.add_argument("--version", default="")
    args = parser.parse_args()
    try:
        old, new = prepare(Path.cwd(), args.bump, args.version)
    except (ValueError, KeyError) as error:
        parser.error(str(error))
    print(f"silkworm-rs: {old} -> {new}")
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as handle:
            handle.write(f"version={new}\nprevious={old}\n")


if __name__ == "__main__":
    main()
