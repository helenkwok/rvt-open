#!/usr/bin/env python3
"""Names of built-in parameters in the 12 other languages of Revit's help.

usage: derive_help.py HELP_DIR NAMES.tsv OUT.tsv

HELP_DIR/<LANG>/<component>/<topic>.htm: Revit 2027 help topics (help.autodesk.com/cloudhelp/2027/<LANG>/
<component>/files/<topic>.htm) in ENU and the 12 languages CHT CHS CSY DEU ESP FRA ITA JPN KOR PLK PTB
RUS; the same topic path in every language. NAMES.tsv: id, English name (the API reference label).
A name found as an exact structured term (table cell, list item, `uicontrol`, bold text, `dt`) of an
English topic takes, in each language, the term at the same position of the same topic where that topic
has as many such terms in that language. Only a name found at the same place in every topic that
carries it in all 12 languages, with the same term on all of them, is written, the first such topic
cited per language (sorted).
Terms only: no help text is kept.
OUT.tsv: id, language, term, topic (`<component>/<topic>`)."""
import collections, html, os, re, sys
helpdir, names, out = sys.argv[1:4]
LANGS = 'CHT CHS CSY DEU ESP FRA ITA JPN KOR PLK PTB RUS'.split()
PAT = {k: re.compile(v, re.S) for k, v in {
    'td': r'<td\b[^>]*>(.*?)</td>', 'li': r'<li\b[^>]*>(.*?)</li>',
    'ui': r'<span class="uicontrol"[^>]*>(.*?)</span>',
    'b': r'<(?:b|strong)\b[^>]*>(.*?)</(?:b|strong)>', 'dt': r'<dt\b[^>]*>(.*?)</dt>'}.items()}
clean = lambda s: re.sub(r'\s+', ' ', html.unescape(re.sub(r'<[^>]+>', ' ', s))).strip()
wanted = collections.defaultdict(set)
for l in open(names, encoding='utf-8'):
    if l[0] != '#' and l.strip():
        i, n = l.rstrip('\n').split('\t')[:2]
        if n:
            wanted[n].add(int(i))


def lists(lang, comp, topic):
    p = os.path.join(helpdir, lang, comp, topic + '.htm')
    if not os.path.exists(p) or os.path.getsize(p) == 0:
        return None
    t = open(p, encoding='utf-8', errors='replace').read()
    return {k: [clean(m.group(1)) for m in rx.finditer(t)] for k, rx in PAT.items()}


terms = collections.defaultdict(lambda: collections.defaultdict(dict))  # name -> lang -> term -> first topic
carried = collections.Counter()  # name -> English occurrences
aligned = collections.defaultdict(collections.Counter)  # name -> lang -> occurrences with a term at the same place
for comp in sorted(os.listdir(os.path.join(helpdir, 'ENU'))):
    for fn in sorted(os.listdir(os.path.join(helpdir, 'ENU', comp))):
        topic = fn[:-4]
        en = lists('ENU', comp, topic)
        if not en:
            continue
        hits = [(k, i, x) for k, items in en.items() for i, x in enumerate(items) if x in wanted and len(x) <= 60]
        if not hits:
            continue
        loc = {l: lists(l, comp, topic) for l in LANGS}
        for k, i, x in hits:
            carried[x] += 1
            for l, d in loc.items():
                if d and len(d[k]) == len(en[k]) and d[k][i] and len(d[k][i]) <= 60:
                    terms[x][l].setdefault(d[k][i], f'{comp}/{topic}')
                    aligned[x][l] += 1
n = 0
with open(out, 'w', encoding='utf-8') as f:
    f.write('# id\tlanguage\tterm\ttopic\n')
    for name, ids in sorted(wanted.items()):
        t = terms.get(name)
        if not t or any(len(t.get(l, {})) != 1 or aligned[name][l] != carried[name] for l in LANGS):
            continue
        for i in sorted(ids):
            n += 1
            for l in LANGS:
                term, topic = next(iter(t[l].items()))
                f.write(f'{i}\t{l}\t{term}\t{topic}\n')
print(n, 'ids')
