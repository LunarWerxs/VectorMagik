// The WebAssembly build, run in Node, against the command line's own
// drawings (kit/web/tests/expected): the browser's floating point is not the
// native one, so this is the check that the page draws what the app draws.
//   node kit/web/js/test.mjs [path/to/vectormagik_web.wasm]

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadEngine, OUTPUT } from './engine.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '../../..');
const wasmPath = process.argv[2] ?? join(root, 'work/rust-target/wasm32-unknown-unknown/release/vectormagik_web.wasm');
const engine = await loadEngine(readFileSync(wasmPath));

let failed = 0;
for (const [sample, name] of [
  ['kit/fixtures/samples/logo-with-blending-small.png', 'logo-with-blending-small'],
  ['kit/fixtures/samples/logo-without-blending.png', 'logo-without-blending'],
  ['kit/fixtures/photos/chelsea.png', 'chelsea'],
]) {
  const started = performance.now();
  const stats = engine.convert(readFileSync(join(root, sample)), '');
  const seconds = ((performance.now() - started) / 1000).toFixed(2);
  const same = engine.svg() === readFileSync(join(root, `kit/web/tests/expected/${name}.svg`), 'utf8');
  if (!same) failed++;
  console.log(`${same ? 'same' : 'DIFFERENT'}  ${name}  ${seconds} s  ${JSON.stringify(stats)}`);
}
const simplified = engine.derive('simplify=3');
console.log(`derive simplify=3: ${simplified.nodes} nodes`);
const pdf = engine.output(OUTPUT.pdf);
console.log(`pdf ${pdf.length} bytes, starts ${new TextDecoder().decode(pdf.slice(0, 5))}`);
try {
  engine.convert(new Uint8Array([1, 2, 3]), '');
  failed++;
  console.log('a bad file was accepted');
} catch (error) {
  console.log(`bad file refused: ${error.message}`);
}
process.exit(failed ? 1 : 0);
