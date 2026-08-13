#!/usr/bin/env node
/**
 * termshot — render ANSI text (from the dream-machine TUI) into a crisp,
 * versionable SVG "screenshot". No dependencies; maps the TUI's known ANSI
 * palette to the dream color system. Usage:
 *   node packages/cli/dist/bin.js tui --path LEDGER.md | node scripts/termshot.mjs > docs/media/tui.svg
 */
import { readFileSync } from 'node:fs';

const PALETTE = {
  '36': '#22d3ee', // cyan
  '35': '#e879f9', // magenta
  '32': '#4ade80', // green
  '31': '#f87171', // red
  '33': '#fbbf24', // yellow
  '38;5;99': '#8b5cf6', // violet
  '38;5;245': '#9ca3af', // gray
};
const FG = '#e5e7ff';

function tokenize(line) {
  // Split into styled runs based on SGR codes.
  const runs = [];
  let color = FG;
  let bold = false;
  let dim = false;
  const re = /\x1b\[([0-9;]*)m/g;
  let last = 0;
  let m;
  const push = (text) => {
    if (text) runs.push({ text, color, bold, dim });
  };
  while ((m = re.exec(line))) {
    push(line.slice(last, m.index));
    const code = m[1];
    if (code === '0' || code === '') {
      color = FG;
      bold = false;
      dim = false;
    } else if (code === '1') bold = true;
    else if (code === '2') dim = true;
    else if (PALETTE[code]) color = PALETTE[code];
    last = re.lastIndex;
  }
  push(line.slice(last));
  return runs;
}

const esc = (s) =>
  s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function render(ansi) {
  const rawLines = ansi.replace(/\n$/, '').split('\n');
  const cw = 8.4; // char width
  const lh = 19; // line height
  const padX = 22;
  const padY = 56;
  const maxCols = Math.max(...rawLines.map((l) => l.replace(/\x1b\[[0-9;]*m/g, '').length));
  const width = Math.ceil(padX * 2 + maxCols * cw);
  const height = padY + rawLines.length * lh + 22;

  const textRows = rawLines
    .map((line, i) => {
      const y = padY + i * lh;
      const runs = tokenize(line);
      let x = padX;
      const spans = runs
        .map((r) => {
          const w = r.text.length * cw;
          const span = `<tspan x="${x.toFixed(1)}" y="${y}" fill="${r.color}"${r.bold ? ' font-weight="700"' : ''}${
            r.dim ? ' opacity="0.6"' : ''
          }>${esc(r.text)}</tspan>`;
          x += w;
          return span;
        })
        .join('');
      return spans;
    })
    .join('\n');

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="13.5">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#0b0a1e"/>
      <stop offset="1" stop-color="#160f33"/>
    </linearGradient>
    <radialGradient id="glow" cx="0.2" cy="0" r="1">
      <stop offset="0" stop-color="#8b5cf6" stop-opacity="0.20"/>
      <stop offset="0.5" stop-color="#22d3ee" stop-opacity="0.06"/>
      <stop offset="1" stop-color="#000" stop-opacity="0"/>
    </radialGradient>
  </defs>
  <rect x="0" y="0" width="${width}" height="${height}" rx="14" fill="url(#bg)"/>
  <rect x="0" y="0" width="${width}" height="${height}" rx="14" fill="url(#glow)"/>
  <rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="14" fill="none" stroke="#312e63" stroke-opacity="0.7"/>
  <circle cx="20" cy="22" r="6" fill="#ff5f56"/>
  <circle cx="42" cy="22" r="6" fill="#ffbd2e"/>
  <circle cx="64" cy="22" r="6" fill="#27c93f"/>
  <text x="${width / 2}" y="26" fill="#8b87b8" text-anchor="middle" font-size="12">dream-machine tui</text>
  <text xml:space="preserve">
${textRows}
  </text>
</svg>
`;
}

let input = '';
try {
  input = readFileSync(0, 'utf8');
} catch {
  input = readFileSync(process.argv[2] ?? '/dev/stdin', 'utf8');
}
process.stdout.write(render(input));
