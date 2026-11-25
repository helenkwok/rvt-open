#!/usr/bin/env python3
"""Group, spec kind and enumeration texts of built-in parameters from Revit's own IFC export.

usage: derive_ifc.py PAIRS_DIR BIP_ENUM.tsv OUT_PREFIX [--releases STEM_RELEASE.tsv]

PAIRS_DIR holds, per exported sample, `<m>.props.tsv` (set, property, IFC value type, count of Revit's own
property sets) and `<m>.pairs.tsv` (an element's IFC property joined with a parameter id the file
stores on that element, by the property name equal to the parameter's Revit 2027 API reference label).
Route 1 (label): a property name equal to the reference label of exactly one id gives that id its
property set (group) and IFC value type. Route 2 (file and label): where the label is shared by
several ids, the element's own stored parameter picks the id. Where both give one, the one whose votes come from the newer
release wins, and route 2 where the release is the same. Route 3 (category and label): where the label is shared by several
ids and exactly one of them has an enum name that starts with the singular of the element's category
(`DOOR_HEIGHT` for the `Height` of a `Doors` element), that id gets the property set and value type
of the property on elements of that category; it only fills an id neither route gave.
The catalog is Revit 2027's, so a vote is counted only among the samples of the newest release that
has one for the id: a group Revit renamed (`Mechanical` to `Mechanical - Flow`) takes its newest name.
A property set whose name holds `Schedule` is a schedule Revit exported as a property set
(`Door Schedule`), not a parameter group: its properties never vote; nor do those of a set whose name holds
no Latin letter (`标识数据`: a family's own group in its author's language, since the exports run Revit in English). (A schedule titled in another language
is not recognised; no count of samples separates it, since the sample projects repeat their schedules.)
`--releases` lists `<sample>\t<release>`; a sample it does not list and whose name ends in `__pf<year>` (optionally
followed by `-<variant>`) counts as that release; any other counts as one release with no year.
Enumeration texts: an integer parameter whose IFC value is a text gives (id, stored integer) -> text (the text of
the newest release that has one),
dropped when the part after its last `: ` is a storey or type name of the sample that wrote it
(`*.names.tsv`).
Writes OUT_PREFIX.group-spec.tsv (id, group, ifc_type, route) and OUT_PREFIX.enum-texts.tsv."""
import argparse, collections, glob, os, re, sys, unicodedata
ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('pairs_dir')
ap.add_argument('bip_enum')
ap.add_argument('out_prefix')
ap.add_argument('--releases')
args = ap.parse_args()
pairs, bip, out = args.pairs_dir, args.bip_enum, args.out_prefix
if not os.path.isdir(pairs):
    sys.exit(f'not a directory: {pairs}')
if not glob.glob(os.path.join(pairs, '*.props.tsv')) or not glob.glob(os.path.join(pairs, '*.pairs.tsv')):
    sys.exit(f'no *.props.tsv and *.pairs.tsv samples in {pairs}')
if not os.path.isfile(bip):
    sys.exit(f'not a file: {bip}')
labels = collections.defaultdict(set)
enums = collections.defaultdict(set)
for l in open(bip):
    if l[0] != '#':
        p = l.rstrip('\n').split('\t')
        enums[int(p[1])].add(p[0])
        if len(p) > 2 and p[2]:
            labels[p[2]].add(int(p[1]))
def sample(fn):
    """The sample a file belongs to: its name without the exact suffix (dots elsewhere stay)."""
    b = os.path.basename(fn)
    for suffix in ('.names.tsv', '.pairs.tsv', '.props.tsv', '.cprops.tsv'):
        if b.endswith(suffix):
            return b[:-len(suffix)]
    return b
release = {}
if args.releases:
    for l in open(args.releases, encoding='utf-8'):
        if l[0] != '#' and l.strip():
            a, b = l.rstrip('\n').split('\t')[:2]
            release[unicodedata.normalize('NFC', a)] = b
def rel(fn):
    """The release of a sample: its row of `--releases`, else the year of a `__pf<year>` name (as saved by that release)."""
    s = unicodedata.normalize('NFC', sample(fn))
    if s in release:
        return release[s]
    m = re.search(r'__pf(\d{4})(?:-[^_]*)?$', s)
    return m.group(1) if m else ''
def is_schedule(set_name):
    """Not a parameter group: a schedule exported as a property set (`Door Schedule`), or a set whose name holds no
    Latin letter (a family's own group in its author's language: the exports run Revit in English)."""
    return 'schedule' in set_name.lower() or not re.search('[A-Za-z]', set_name)
def newest(by_release):
    """The votes of the newest release that has any."""
    return by_release[max(by_release)]
props = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))  # name -> release -> (set, type)
for fn in glob.glob(os.path.join(pairs, '*.props.tsv')):
    r = rel(fn)
    for l in open(fn, encoding='utf-8', errors='replace'):
        if l[0] == '#':
            continue
        s, n, t, c = l.rstrip('\n').split('\t')
        if not is_schedule(s):
            props[n][r][(s, t)] += int(c)
