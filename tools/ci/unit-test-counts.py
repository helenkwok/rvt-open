#!/usr/bin/env python3
"""Hold the number of unit tests in each source file to the list in unit-test-counts.txt.

Unit tests are banned in this repository (CONTRIBUTING.md): no new ones. The ones the source
still holds are listed here, file by file, so the
ban is checked rather than trusted: a file that gains a unit test, or a new file that holds
one, fails this check. A file whose count falls (a test removed, or moved to an end-to-end
target under tests/) fails too until the same change lowers its number, so the list stays
exact. Numbers only ever fall.

A unit test is a `#[test]` attribute, or a namespaced or named one such as `#[tokio::test]`,
`#[test_case(..)]` or one with arguments (at the start of a line, whatever follows it), in a
tracked `.rs` file under a crate's `src/` folder, `src/**/tests/` included; the end-to-end
targets in each crate's top-level `tests/` folder are not counted, nor vendored crates
(`vendor/`), whose tests are their authors'.

Usage: python3 tools/ci/unit-test-counts.py [--write]
"""
import pathlib
import re
import subprocess
import sys

LIST = pathlib.Path(__file__).with_name("unit-test-counts.txt")
# `#[test]`, the namespaced and named kinds (`#[tokio::test]`, `#[wasm_bindgen_test]`,
# `#[test_case(..)]`) and any of them with arguments (`#[tokio::test(flavor = ..)]`); never
# `#[cfg(test)]`, whose name is not a test's.
# The name is enough, followed by `]` or `(`: rustfmt splits a long attribute across lines.
TEST = re.compile(r"^\s*#\[(?:[a-z_]+::)*[a-z_]*test[a-z_]*[(\]]")


def counted():
    files = subprocess.run(["git", "ls-files", "*.rs"], check=True, capture_output=True, text=True).stdout.split()
    out = {}
    for path in files:
        parts = pathlib.PurePosixPath(path).parts
        if parts[0] == "vendor" or "src" not in parts[:-1]:
            continue
        n = sum(1 for line in pathlib.Path(path).read_text(encoding="utf-8").splitlines() if TEST.match(line))
        if n:
            out[path] = n
    return out


def listed():
    out = {}
    for line in LIST.read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            count, path = line.split(maxsplit=1)
            out[path] = int(count)
    return out


def main():
    now = counted()
    if sys.argv[1:] == ["--write"]:
        head = "".join(l + "\n" for l in LIST.read_text(encoding="utf-8").splitlines() if l.startswith("#")) if LIST.exists() else ""
        LIST.write_text(head + "".join(f"{n:4d} {path}\n" for path, n in sorted(now.items())), encoding="utf-8")
        print(f"wrote {sum(now.values())} unit tests in {len(now)} files")
        return 0
    want = listed()
    wrong = sorted(set(now) | set(want))
    wrong = [(p, want.get(p, 0), now.get(p, 0)) for p in wrong if want.get(p, 0) != now.get(p, 0)]
    if not wrong:
        print(f"unit tests: {sum(now.values())} in {len(now)} files, as listed")
        return 0
    for path, was, is_ in wrong:
        how = "gained" if is_ > was else "lost"
        print(f"::error file={path}::{path} holds {is_} unit tests and the list says {was}: it {how} {abs(is_ - was)}")
    print("Unit tests are banned (CONTRIBUTING.md): show a change working end to end under tests/ instead.")
    print("Where a file lost one, lower its number in tools/ci/unit-test-counts.txt in the same change")
    print("(python3 tools/ci/unit-test-counts.py --write); a number is never raised.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
