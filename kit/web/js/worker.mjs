// The Web Worker the page talks to, so a trace never freezes the page.
// Messages in: {id, type: 'convert', file: ArrayBuffer, settings},
// {id, type: 'derive', settings}, {id, type: 'output', kind}.
// Messages out: {id, ok: true, stats, svg} (convert, derive),
// {id, ok: true, bytes} (output), or {id, ok: false, error}.

import { loadEngine, OUTPUT } from './engine.mjs';

const ready = fetch(new URL('./vectormagik_web.wasm', import.meta.url))
  .then((response) => response.arrayBuffer())
  .then(loadEngine);

self.onmessage = async ({ data }) => {
  const { id, type } = data;
  try {
    const engine = await ready;
    if (type === 'convert' || type === 'derive') {
      const stats = type === 'convert' ? engine.convert(data.file, data.settings) : engine.derive(data.settings);
      self.postMessage({ id, ok: true, stats, svg: engine.svg() });
    } else if (type === 'output') {
      const bytes = engine.output(data.kind ?? OUTPUT.svg);
      self.postMessage({ id, ok: true, bytes }, [bytes.buffer]);
    } else {
      throw new Error(`Unknown request: ${type}`);
    }
  } catch (error) {
    self.postMessage({ id, ok: false, error: String(error?.message ?? error) });
  }
};
