// The browser build driven from Node the way js/app.mjs drives it in a page,
// without the painting: frames of input in, the app's status and drawing out
// (the protocol is the doc comment of kit/web/src/app.rs). For the checks
// that the app in a tab draws what the command line draws.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

const withLength = (bytes) => {
  const b = new Uint8Array(4 + bytes.length);
  new DataView(b.buffer).setUint32(0, bytes.length, true);
  b.set(bytes, 4);
  return b;
};
const concat = (...parts) => {
  const b = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    b.set(p, at);
    at += p.length;
  }
  return b;
};
const text = (s) => withLength(encoder.encode(s));
// Ctrl+Enter, Convert: ctrl (2) and command (16), as on Windows.
const CONVERT = 18;

// A fresh app in a 1280 x 860 point tab, from the module's bytes.
export async function openTab(wasm) {
  const { instance } = await WebAssembly.instantiate(wasm, {
    env: { vm_now_ms: () => performance.now() },
  });
  const e = instance.exports;
  const out = () => new Uint8Array(e.memory.buffer, e.vm_out_ptr(), e.vm_out_len()).slice();
  e.vm_app_start(0, 0);
  const frame = (events = [], modifiers = 0) => {
    const size = 30 + events.reduce((n, ev) => n + ev.length, 0);
    const ptr = e.vm_alloc(size);
    const view = new DataView(e.memory.buffer, ptr, size);
    view.setFloat64(0, performance.now() / 1000, true);
    view.setFloat32(8, 1280, true);
    view.setFloat32(12, 860, true);
    view.setFloat32(16, 1, true);
    view.setUint32(20, 8192, true);
    view.setUint8(24, 1);
    view.setUint8(25, modifiers);
    view.setUint32(26, events.length, true);
    let at = ptr + 30;
    for (const ev of events) {
      new Uint8Array(e.memory.buffer).set(ev, at);
      at += ev.length;
    }
    const code = e.vm_app_frame(ptr, size);
    e.vm_dealloc(ptr, size);
    if (code !== 0) throw new Error(decoder.decode(out()));
  };
  const status = () => (e.vm_app_status(), decoder.decode(out()));
  const svg = () => (e.vm_app_svg() === 0 ? decoder.decode(out()) : null);
  return {
    frame,
    status,
    svg,
    // A file dropped on the page.
    drop(name, data) {
      frame([concat([11], text(name), withLength(data))]);
    },
    // Ctrl+Enter, then frames until the drawing settles; its SVG, or null.
    convert(frames = 400) {
      frame([concat([5], text('Enter'), text('Enter'), [1, 0])], CONVERT);
      for (let i = 0; i < frames; i++) {
        frame();
        const drawn = svg();
        if (drawn !== null) return drawn;
      }
      return null;
    },
  };
}

// The command line's drawing as the app saves it: the app declares the saved
// size in pixels where the engine's file says points; nothing else differs.
export const asSaved = (cli) => cli.replace(/<svg width="([\d.]+)pt" height="([\d.]+)pt"/, '<svg width="$1" height="$2"');

// The drawing the app in a tab makes of one picture file.
export async function drawInTab(wasm, name, data) {
  const tab = await openTab(wasm);
  tab.drop(name, data);
  return { svg: tab.convert(), status: tab.status() };
}
