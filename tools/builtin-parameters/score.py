#!/usr/bin/env python3
"""Score src/builtin_parameters/table.rs against the Revit API dump of the public samples (the answer key).

usage: score.py KEY_DIR [--table PATH] [--expected PATH] [--write-expected]

KEY_DIR holds the dump's catalog files: builtin_parameters.tsv (id, name, storage, spec, group, ...),
builtin_enums.tsv (id, raw, display, count) and builtin_categories.tsv (id, name, count). The dump is not
published; this repository keeps only the counts below (expected-score.tsv) and the hash of each key file.
For each field it counts, over the ids the key has: filled (the table has a value), match, differ, empty
(the table has none). It exits 1 when `match` falls below, or `differ` rises above, the counts in
expected-score.tsv: a field filled from a source that disagrees with the key, or a match lost, fails."""
import csv, hashlib, os, re, sys
here = os.path.dirname(os.path.abspath(__file__))
args = sys.argv[1:]
key = args.pop(0)
opt = {}
while args:
    a = args.pop(0)
    opt[a] = True if a == '--write-expected' else args.pop(0)
table = opt.get('--table', os.path.join(here, '..', '..', 'src', 'builtin_parameters', 'table.rs'))
expected = opt.get('--expected', os.path.join(here, 'expected-score.tsv'))
src = open(table, encoding='utf-8').read()
tup = re.compile(r'\(\s*(-?\d+),\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)",\s*"([-A-Z]{6})",?\s*\)', re.S)
un = lambda s: s.replace('\\"', '"').replace('\\\\', '\\')
cat = {int(m[0]): [un(x) for x in m[1:]] for m in tup.findall(src.split('PARAMETERS: &[', 1)[1].split('ENUM_TEXTS', 1)[0])}
enum_texts = {}
for m in re.finditer(r'\((-?\d+), (-?\d+), "((?:[^"\\]|\\.)*)"\),', src.split('ENUM_TEXTS', 1)[1].split('CATEGORIES', 1)[0]):
    enum_texts[(int(m[1]), int(m[2]))] = un(m[3])


def rows(name):
    with open(os.path.join(key, name), encoding='utf-8') as f:
        return [l.rstrip('\n').split('\t') for l in f if not l.startswith('#') and l.strip()]


def sha(name):
    return hashlib.sha256(open(os.path.join(key, name), 'rb').read()).hexdigest()[:16]


kp = {int(r[0]): r for r in rows('builtin_parameters.tsv')}
counts = {}


def tally(field, pairs):
    f = m = d = e = 0
    for have, want in pairs:
        if not have:
            e += 1
        else:
            f += 1
            if have == want:
                m += 1
            else:
                d += 1
    counts[field] = (len(pairs), f, m, d, e)


# column in the table tuple: name 0, enum 1, storage 2, spec 3, group 4
tally('name', [(cat.get(i, [''])[0], r[1]) for i, r in kp.items()])
tally('storage', [(cat.get(i, [''] * 6)[2], r[2]) for i, r in kp.items() if r[2]])
tally('spec', [(cat.get(i, [''] * 6)[3], r[3]) for i, r in kp.items() if r[3]])
tally('group', [(cat.get(i, [''] * 6)[4], r[4]) for i, r in kp.items() if len(r) > 4 and r[4]])
ke = {}
for r in rows('builtin_enums.tsv'):
    if r[1].lstrip('-').isdigit():
        k = (int(r[0]), int(r[1]))
        if k not in ke or int(r[3]) > ke[k][1]:
            ke[k] = (r[2], int(r[3]))
tally('enum_text', [(enum_texts.get(k, ''), v[0]) for k, v in ke.items()])
kc = {int(r[0]): r[1] for r in rows('builtin_categories.tsv') if int(r[0]) < 0 and r[1]}
# category names: the table's (Revit's IFC export), else program_names' help term
ct = {}
for m in re.finditer(r'\((-?\d+), "((?:[^"\\]|\\.)*)", "((?:[^"\\]|\\.)*)"\),', src.split('CATEGORIES', 1)[1].split('LOCALIZED', 1)[0]):
    ct[int(m[1])] = un(m[3])
pn = open(opt.get('--program-names', os.path.join(here, '..', '..', 'src', 'program_names', 'table.rs')), encoding='utf-8').read()
pn_en = {int(m.group(1)): m.group(2) for m in re.finditer(r'\n        (-\d+),\n        \[\n            "((?:[^"\\]|\\.)*)"', pn)}
tally('category_name', [(ct.get(i) or pn_en.get(i, ''), n) for i, n in kc.items()])
out = ['field\tkey_ids\tfilled\tmatch\tdiffer\tempty']
for k, v in counts.items():
    out.append(k + '\t' + '\t'.join(map(str, v)))
hashes = '# key files: ' + ', '.join(f"{n} {sha(n)}" for n in ('builtin_parameters.tsv', 'builtin_enums.tsv', 'builtin_categories.tsv'))
print(hashes)
print('\n'.join(out))
if opt.get('--write-expected'):
    open(expected, 'w').write(hashes + '\n' + '\n'.join(out) + '\n')
    sys.exit(0)
bad = 0
for l in open(expected):
    if l.startswith('#') or l.startswith('field'):
        continue
    f, _, _, em, ed, _ = l.split()
    _, _, m, d, _ = counts[f]
    if m < int(em) or d > int(ed):
        print(f'FAIL {f}: match {m} (expected at least {em}), differ {d} (expected at most {ed})')
        bad = 1
sys.exit(bad)
