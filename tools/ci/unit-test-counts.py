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
tracked `.rs` file outside a crate's top-level `tests/` folder (a `tests` folder with no `src`
above it), where the end-to-end targets live; `src/**/tests/`, `examples/` and `benches/` are
counted. Vendored crates (`vendor/`) are not: their tests are their authors'. A test attribute applied conditionally,
`#[cfg_attr(<predicate>, test)]`, nested `cfg_attr` included, counts too.

`--write` rewrites the list from the source, but refuses to raise a number. `--base <rev>`
also fails where the list itself holds a number above the list at <rev> (CI passes the pull
request's base; where the base holds no list yet, the counts read from its source), so a
raise cannot land by editing the list by hand.

`--self-test` checks the attribute forms in FORMS.

Usage: python3 tools/ci/unit-test-counts.py [--write] [--base <rev>] | --self-test
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


def scan(text):
    """Yield (char, depth before it, inside a string) for `text`, Rust strings skipped whole."""
    depth, quoted, escaped = 0, False, False
    for ch in text:
        yield ch, depth, quoted
        if quoted:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                quoted = False
        elif ch == '"':
            quoted = True
        elif ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1


def open_brackets(text):
    """How many brackets `text` leaves open, not counting those inside strings."""
    depth = 0
    for ch, before, quoted in scan(text):
        if not quoted and ch != '"':
            depth = before + (ch in "([{") - (ch in ")]}")
    return depth


def top_level(text):
    """`text` split at its top-level commas (outside brackets and strings)."""
    parts, cur = [], ""
    for ch, depth, quoted in scan(text):
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
            while open_brackets(attr) > 0 and i + 1 < len(lines):
                i += 1
                attr += " " + lines[i].strip()
            body = attr[attr.index("(") + 1:attr.rindex(")")]
            n += applies_a_test(body)
        i += 1
    return n


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def at_revision(rev, paths):
    """The text of each of `paths` at `rev`, read through one `git cat-file --batch`."""
    batch = subprocess.run(["git", "cat-file", "--batch"], input="".join(f"{rev}:{p}\n" for p in paths).encode(),
                           check=True, capture_output=True).stdout
    out, at = {}, 0
    for path in paths:
        header_end = batch.index(b"\n", at)
        size = int(batch[at:header_end].split()[2])
        out[path] = batch[header_end + 1:header_end + 1 + size].decode("utf-8")
        at = header_end + 1 + size + 1
    return out


def source(path):
    """Whether `path` can hold a unit test: not vendored, and not under a crate's top-level
    `tests/` folder (a `tests` folder with no `src` above it), where the end-to-end targets
    live. A `tests` folder inside `src/` counts, and so do `examples/` and `benches/`."""
    parts = pathlib.PurePosixPath(path).parts
    folders = parts[:-1]
    if parts[0] == "vendor":
        return False
    return not ("tests" in folders and "src" not in folders[: folders.index("tests")])


def counted(rev=None):
    """Unit tests per source file, in the working tree or at revision `rev`."""
    files = git("ls-files", "*.rs").split() if rev is None else git("ls-tree", "-r", "--name-only", rev).split()
    files = [path for path in files if path.endswith(".rs") and source(path)]
    texts = ({path: pathlib.Path(path).read_text(encoding="utf-8") for path in files} if rev is None
             else at_revision(rev, files))
    return {path: n for path, n in ((path, tests_in(texts[path])) for path in files) if n}


def listed(text=None):
    out = {}
    for line in (LIST.read_text(encoding="utf-8") if text is None else text).splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            count, path = line.split(maxsplit=1)
            out[path] = int(count)
    return out


# The attribute forms the count must and must not take, checked by `--self-test` in CI.
FORMS = [
    ("#[test]\nfn a() {}", 1),
    ("    #[tokio::test]\n", 1),
    ("#[tokio::test(flavor = \"multi_thread\")]", 1),
    ("#[test_case(1, 2)]\n#[wasm_bindgen_test]\n#[test] fn x() {}", 3),
    ("#[tokio::test(\n    flavor = \"current_thread\"\n)]", 1),
    ("#[cfg_attr(test, test)]", 1),
    ("#[cfg_attr(unix, cfg_attr(test, test))]", 1),
    ("#[cfg_attr(\n    all(test, unix),\n    tokio::test(flavor = \"multi_thread\")\n)]", 1),
    ("#[cfg_attr(any(test), doc = \"]\",\n    test\n)]\nfn t() {}", 1),
    ("#[cfg_attr(test, doc = \"a, test\")]", 0),
    ("#[cfg_attr(test, derive(Debug))]\n#[cfg_attr(x, cfg_attr(test, derive(Debug)))]", 0),
    ("#[cfg(test)]\n#[should_panic]\n// #[test]\n#[allow(clippy::x)]", 0),
]


def self_test():
    paths = {"src/a/tests/mod.rs": True, "examples/x.rs": True, "benches/b.rs": True,
             "rvt-open-mcp/src/lib.rs": True, "tests/e2e.rs": False, "rvt-open-mcp/tests/y.rs": False,
             "vendor/c/src/lib.rs": False}
    for path, want in paths.items():
        if source(path) != want:
            print(f"::error::{path} {'should' if want else 'should not'} be counted")
            return 1
    bad = [(text, want, tests_in(text)) for text, want in FORMS if tests_in(text) != want]
    for text, want, got in bad:
        print(f"::error::counted {got} unit tests, not {want}, in {text!r}")
    if not bad:
        print(f"unit test forms: {len(FORMS)} read as expected")
    return 1 if bad else 0


def raised(was, now):
    """(path, before, after) for each number `now` holds above `was` (a new file counts from 0)."""
    return [(p, was.get(p, 0), n) for p, n in sorted(now.items()) if n > was.get(p, 0)]


def main():
    args = sys.argv[1:]
    if args == ["--self-test"]:
        return self_test()
    base = None
    if "--base" in args:
        at = args.index("--base")
        base = args[at + 1]
        del args[at:at + 2]
    now = counted()
    if base:
        # A base that is not there is an error, never a skipped check.
        if subprocess.run(["git", "cat-file", "-e", f"{base}^{{commit}}"], capture_output=True).returncode:
            print(f"::error::--base {base} is not a commit here (fetch it first)")
            return 1
        rel = LIST.resolve().relative_to(pathlib.Path(git("rev-parse", "--show-toplevel").strip()).resolve()).as_posix()
        if rel in git("ls-tree", "-r", "--name-only", base, "--", rel).split():
            was = listed(git("show", f"{base}:{rel}"))
        else:
            # No list at the base yet: its source is the baseline.
            was = counted(base)
        up = raised(was, listed())
        for path, before, after in up:
            print(f"::error file={path}::the list raises {path} from {before} to {after} unit tests: a number is never raised")
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
