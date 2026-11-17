#!/usr/bin/env python3
"""Group, spec kind and enumeration texts of built-in parameters from Revit's own IFC export.

usage: derive_ifc.py PAIRS_DIR BIP_ENUM.tsv OUT_PREFIX

PAIRS_DIR holds, per exported sample, `<m>.props.tsv` (set, property, IFC value type, count of Revit's own
property sets) and `<m>.pairs.tsv` (an element's IFC property joined with a parameter id the file
stores on that element, by the property name equal to the parameter's Revit 2027 API reference label).
Route 1 (label): a property name equal to the reference label of exactly one id gives that id its
property set (group) and IFC value type. Route 2 (file and label): where the label is shared by
several ids, the element's own stored parameter picks the id. Route 2 wins where both give one.
Enumeration texts: an integer parameter whose IFC value is a text gives (id, stored integer) -> text,
dropped when the part after its last `: ` is a storey or type name of the sample that wrote it
(`*.names.tsv`).
Writes OUT_PREFIX.group-spec.tsv (id, group, ifc_type, route) and OUT_PREFIX.enum-texts.tsv."""
import argparse, collections, glob, os, sys
ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('pairs_dir')
ap.add_argument('bip_enum')
ap.add_argument('out_prefix')
args = ap.parse_args()
pairs, bip, out = args.pairs_dir, args.bip_enum, args.out_prefix
if not os.path.isdir(pairs):
    sys.exit(f'not a directory: {pairs}')
if not glob.glob(os.path.join(pairs, '*.props.tsv')) or not glob.glob(os.path.join(pairs, '*.pairs.tsv')):
    sys.exit(f'no *.props.tsv and *.pairs.tsv samples in {pairs}')
if not os.path.isfile(bip):
    sys.exit(f'not a file: {bip}')
labels = collections.defaultdict(set)
for l in open(bip):
    if l[0] != '#':
        p = l.rstrip('\n').split('\t')
        if len(p) > 2 and p[2]:
            labels[p[2]].add(int(p[1]))
props = collections.defaultdict(collections.Counter)
for fn in glob.glob(os.path.join(pairs, '*.props.tsv')):
    for l in open(fn, encoding='utf-8', errors='replace'):
        if l[0] == '#':
            continue
        s, n, t, c = l.rstrip('\n').split('\t')
        props[n][(s, t)] += int(c)
res = {}
for n, c in props.items():
    ids = labels.get(n, ())
    if len(ids) != 1:
        continue
    sets = collections.Counter()
    for (s, t), k in c.items():
        sets[s] += k
    t = collections.Counter()
    for (s, ty), k in c.items():
        t[ty] += k
    res[next(iter(ids))] = (sets.most_common(1)[0][0], t.most_common(1)[0][0], 'label')
file_votes = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))
enum = collections.defaultdict(lambda: collections.defaultdict(collections.Counter))
enum_models = collections.defaultdict(set)  # (id, raw, text) -> samples that wrote it
for fn in glob.glob(os.path.join(pairs, '*.pairs.tsv')):
    for l in open(fn, encoding='utf-8', errors='replace'):
        if l[0] == '#':
            continue
        p = l.rstrip('\n').split('\t')
        if len(p) < 10 or 'L' not in p[9]:
            continue
        pid = int(p[5])
        file_votes[pid][p[1]][p[3]] += 1
        if p[7] == 'Integer' and p[3] in ('IFCLABEL', 'IFCTEXT', 'IFCIDENTIFIER'):
            enum[pid][p[8]][p[4]] += 1
            enum_models[(pid, p[8], p[4])].add(os.path.basename(fn))
for pid, sv in file_votes.items():
    sets = collections.Counter()
    types = collections.Counter()
    for s, c in sv.items():
        sets[s] += sum(c.values())
        types.update(c)
    res[pid] = (sets.most_common(1)[0][0], types.most_common(1)[0][0], 'file+label')
# An option text never embeds a model's own element name (a level, a type: `Up to level: Roof`):
# it is dropped when the part after its last `: ` is a storey or type name of the sample that wrote it.
own = {}
for fn in glob.glob(os.path.join(pairs, '*.names.tsv')):
    own[os.path.basename(fn).split('.')[0]] = {tuple(l.rstrip('\n').split('\t', 1)) for l in open(fn, encoding='utf-8') if l[0] != '#'}
def embeds_name(text, models):
    if ': ' not in text:
        return False
    tail = text.rsplit(': ', 1)[1]
    return any(name == tail for m in models for _, name in own.get(m, ()))
dropped = []
rows = []
for i in sorted(enum):
    for raw in sorted(enum[i], key=lambda x: int(x) if x.lstrip('-').isdigit() else 0):
        text = enum[i][raw].most_common(1)[0][0]
        models = {m.split('.')[0] for m in enum_models[(i, raw, text)]}
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
