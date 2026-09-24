// VectorMagik's engine in JavaScript's hands: loads vectormagik_web.wasm and
// wraps its plain exports (see kit/web/src/lib.rs). Works in a Web Worker,
// in a page and in Node (the test in this folder).

const encoder = new TextEncoder();
const decoder = new TextDecoder();

export const OUTPUT = { stats: 0, svg: 1, pdf: 2, eps: 3 };

export async function loadEngine(wasm) {
  const { instance } = await WebAssembly.instantiate(wasm, {});
  const e = instance.exports;

  const put = (bytes) => {
    const ptr = e.vm_alloc(bytes.length);
    new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
  };
  // A fresh view every time: the module's memory grows and detaches old ones.
  const take = () => new Uint8Array(e.memory.buffer, e.vm_out_ptr(), e.vm_out_len()).slice();
  const settle = (code) => {
    const out = take();
    if (code !== 0) throw new Error(decoder.decode(out));
    return out;
  };
  const withBytes = (bytes, run) => {
    const ptr = put(bytes);
    try {
      return run(ptr, bytes.length);
    } finally {
      e.vm_dealloc(ptr, bytes.length);
    }
  };

  return {
    /** Trace an image file's bytes; returns the statistics. */
    convert(file, settings = '') {
      const image = file instanceof Uint8Array ? file : new Uint8Array(file);
      return withBytes(image, (ip, il) =>
        withBytes(encoder.encode(settings), (sp, sl) => JSON.parse(decoder.decode(settle(e.vm_convert(ip, il, sp, sl))))),
      );
    },
    /** Draw the last trace again with other settings; returns the statistics. */
    derive(settings = '') {
      return withBytes(encoder.encode(settings), (sp, sl) => JSON.parse(decoder.decode(settle(e.vm_derive(sp, sl)))));
    },
    /** The last drawing as bytes: OUTPUT.svg, .pdf or .eps. */
    output(kind) {
      return settle(e.vm_output(kind));
    },
    svg() {
      return decoder.decode(settle(e.vm_output(OUTPUT.svg)));
    },
  };
}
