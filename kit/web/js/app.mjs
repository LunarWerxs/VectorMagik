// VectorMagik's desktop app: egui compiled to WebAssembly, running inside a
// <canvas> on a page, painted by a small hand-rolled WebGL2 renderer. The page
// and the module talk through two byte buffers per frame, exactly as described
// by the doc comment at the top of kit/web/src/app.rs -- read that for the
// protocol. No dependencies, no build step; everything lives in gamma space.

const encoder = new TextEncoder();
const decoder = new TextDecoder();

const VERTEX_SRC = `#version 300 es
in vec2 a_pos;
in vec2 a_tc;
in vec4 a_srgba;
uniform vec2 u_screen;
out vec2 v_tc;
out vec4 v_rgba;
void main() {
  gl_Position = vec4(2.0 * a_pos.x / u_screen.x - 1.0, 1.0 - 2.0 * a_pos.y / u_screen.y, 0.0, 1.0);
  v_tc = a_tc;
  v_rgba = a_srgba;
}`;

const FRAGMENT_SRC = `#version 300 es
precision mediump float;
in vec2 v_tc;
in vec4 v_rgba;
uniform sampler2D u_sampler;
out vec4 out_color;
void main() {
  out_color = v_rgba * texture(u_sampler, v_tc);
}`;

// Pictures, Photoshop documents, and the vector files the app asks about
// (trace them from pixels or convert them as they are).
const ACCEPT = 'image/png,image/jpeg,image/gif,image/bmp,image/tiff,.png,.jpg,.jpeg,.gif,.bmp,.tif,.tiff,.tga,.pnm,.pbm,.pgm,.ppm,.pam,'
  + '.psd,.psb,image/vnd.adobe.photoshop,.svg,image/svg+xml,.pdf,application/pdf,.ai,.eps,.ps,application/postscript';
const PREVENT_KEYS = new Set([
  'Tab', 'Backspace', 'Delete', 'Enter', 'Escape', ' ', 'ArrowLeft', 'ArrowRight',
  'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown',
]);

// A tiny growable little-endian byte writer for the input events.
class Bytes {
  constructor() {
    this.buf = new Uint8Array(64);
    this.view = new DataView(this.buf.buffer);
    this.len = 0;
  }
  grow(n) {
    if (this.len + n <= this.buf.length) return;
    let cap = this.buf.length * 2;
    while (cap < this.len + n) cap *= 2;
    const next = new Uint8Array(cap);
    next.set(this.buf.subarray(0, this.len));
    this.buf = next;
    this.view = new DataView(next.buffer);
  }
  u8(v) { this.grow(1); this.view.setUint8(this.len, v); this.len += 1; return this; }
  u32(v) { this.grow(4); this.view.setUint32(this.len, v, true); this.len += 4; return this; }
  f32(v) { this.grow(4); this.view.setFloat32(this.len, v, true); this.len += 4; return this; }
  raw(b) { this.grow(b.length); this.buf.set(b, this.len); this.len += b.length; return this; }
  // The protocol's `bytes` and `str`: a u32 length, then the bytes.
  bytes(b) { return this.u32(b.length).raw(b); }
  str(s) { return this.bytes(encoder.encode(s)); }
  done() { return this.buf.subarray(0, this.len); }
}

// Small encoder functions, one per protocol event tag.
const evMoved = (x, y) => () => new Bytes().u8(1).f32(x).f32(y).done();
const evButton = (x, y, button, pressed) => () => new Bytes().u8(2).f32(x).f32(y).u8(button).u8(pressed).done();
const evGone = () => () => new Bytes().u8(3).done();
const evWheel = (unit, dx, dy) => () => new Bytes().u8(4).u8(unit).f32(dx).f32(dy).done();
const evKey = (key, code, pressed, repeat) => () => new Bytes().u8(5).str(key).str(code).u8(pressed).u8(repeat).done();
const evText = (text) => () => new Bytes().u8(6).str(text).done();
const evSimple = (tag) => () => new Bytes().u8(tag).done();
const evPaste = (text) => () => new Bytes().u8(9).str(text).done();
const evFile = (name, data) => () => new Bytes().u8(11).str(name).bytes(data).done();
const evHover = (on) => () => new Bytes().u8(12).u8(on).done();
const evShot = (w, h, rgba) => () => new Bytes().u8(13).u32(w).u32(h).bytes(rgba).done();
const evReply = (id, status, body) => () => new Bytes().u8(14).u32(id).u32(status).str(body).done();
const evTheme = (dark) => () => new Bytes().u8(15).u8(dark ? 1 : 0).done();

