#!/usr/bin/env python3
"""Independent Python re-implementation of the reference TypeScript ledger
functions (parseLedger, verifyLedger, validateLedger, learningSignals,
verdictStats, parsePriorFates), derived by reading `index.ts` and
`rowContract.ts`. It exists only to produce the EXPECTED values asserted by
`crates/dream-core/tests/ts_parity.rs`; it never runs the TypeScript.

Run:  python3 -I derive_expected.py fork-LEDGER.md
JS semantics are reproduced deliberately: UTF-16 length, ASCII \\b/\\w,
split(/\\s+/) keeping a leading empty token.
"""
import json, re, sys

A = re.ASCII
COLUMNS = 10
NIGHT = ["ACCEPT", "REJECT", "INCONCLUSIVE"]
NON_NIGHT = ["BLOCKED-ENV", "HANDOFF", "OPERATOR"]
VERDICTS = NIGHT + NON_NIGHT
EVALS = ["yes", "no", "blocked", "n/a"]
FATE_STRICT = re.compile(r"^#\d+:(MERGED|CLOSED|OPEN|STALE)$", A)
FATE_LENIENT = re.compile(r"#(\d+)\s*:\s*(MERGED|CLOSED|OPEN|STALE)\b", A | re.I)


def js_trim(s):
    return s.strip(" \t\n\r\x0b\x0c ﻿  ")


def split_row(line):  # index.ts splitRow
    t = js_trim(line)
    t = re.sub(r"^\|", "", t)
    t = re.sub(r"\|$", "", t)
    t = t.replace("\\|", "\0")
    return [js_trim(c).replace("\0", "|") for c in t.split("|")]


def is_divider(cells):
    return len(cells) > 0 and all(re.fullmatch(r":?-{3,}:?", re.sub(r"\s", "", c)) for c in cells)


def parse_ledger(md):
    rows, warnings, seen = [], [], False
    for line in re.split(r"\r?\n", md):
        if not js_trim(line) or "|" not in line:
            continue
        cells = split_row(line)
        if is_divider(cells):
            continue
        if not seen:
            seen = True
            if cells[0].lower() != "date":
                warnings.append('first table row is not the expected header (got "%s")' % cells[0])
            continue
        if len(cells) != COLUMNS:
            warnings.append("row has %d columns, expected %d" % (len(cells), COLUMNS))
        g = lambda i: cells[i] if i < len(cells) else ""
        rows.append(dict(date=g(0), deep=g(1), finding=g(2), issue=g(3), pr=g(4), evaluated=g(5),
                         verdict=g(6), effect=g(7), witness=g(8), priorFates=g(9)))
    return rows, warnings


def verify_ledger(md):
    errors = []
    rows, warnings = parse_ledger(md)
    if not re.search(r"\|\s*Date\s*\|", md, A):
        errors.append("ledger is missing the Date header row")
    for i, r in enumerate(rows):
        if r["verdict"] and r["verdict"] not in VERDICTS:
            errors.append('row %d: verdict "%s"' % (i + 1, r["verdict"]))
        if r["evaluated"] and r["evaluated"] not in EVALS:
            errors.append('row %d: evaluated "%s"' % (i + 1, r["evaluated"]))
        if r["date"] and not re.fullmatch(r"\d{4}-\d{2}-\d{2}", r["date"], A):
            errors.append('row %d: date "%s"' % (i + 1, r["date"]))
    return dict(ok=not errors, errors=errors, warnings=warnings, rowCount=len(rows))


def parse_row(line):  # rowContract.ts parseRow
    t = js_trim(line)
    if not t.startswith("|"):
        return None
    cells = [js_trim(c).replace("\0", "|") for c in t.replace("\\|", "\0").split("|")]
    if len(cells) < 12:
        return None
    return cells[1:11]


def u16(s):
    return len(s.encode("utf-16-le")) // 2


def validate_row(cells, enforce_from):
    out = []
    at = lambda i: cells[i] if i < len(cells) else ""
    date = at(0)
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date, A) or date < enforce_from:
        return out
    flag = lambda rule: out.append((date, rule))
    f = at(2)
    if u16(f) == 0:
        flag("finding-empty")
    if u16(f) > 80:
        flag("finding-too-long")
    if re.search(r"\bsee\s+(report|gist)\b", f, A | re.I) or re.search(r"^(see|gist)\b", f, A | re.I):
        flag("finding-pointer")
    if re.search(r"^given\b", f, A | re.I):
        flag("finding-hypothesis-leak")
    v = at(6)
    if v not in VERDICTS:
        flag("verdict-vocab")
    if v == "ACCEPT" and at(4) in ("NONE", ""):
        flag("accept-without-pr")
    if v == "ACCEPT" and at(8) == "":
        flag("accept-without-witness")
    fates = at(9)
    if fates != "" and not all(FATE_STRICT.match(t) for t in re.split(r"\s+", fates, flags=A)):
        flag("fates-grammar")
    return out


def validate_ledger(md, enforce_from):
    out = []
    for line in re.split(r"\r?\n", md):
        cells = parse_row(line)
        if cells is not None:
            out.extend(validate_row(cells, enforce_from))
    return out


def prior_fates(rows):
    fates = {}
    for r in rows:
        for m in FATE_LENIENT.finditer(r["priorFates"]):
            fates[m.group(1)] = m.group(2).upper()
    return fates


def learning_signals(rows, window=14):
    recent = rows[-window:] if window else []
    merged = {n for n, f in prior_fates(rows).items() if f == "MERGED"}
    prs = []
    for r in recent:
        m = re.search(r"#?(\d+)", r["pr"], A)
        if m:
            prs.append(m.group(1))
    zero = len(prs) > 0 and all(n not in merged for n in prs)
    counts = {}
    for r in rows:
        k = re.sub(r"[^a-z0-9 ]", "", r["finding"].lower())
        parts = re.split(r"\s+", k, flags=A)  # python keeps leading/trailing empties like JS
        k = js_trim(" ".join(parts[:6]))
        if not k:
            continue
        counts[k] = counts.get(k, 0) + 1
    dups = [k for k, c in counts.items() if c >= 3]
    last3 = [r["evaluated"] for r in recent[-3:]]
    blocked = len(recent) >= 3 and len(last3) == 3 and all(e == "blocked" for e in last3)
    return dict(zeroMergeStreak=zero, duplicateDirections=dups, lowScoreStreak=False,
                blockedEvalStreak=blocked, nightsConsidered=len(recent))


def verdict_stats(rows):
    s = dict(ACCEPT=0, REJECT=0, INCONCLUSIVE=0, other=0)
    for r in rows:
        if r["verdict"] in s:
            s[r["verdict"]] += 1
        else:
            s["other"] += 1
    return s


if __name__ == "__main__":
    md = open(sys.argv[1], encoding="utf-8").read()
    rows, warnings = parse_ledger(md)
    print(json.dumps(dict(
        parse=dict(rowCount=len(rows), warnings=warnings),
        verify=verify_ledger(md),
        validate=[list(v) for v in validate_ledger(md, "2026-09-06")],
        validateAll=[list(v) for v in validate_ledger(md, "0000-00-00")],
        signals=learning_signals(rows),
        stats=verdict_stats(rows),
        fates=prior_fates(rows),
    ), indent=1))
