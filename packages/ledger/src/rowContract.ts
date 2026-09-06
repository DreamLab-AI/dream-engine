// Ledger row contract validator — dream-cycle 2026-09-06, deep: ledger-signals.
// Enforces the machine-readable invariants of docs/dream-cycle/LEDGER.md rows:
// finding is concrete and self-contained (<= 80 chars, no pointers, no
// frozen-hypothesis leakage); prior-night fates are token-only (#N:FATE with
// FATE in MERGED|CLOSED|OPEN|STALE); ACCEPT rows track a PR and carry a witness.
// Rows dated before `enforceFrom` are grandfathered.

export interface RowViolation {
  date: string;
  rule: string;
  detail: string;
}

const FATE_TOKEN = /^#\d+:(MERGED|CLOSED|OPEN|STALE)$/;

/** Parse a markdown table line into its 10 ledger cells, or null if not a row. */
export function parseRow(line: string): string[] | null {
  const trimmed = line.trim();
  if (!trimmed.startsWith("|")) return null;
  const cells = trimmed.split("|").map((cell) => cell.trim());
  if (cells.length < 12) return null;
  return cells.slice(1, 11);
}

/** Validate one parsed row (10 cells). Rows dated before enforceFrom are skipped. */
export function validateRow(cells: string[], enforceFrom: string): RowViolation[] {
  const violations: RowViolation[] = [];
  const at = (index: number): string => (cells[index] ?? "");
  const date = at(0);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return violations;
  if (date < enforceFrom) return violations;

  const flag = (rule: string, detail: string): void => {
    violations.push({ date, rule, detail });
  };

  const finding = at(2);
  if (finding.length === 0) flag("finding-empty", "finding column is empty");
  if (finding.length > 80) flag("finding-too-long", `${finding.length} chars > 80`);
  if (/\bsee\s+(report|gist)\b/i.test(finding) || /^(see|gist)\b/i.test(finding)) {
    flag("finding-pointer", "finding points elsewhere instead of stating the result");
  }
  if (/^given\b/i.test(finding)) {
    flag("finding-hypothesis-leak", "finding column contains frozen-hypothesis text");
  }

  const verdict = at(6);
  if (!/^(ACCEPT|REJECT|INCONCLUSIVE)$/.test(verdict)) {
    flag("verdict-vocab", `unrecognised verdict "${verdict}"`);
  }
  if (verdict === "ACCEPT" && (at(4) === "NONE" || at(4) === "")) {
    flag("accept-without-pr", "ACCEPT rows must record PR as pending or #N");
  }
  if (verdict === "ACCEPT" && at(8) === "") {
    flag("accept-without-witness", "ACCEPT rows must carry a witness");
  }

  const fates = at(9);
  if (fates !== "" && !fates.split(/\s+/).every((token) => FATE_TOKEN.test(token))) {
    flag("fates-grammar", "prior-night fates must be space-separated #N:FATE tokens only");
  }

  return violations;
}

/** Validate a whole ledger file body. */
export function validateLedger(text: string, enforceFrom: string): RowViolation[] {
  const violations: RowViolation[] = [];
  for (const line of text.split(/\r?\n/)) {
    const cells = parseRow(line);
    if (cells !== null) violations.push(...validateRow(cells, enforceFrom));
  }
  return violations;
}
