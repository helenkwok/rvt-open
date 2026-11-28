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
(`vendor/`), whose tests are their authors'. A test attribute applied conditionally,
`#[cfg_attr(<predicate>, test)]`, nested `cfg_attr` included, counts too.

`--write` rewrites the list from the source, but refuses to raise a number. `--base <rev>`
also fails where the list itself holds a number above the list at <rev> (CI passes the pull
request's base), so a raise cannot land by editing the list by hand.

Usage: python3 tools/ci/unit-test-counts.py [--write] [--base <rev>]
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
NAME = re.compile(r"(?:[a-z_]+::)*[a-z_]*test[a-z_]*\s*(?:\(|$)")
CFG_ATTR = re.compile(r"^\s*#\[\s*cfg_attr\s*\(")


def top_level(text):
    """`text` split at its top-level commas (outside brackets and strings)."""
    parts, depth, quoted, cur = [], 0, False, ""
    for ch in text:
        if ch == '"':
            quoted = not quoted
        elif not quoted and ch in "([{":
            depth += 1
        elif not quoted and ch in ")]}":
            depth -= 1
        if ch == "," and depth == 0 and not quoted:
            parts.append(cur.strip())
            cur = ""
        else:
            cur += ch
    parts.append(cur.strip())
    return [p for p in parts if p]


def applies_a_test(inner):
    """Whether `cfg_attr(..)`'s inside (predicate, attributes) applies a test attribute."""
    for attr in top_level(inner)[1:]:
        if attr.startswith("cfg_attr") and attr[len("cfg_attr"):].lstrip().startswith("("):
            body = attr[attr.index("(") + 1:attr.rindex(")")]
            if applies_a_test(body):
                return True
        elif NAME.fullmatch(attr.split("(", 1)[0].strip() + ("(" if "(" in attr else "")):
            return True
    return False


def tests_in(text):
    lines = text.splitlines()
    n, i = 0, 0
    while i < len(lines):
        line = lines[i]
        if TEST.match(line):
            n += 1
        elif CFG_ATTR.match(line):
            # Join the attribute's lines until its brackets close: rustfmt splits long ones.
            attr = line
            while attr.count("[") > attr.count("]") and i + 1 < len(lines):
                i += 1
                attr += " " + lines[i].strip()
            body = attr[attr.index("(") + 1:attr.rindex(")")]
            n += applies_a_test(body)
        i += 1
    return n


def counted():
    files = subprocess.run(["git", "ls-files", "*.rs"], check=True, capture_output=True, text=True).stdout.split()
    out = {}
    for path in files:
        parts = pathlib.PurePosixPath(path).parts
        if parts[0] == "vendor" or "src" not in parts[:-1]:
            continue
        n = tests_in(pathlib.Path(path).read_text(encoding="utf-8"))
        if n:
            out[path] = n
    return out


def listed(text=None):
    out = {}
    for line in (LIST.read_text(encoding="utf-8") if text is None else text).splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            count, path = line.split(maxsplit=1)
            out[path] = int(count)
    return out


def raised(was, now):
    """(path, before, after) for each number `now` holds above `was` (a new file counts from 0)."""
    return [(p, was.get(p, 0), n) for p, n in sorted(now.items()) if n > was.get(p, 0)]


def main():
    args = sys.argv[1:]
    base = None
    if "--base" in args:
        at = args.index("--base")
        base = args[at + 1]
        del args[at:at + 2]
    now = counted()
    if base:
        rel = LIST.resolve().relative_to(pathlib.Path.cwd().resolve()).as_posix()
        shown = subprocess.run(["git", "show", f"{base}:{rel}"], capture_output=True, text=True)
        if shown.returncode == 0:
            up = raised(listed(shown.stdout), listed())
            for path, was, is_ in up:
                print(f"::error file={path}::the list raises {path} from {was} to {is_} unit tests: a number is never raised")
            if up:
                return 1
    if args == ["--write"]:
        up = raised(listed(), now) if LIST.exists() else []
        for path, was, is_ in up:
            print(f"::error file={path}::{path} holds {is_} unit tests and the list says {was}: not written, a number is never raised")
        if up:
            return 1
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