cprops = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))  # (category, name) -> release -> (set, type)
for fn in glob.glob(os.path.join(pairs, '*.cprops.tsv')):
    r = rel(fn)
    for l in open(fn, encoding='utf-8', errors='replace'):
        if l[0] == '#':
            continue
        cat, s, n, t, c = l.rstrip('\n').split('\t')
        if not is_schedule(s):
            cprops[(cat, n)][r][(s, t)] += int(c)


def pick(c):
    """(most voted set, most voted value type) of a Counter of ((set, type), n)."""
    sets, types = collections.Counter(), collections.Counter()
    for (s, t), k in c.items():
        sets[s] += k
        types[t] += k
    return sets.most_common(1)[0][0], types.most_common(1)[0][0]
label_by = {}  # id -> release -> votes, for a label that names one id
for n, by in props.items():
    ids = labels.get(n, ())
    if len(ids) == 1:
        label_by[next(iter(ids))] = by
file_by = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))  # id -> release -> (set, type)
enum = collections.defaultdict(lambda: collections.defaultdict(lambda: collections.defaultdict(collections.Counter)))  # id -> raw -> release -> text
enum_models = collections.defaultdict(set)  # (id, raw, text) -> samples that wrote it
for fn in glob.glob(os.path.join(pairs, '*.pairs.tsv')):
    for l in open(fn, encoding='utf-8', errors='replace'):
        if l[0] == '#':
            continue
        p = l.rstrip('\n').split('\t')
        if len(p) < 10 or 'L' not in p[9] or is_schedule(p[1]):
            continue
        pid = int(p[5])
        file_by[pid][rel(fn)][(p[1], p[3])] += 1
        if p[7] == 'Integer' and p[3] in ('IFCLABEL', 'IFCTEXT', 'IFCIDENTIFIER'):
            enum[pid][p[8]][rel(fn)][p[4]] += 1
            enum_models[(pid, p[8], p[4])].add(sample(fn))
res = {}
for i in set(label_by) | set(file_by):
    # the newer release wins, route 2 (the file's own parameter) where the release is the same
    cands = [(max(by), k, name, by) for k, name, by in ((2, 'file+label', file_by.get(i)), (1, 'label', label_by.get(i))) if by]
    r, k, name, by = max(cands, key=lambda c: (c[0], c[1]))
    res[i] = (*pick(by[r]), name)
def category_ids(cat, ids):
    """The ids of a shared label whose enum name starts with the singular of the category (`DOOR_`)."""
    base = ''.join(ch if ch.isalnum() else '_' for ch in cat.upper()).strip('_')
    stems = {base, base[:-1] if base.endswith('S') else base, base[:-2] if base.endswith('ES') else base}
    return {i for i in ids if any(e.startswith(st + '_') for e in enums[i] for st in stems)}
cat_by = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))  # id -> release -> (set, type), over every category key
for (cat, n), by in cprops.items():
    ids = category_ids(cat, labels.get(n, ()))
    if len(labels.get(n, ())) > 1 and len(ids) == 1:
        for r, c in by.items():
            cat_by[next(iter(ids))][r].update(c)
by_category = 0
for i, by in sorted(cat_by.items()):
    if i not in res:
        res[i] = (*pick(newest(by)), 'category+label')
        by_category += 1
print(by_category, 'ids from a category and a label')
# An option text never embeds a model's own element name (a level, a type: `Up to level: Roof`):
# it is dropped when the part after its last `: ` is a storey or type name of the sample that wrote it.
own = {}
for fn in glob.glob(os.path.join(pairs, '*.names.tsv')):
    own[sample(fn)] = {tuple(l.rstrip('\n').split('\t', 1)) for l in open(fn, encoding='utf-8') if l[0] != '#'}
def embeds_name(text, models):
    if ': ' not in text:
        return False
    tail = text.rsplit(': ', 1)[1]
    return any(name == tail for m in models for _, name in own.get(m, ()))
dropped = []
rows = []
for i in sorted(enum):
    for raw in sorted(enum[i], key=lambda x: int(x) if x.lstrip('-').isdigit() else 0):
        text = newest(enum[i][raw]).most_common(1)[0][0]
        models = enum_models[(i, raw, text)]
        if embeds_name(text, models):
            dropped.append((i, raw, text))
            continue
        rows.append((i, raw, text))
tmp_gs, tmp_en = out + '.group-spec.tsv.tmp', out + '.enum-texts.tsv.tmp'
with open(tmp_gs, 'w') as f:
    f.write('# id\tgroup\tifc_type\troute\n')
    for i in sorted(res):
        f.write(f"{i}\t{res[i][0]}\t{res[i][1]}\t{res[i][2]}\n")
with open(tmp_en, 'w') as f:
    f.write('# id\tvalue\ttext\n')
    for i, raw, text in rows:
        f.write(f"{i}\t{raw}\t{text}\n")
os.replace(tmp_gs, out + '.group-spec.tsv')
os.replace(tmp_en, out + '.enum-texts.tsv')
print(len(rows), 'enumeration texts kept,', len(dropped), 'dropped for embedding a model element name:', dropped)
