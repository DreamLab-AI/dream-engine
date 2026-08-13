#!/usr/bin/env node
/** The real executable: wires `run` to the process + node fs. */
import { readFile, writeFile } from 'node:fs/promises';
import { run, type IO } from './index.js';

const io: IO = {
  readFile: (p) => readFile(p, 'utf8'),
  writeFile: (p, c) => writeFile(p, c, 'utf8'),
  now: () => new Date().toISOString().slice(0, 10),
  env: process.env,
};

run(process.argv.slice(2), io).then((r) => {
  if (r.out) process.stdout.write(r.out);
  if (r.err) process.stderr.write(r.err);
  process.exit(r.code);
});
