// The WebAssembly build, run in Node the way js/app.mjs runs it in a page:
// the desktop app gets a dropped picture and Ctrl+Enter, one frame at a time,
// until its drawing settles, which must be the command line's own drawing
// (kit/web/tests/expected). The browser's floating point is not the native
// one, so this is the check that the page draws what the app draws.
//   node kit/web/js/test.mjs [path/to/vectormagik_web.wasm]

import { readFileSync } from 'node:fs';
import { basename, dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { asSaved, drawInTab, openTab } from './tab.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '../../..');
const wasm = readFileSync(
  process.argv[2] ?? join(root, 'work/rust-target/wasm32-unknown-unknown/release/vectormagik_web.wasm'),
);

let failed = 0;
for (const [sample, name] of [
  ['kit/fixtures/samples/logo-with-blending-small.png', 'logo-with-blending-small'],
  ['kit/fixtures/samples/logo-without-blending.png', 'logo-without-blending'],
  ['kit/fixtures/photos/chelsea.png', 'chelsea'],
]) {
  const started = performance.now();
  const { svg, status } = await drawInTab(wasm, basename(sample), readFileSync(join(root, sample)));
  const seconds = ((performance.now() - started) / 1000).toFixed(2);
  const same = svg === asSaved(readFileSync(join(root, `kit/web/tests/expected/${name}.svg`), 'utf8'));
  if (!same) failed++;
  console.log(`${same ? 'same' : 'DIFFERENT'}  ${name}  ${seconds} s  ${status}`);
}
const tab = await openTab(wasm);
tab.drop('notes.txt', new Uint8Array([1, 2, 3]));
const refused = tab.status();
if (!/picture|format/i.test(refused)) {
  failed++;
  console.log(`a bad file was not refused: ${refused}`);
} else {
  console.log(`bad file refused: ${refused}`);
}
process.exit(failed ? 1 : 0);