export async function startApp(canvas, { wasmUrl, onError } = {}) {
  const bytes = await (await fetch(wasmUrl)).arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes, {
    // Milliseconds since 1970 on the monotonic clock: a stopwatch and a date.
    env: { vm_now_ms: () => performance.timeOrigin + performance.now() },
  });
  const e = instance.exports;

  const gl = canvas.getContext('webgl2', {
    alpha: false,
    antialias: false,
    depth: false,
    stencil: false,
    premultipliedAlpha: true,
    preserveDrawingBuffer: false,
  });
  if (!gl) {
    if (onError) onError('This browser has no WebGL2.');
    return { stop() {} };
  }

  // --- painter: one program, one VAO and the two buffers reused ------------
  const compile = (type, source) => {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, source);
    gl.compileShader(shader);
    return shader;
  };
  const program = gl.createProgram();
  gl.attachShader(program, compile(gl.VERTEX_SHADER, VERTEX_SRC));
  gl.attachShader(program, compile(gl.FRAGMENT_SHADER, FRAGMENT_SRC));
  gl.linkProgram(program);
  const aPos = gl.getAttribLocation(program, 'a_pos');
  const aTc = gl.getAttribLocation(program, 'a_tc');
  const aSrgba = gl.getAttribLocation(program, 'a_srgba');
  const uScreen = gl.getUniformLocation(program, 'u_screen');
  const uSampler = gl.getUniformLocation(program, 'u_sampler');

  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);
  const vbo = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
  const ibo = gl.createBuffer();
  gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ibo);
  gl.enableVertexAttribArray(aPos);
  gl.vertexAttribPointer(aPos, 2, gl.FLOAT, false, 20, 0);
  gl.enableVertexAttribArray(aTc);
  gl.vertexAttribPointer(aTc, 2, gl.FLOAT, false, 20, 8);
  gl.enableVertexAttribArray(aSrgba);
  gl.vertexAttribPointer(aSrgba, 4, gl.UNSIGNED_BYTE, true, 20, 16);

  gl.useProgram(program);
  gl.uniform1i(uSampler, 0);
  gl.activeTexture(gl.TEXTURE0);
  gl.disable(gl.DEPTH_TEST);
  gl.disable(gl.CULL_FACE);
  gl.enable(gl.SCISSOR_TEST);
  gl.enable(gl.BLEND);
  gl.blendEquation(gl.FUNC_ADD);
  gl.blendFuncSeparate(gl.ONE, gl.ONE_MINUS_SRC_ALPHA, gl.ONE_MINUS_DST_ALPHA, gl.ONE);
  gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);

  const textures = new Map();
  const maxTextureSize = gl.getParameter(gl.MAX_TEXTURE_SIZE);

  const setTexture = (key, partial, x, y, w, h, mag, min, wrap, data) => {
    let tex = textures.get(key);
    if (!tex) {
      tex = gl.createTexture();
      textures.set(key, tex);
    }
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    if (partial) gl.texSubImage2D(gl.TEXTURE_2D, 0, x, y, w, h, gl.RGBA, gl.UNSIGNED_BYTE, data);
    else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, data);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, mag ? gl.LINEAR : gl.NEAREST);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, min ? gl.LINEAR : gl.NEAREST);
    const mode = wrap === 1 ? gl.REPEAT : wrap === 2 ? gl.MIRRORED_REPEAT : gl.CLAMP_TO_EDGE;
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, mode);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, mode);
  };

  const drawMesh = (tex, minx, miny, maxx, maxy, vbytes, icount, ibytes) => {
    const sx = Math.max(0, Math.floor(minx * ppp));
    const sy = Math.max(0, Math.floor(miny * ppp));
    const ex = Math.min(canvas.width, Math.ceil(maxx * ppp));
    const ey = Math.min(canvas.height, Math.ceil(maxy * ppp));
    // GL's scissor y counts from the bottom of the drawing buffer.
    if (ex <= sx || ey <= sy) gl.scissor(0, 0, 0, 0);
    else gl.scissor(sx, canvas.height - ey, ex - sx, ey - sy);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
    gl.bufferData(gl.ARRAY_BUFFER, vbytes, gl.STREAM_DRAW);
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, ibo);
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, ibytes, gl.STREAM_DRAW);
    gl.drawElements(gl.TRIANGLES, icount, gl.UNSIGNED_INT, 0);
  };

  // --- state ---------------------------------------------------------------
  const queue = [];
  // A release waits for the frame after its press, as a window delivers them
  // at human speed: the app's node clicks and drags read the press and the
  // release in frames of their own (a tap or a synthetic click sends both at once).
  let pressQueued = false;
  const afterPress = [];
  let mods = 0;
  let running = true;
  let rafId = 0;
  let waitId = 0;
  let repaintAt = 0;
  let ppp = window.devicePixelRatio || 1;
  let cssW = 0;
  let cssH = 0;
  let hoverFiles = false;
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform);

  const modBits = (ev) =>
    (ev.altKey ? 1 : 0) | (ev.ctrlKey ? 2 : 0) | (ev.shiftKey ? 4 : 0) |
    (ev.metaKey ? 8 : 0) | ((isMac ? ev.metaKey : ev.ctrlKey) ? 16 : 0);

  const push = (fn) => { queue.push(fn); wake(); };

  // --- scheduling ----------------------------------------------------------
  function wake() {
    if (!running) return;
    repaintAt = 0;
    if (waitId) { clearTimeout(waitId); waitId = 0; }
    if (!rafId) rafId = requestAnimationFrame(loop);
  }

  function arm() {
    if (!running || rafId || waitId) return;
    const wait = repaintAt - performance.now();
    if (!(wait > 16)) rafId = requestAnimationFrame(loop);
    else if (wait < Infinity) waitId = setTimeout(() => { waitId = 0; arm(); }, wait);
  }

  function loop() {
    rafId = 0;
    if (!running || document.hidden) return;
    const now = performance.now();
    if (queue.length === 0 && now < repaintAt) { arm(); return; }
    try {
      runFrame(now);
    } catch (err) {
      fail('VectorMagik stopped: ' + err.message);
      return;
    }
    if (running) arm();
  }

  function fail(message) {
    stop();
    if (onError) onError(message);
  }

  // --- the frame: input out, output in -------------------------------------
  function runFrame(now) {
    const encoded = queue.splice(0).map((fn) => fn());
    pressQueued = false;
    let size = 8 + 4 + 4 + 4 + 4 + 1 + 1 + 4; // time, size, ppp, max side, flags, count
    for (const ev of encoded) size += ev.length;

    const ptr = e.vm_alloc(size);
    let code;
    try {
      const mem = new Uint8Array(e.memory.buffer); // fresh: the heap can grow
      const view = new DataView(e.memory.buffer, ptr, size);
      let o = 0;
      view.setFloat64(o, now / 1000, true); o += 8;
      view.setFloat32(o, cssW, true); o += 4;
      view.setFloat32(o, cssH, true); o += 4;
      view.setFloat32(o, ppp, true); o += 4;
      view.setUint32(o, maxTextureSize, true); o += 4;
      view.setUint8(o, document.hasFocus() && document.activeElement === canvas ? 1 : 0); o += 1;
      view.setUint8(o, mods); o += 1;
      view.setUint32(o, encoded.length, true); o += 4;
      for (const ev of encoded) { mem.set(ev, ptr + o); o += ev.length; }
      code = e.vm_app_frame(ptr, size);
    } finally {
      e.vm_dealloc(ptr, size);
    }

    if (code === 1) {
      const out = new Uint8Array(e.memory.buffer, e.vm_out_ptr(), e.vm_out_len());
      fail(decoder.decode(out));
      return;
    }
    const out = new Uint8Array(e.memory.buffer, e.vm_out_ptr(), e.vm_out_len()).slice();
    handleOutput(out, now);
    if (afterPress.length) {
      for (const fn of afterPress.splice(0)) push(fn);
    }
  }

  function handleOutput(out, now) {
    const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
    let p = 0;
    const str = () => {
      const n = view.getUint32(p, true); p += 4;
      const s = decoder.decode(out.subarray(p, p + n)); p += n;
      return s;
    };
    const texKey = () => {
      const kind = view.getUint8(p); p += 1;
      const id = view.getBigUint64(p, true); p += 8;
      return kind + ':' + id;
    };

    // textures to set come before painting
    const setCount = view.getUint32(p, true); p += 4;
    for (let i = 0; i < setCount; i++) {
      const key = texKey();
      const partial = view.getUint8(p); p += 1;
      const x = view.getUint32(p, true); p += 4;
      const y = view.getUint32(p, true); p += 4;
      const w = view.getUint32(p, true); p += 4;
      const h = view.getUint32(p, true); p += 4;
      const mag = view.getUint8(p); p += 1;
      const min = view.getUint8(p); p += 1;
      const wrap = view.getUint8(p); p += 1;
      const data = out.subarray(p, p + w * h * 4); p += w * h * 4;
      setTexture(key, partial, x, y, w, h, mag, min, wrap, data);
    }

    // frees wait until after this frame is painted, egui's rule
    const freeCount = view.getUint32(p, true); p += 4;
    const toFree = [];
    for (let i = 0; i < freeCount; i++) toFree.push(texKey());

    gl.viewport(0, 0, canvas.width, canvas.height);
    gl.clearColor(13 / 255, 16 / 255, 20 / 255, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.useProgram(program);
    gl.uniform2f(uScreen, cssW, cssH);
    gl.bindVertexArray(vao);

    const meshCount = view.getUint32(p, true); p += 4;
    for (let i = 0; i < meshCount; i++) {
      const minx = view.getFloat32(p, true); p += 4;
      const miny = view.getFloat32(p, true); p += 4;
      const maxx = view.getFloat32(p, true); p += 4;
      const maxy = view.getFloat32(p, true); p += 4;
      const key = texKey();
      const vcount = view.getUint32(p, true); p += 4;
      const vbytes = out.subarray(p, p + vcount * 20); p += vcount * 20;
      const icount = view.getUint32(p, true); p += 4;
      const ibytes = out.subarray(p, p + icount * 4); p += icount * 4;
      const tex = textures.get(key);
      if (!tex) continue; // unknown texture: skip the mesh
      drawMesh(tex, minx, miny, maxx, maxy, vbytes, icount, ibytes);
    }

    for (const key of toFree) {
      const tex = textures.get(key);
      if (tex) { gl.deleteTexture(tex); textures.delete(key); }
    }

    canvas.style.cursor = str() || 'default';
    const copied = str();
    if (copied) navigator.clipboard?.writeText(copied).catch(() => {});
    const url = str();
    const newTab = view.getUint8(p); p += 1;
    // A new tab, unless a blocker refuses it (the frame runs just after the
    // click): then the page itself goes there rather than doing nothing.
    if (url) {
      const tab = newTab ? window.open(url, '_blank') : null;
      if (tab) tab.opener = null;
      else window.location.assign(url);
    }

    const nextSeconds = view.getFloat64(p, true); p += 8;
    repaintAt = nextSeconds >= 0 ? now + nextSeconds * 1000 : Infinity;

    const commandCount = view.getUint32(p, true); p += 4;
    let screenshot = false;
    for (let i = 0; i < commandCount; i++) {
      const tag = view.getUint8(p); p += 1;
      if (tag === 1) {
        hiddenInput.click();
      } else if (tag === 2) {
        const name = str();
        const mime = str();
        const n = view.getUint32(p, true); p += 4;
        const data = out.slice(p, p + n); p += n;
        download(name, mime, data);
      } else if (tag === 3) {
        const text = str();
        try { localStorage.setItem('vectormagik.prefs', text); } catch { /* private mode */ }
        // The app's answer to "personal or commercial use?" is the site's
        // license choice too (its header badge, 'vm-license').
        const use = /^licence_use=(personal|commercial)/m.exec(text);
        if (use) {
          try { localStorage.setItem('vm-license', use[1]); } catch { /* private mode */ }
          window.dispatchEvent(new CustomEvent('vm-license'));
        }
      } else if (tag === 4) {
        screenshot = true;
      } else if (tag === 5) {
        const id = view.getUint32(p, true); p += 4;
        const url = str();
        const body = str();
        postJson(id, url, body);
      }
    }

    if (screenshot) captureScreenshot();
  }

  // The painted frame is still in the drawing buffer here; read it back with
  // the top row first, as the protocol wants.
  function captureScreenshot() {
    const w = canvas.width;
    const h = canvas.height;
    if (!w || !h) return;
    const px = new Uint8Array(w * h * 4);
    gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, px);
    const flipped = new Uint8Array(px.length);
    const row = w * 4;
    for (let y = 0; y < h; y++) {
      flipped.set(px.subarray((h - 1 - y) * row, (h - y) * row), y * row);
    }
    push(evShot(w, h, flipped));
  }

  // The licence's one network call: its reply goes back as an event, 0 for
  // no response at all.
  function postJson(id, url, body) {
    fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body })
      .then(async (r) => push(evReply(id, r.status, await r.text())))
      .catch(() => push(evReply(id, 0, '')));
  }

  function download(name, mime, data) {
    const url = URL.createObjectURL(new Blob([data], { type: mime }));
    const a = document.createElement('a');
    a.href = url;
    a.download = name || 'download';
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 60000);
  }

  async function readFile(file) {
    const data = new Uint8Array(await file.arrayBuffer());
    const name = file.name;
    push(evFile(name, data));
  }

  // --- sizing --------------------------------------------------------------
  const resize = () => {
    const parent = canvas.parentElement;
    if (!parent) return;
    ppp = window.devicePixelRatio || 1;
    cssW = parent.clientWidth;
    cssH = parent.clientHeight;
    canvas.style.width = cssW + 'px';
    canvas.style.height = cssH + 'px';
    canvas.width = Math.round(cssW * ppp);
    canvas.height = Math.round(cssH * ppp);
    wake();
  };
  const ro = new ResizeObserver(resize);
  ro.observe(canvas.parentElement || document.body);

  canvas.style.touchAction = 'none';
  canvas.style.outline = 'none';
  canvas.tabIndex = 0;

  // A hidden picker for file commands and drag-and-drop's fallback.
  const hiddenInput = document.createElement('input');
  hiddenInput.type = 'file';
  hiddenInput.accept = ACCEPT;
  hiddenInput.style.display = 'none';
  hiddenInput.addEventListener('change', () => {
    const file = hiddenInput.files[0];
    if (file) readFile(file);
    hiddenInput.value = '';
  });
  document.body.appendChild(hiddenInput);

  // --- input ---------------------------------------------------------------
  const ctrl = new AbortController();
  const signal = ctrl.signal;
  const on = (target, type, fn, opts) => target.addEventListener(type, fn, { ...opts, signal });

  const localPos = (event) => {
    const r = canvas.getBoundingClientRect();
    return [event.clientX - r.left, event.clientY - r.top];
  };
  const buttonOf = (button) => (button === 2 ? 1 : button === 1 ? 2 : button >= 3 ? button : 0);

  on(canvas, 'pointermove', (event) => {
    mods = modBits(event);
    const [x, y] = localPos(event);
    push(evMoved(x, y));
  });

  on(canvas, 'pointerdown', (event) => {
    mods = modBits(event);
    canvas.focus();
    try { canvas.setPointerCapture(event.pointerId); } catch { /* already gone */ }
    const [x, y] = localPos(event);
    const button = buttonOf(event.button);
    push(evMoved(x, y));
    push(evButton(x, y, button, 1));
    pressQueued = true;
  });

  on(canvas, 'pointerup', (event) => {
    mods = modBits(event);
    const [x, y] = localPos(event);
    const release = evButton(x, y, buttonOf(event.button), 0);
    if (pressQueued) afterPress.push(release);
    else push(release);
    try { canvas.releasePointerCapture(event.pointerId); } catch { /* not captured */ }
  });

  on(canvas, 'pointerleave', (event) => {
    if (canvas.hasPointerCapture(event.pointerId)) return;
    push(evGone());
  });

  on(canvas, 'contextmenu', (event) => event.preventDefault());

  on(canvas, 'wheel', (event) => {
    mods = modBits(event);
    event.preventDefault();
    const unit = event.deltaMode === 1 ? 1 : event.deltaMode === 2 ? 2 : 0;
    // Already negated: how far the content moves. Shift+wheel stays as is,
    // egui turns it into horizontal scrolling itself.
    push(evWheel(unit, -event.deltaX, -event.deltaY));
  }, { passive: false });

  on(canvas, 'keydown', (event) => {
    mods = modBits(event);
    push(evKey(event.key, event.code, 1, event.repeat));
    // A key typed while an input method composes arrives as 'Process', never
    // one character, so this sends only plain typing.
    if (event.key.length === 1 && !event.ctrlKey && !event.metaKey) {
      push(evText(event.key));
    }
    if (shouldPrevent(event)) event.preventDefault();
  });

  on(canvas, 'keyup', (event) => {
    mods = modBits(event);
    push(evKey(event.key, event.code, 0, event.repeat));
  });

  function shouldPrevent(event) {
    const key = event.key;
    if (event.shiftKey && (key === 'i' || key === 'I')) return false; // devtools
    if (key === 'F12' || key === 'F5') return false; // devtools, reload
    if (PREVENT_KEYS.has(key)) return true;
    if ((event.ctrlKey || event.metaKey) && !'cvxrCVXR'.includes(key)) return true;
    return false;
  }

  on(document, 'copy', (event) => {
    if (document.activeElement !== canvas) return;
    event.preventDefault();
    push(evSimple(7));
  });
  on(document, 'cut', (event) => {
    if (document.activeElement !== canvas) return;
    event.preventDefault();
    push(evSimple(8));
  });
  on(document, 'paste', (event) => {
    if (document.activeElement !== canvas) return;
    event.preventDefault();
    const text = event.clipboardData ? event.clipboardData.getData('text/plain') : '';
    push(evPaste(text));
  });

  const dropZone = canvas.parentElement || document.body;
  on(dropZone, 'dragover', (event) => {
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
    if (!hoverFiles) { hoverFiles = true; push(evHover(1)); }
  });
  on(dropZone, 'dragleave', () => {
    if (!hoverFiles) return;
    hoverFiles = false;
    push(evHover(0));
  });
  on(dropZone, 'drop', (event) => {
    event.preventDefault();
    if (hoverFiles) { hoverFiles = false; push(evHover(0)); }
    const file = event.dataTransfer && event.dataTransfer.files[0];
    if (file) readFile(file);
  });

  on(window, 'focus', wake);
  on(window, 'blur', wake);
  on(window, 'resize', resize);
  on(document, 'visibilitychange', () => { if (!document.hidden) wake(); });

  // --- start the app -------------------------------------------------------
  let prefs = '';
  try { prefs = localStorage.getItem('vectormagik.prefs') || ''; } catch { /* private mode */ }
  // A visitor who already told the site personal or commercial use is not
  // asked again by the app.
  try {
    const choice = localStorage.getItem('vm-license');
    if ((choice === 'personal' || choice === 'commercial') && !/^licence_use=/m.test(prefs)) {
      prefs += `licence_use=${choice}\r\n`;
    }
  } catch { /* private mode */ }
  const prefsBytes = encoder.encode(prefs);
  const prefsLen = Math.max(prefsBytes.length, 1);
  const prefsPtr = e.vm_alloc(prefsLen);
  if (prefsBytes.length) new Uint8Array(e.memory.buffer, prefsPtr, prefsBytes.length).set(prefsBytes);
  e.vm_app_start(prefsPtr, prefsBytes.length);
  e.vm_dealloc(prefsPtr, prefsLen);

  // The page's light and dark switch (data-theme on <html>) is the app's
  // too; without one, the system's preference.
  const pageDark = () => {
    const theme = document.documentElement.dataset.theme;
    if (theme === 'light' || theme === 'dark') return theme === 'dark';
    return !(window.matchMedia && window.matchMedia('(prefers-color-scheme: light)').matches);
  };
  const sendTheme = () => { push(evTheme(pageDark())); wake(); };
  const themeWatch = new MutationObserver(sendTheme);
  themeWatch.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
  sendTheme();

  resize();
  canvas.focus({ preventScroll: true });
  wake();

  return {
    stop() {
      if (!running) return;
      running = false;
      if (rafId) cancelAnimationFrame(rafId);
      if (waitId) clearTimeout(waitId);
      rafId = 0;
      waitId = 0;
      ro.disconnect();
      themeWatch.disconnect();
      ctrl.abort();
      hiddenInput.remove();
    },
  };
}
