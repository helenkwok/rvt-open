#!/usr/bin/env python3
"""Group, spec kind and enumeration texts of built-in parameters from Revit's own IFC export.

usage: derive_ifc.py PAIRS_DIR BIP_ENUM.tsv OUT_PREFIX

PAIRS_DIR holds, per exported sample, `<m>.props.tsv` (set, property, IFC value type, count of Revit's own
property sets) and `<m>.pairs.tsv` (an element's IFC property joined with a parameter id the file
stores on that element, by the property name equal to the parameter's Revit 2027 API reference label).
Route 1 (label): a property name equal to the reference label of exactly one id gives that id its
property set (group) and IFC value type. Route 2 (file and label): where the label is shared by
several ids, the element's own stored parameter picks the id. Route 2 wins where both give one.
Enumeration texts: an integer parameter whose IFC value is a text gives (id, stored integer) -> text.
Writes OUT_PREFIX.group-spec.tsv (id, group, ifc_type, route) and OUT_PREFIX.enum-texts.tsv."""
import argparse, collections, glob, os
ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
ap.add_argument('pairs_dir')
ap.add_argument('bip_enum')
ap.add_argument('out_prefix')
args = ap.parse_args()
pairs, bip, out = args.pairs_dir, args.bip_enum, args.out_prefix
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
for pid, sv in file_votes.items():
    sets = collections.Counter()
    types = collections.Counter()
    for s, c in sv.items():
        sets[s] += sum(c.values())
        types.update(c)
    res[pid] = (sets.most_common(1)[0][0], types.most_common(1)[0][0], 'file+label')
with open(out + '.group-spec.tsv', 'w') as f:
    f.write('# id\tgroup\tifc_type\troute\n')
    for i in sorted(res):
        f.write(f"{i}\t{res[i][0]}\t{res[i][1]}\t{res[i][2]}\n")
with open(out + '.enum-texts.tsv', 'w') as f:
    f.write('# id\tvalue\ttext\n')
    for i in sorted(enum):
        for raw in sorted(enum[i], key=lambda x: int(x) if x.lstrip('-').isdigit() else 0):
            f.write(f"{i}\t{raw}\t{enum[i][raw].most_common(1)[0][0]}\n")
