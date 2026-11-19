// The journey: one fixed canvas behind the whole page, driven by scroll. It
// is liquid light and nothing else.
//
//  At the top   the liquid is hot: molten folds beside the hero's words on a
//               wide screen, over them on a narrow one. The light is a place
//               on the page, so it scrolls away with the hero.
//  Through it   the same liquid at rest: indigo and umber silk behind the
//               sections (pearl by day), stopped down behind every block of
//               copy, with a ribbon of light crossing two of the gaps between
//               them. The scroll and the cursor stir it.
//  At the end   it is hot again, under the closing call to action.
//
// The liquid is a warped field and a fluid simulation (journey-liquid.ts).
// Raw WebGL2 with no dependency, in its own chunk, loaded after the hero's
// words have painted; until it draws, and wherever it cannot, the page shows
// a still of it in the same place (journey.css).
import { LIQ_FS, LIQN_FS, COMPOSE, SIM_VS, SIM, BRIGHT, BLUR, FINAL } from "./journey-liquid";

export type Options = { light: boolean; still: boolean; paused: boolean };
export type State = "running" | "paused" | "still" | "away" | "fallback" | "software";
export type Handle = { update(next: Partial<Options>): void; destroy(): void };

type GL = WebGL2RenderingContext;
type Prog = { p: WebGLProgram; u: Record<string, WebGLUniformLocation | null> };
type Target = { fb: WebGLFramebuffer; texs: WebGLTexture[]; w: number; h: number };
type Pair = { read: Target; write: Target };
type Setter = {
  tex(name: string, t: WebGLTexture): void;
  f(name: string, ...v: number[]): void;
  i(name: string, v: number): void;
  fv(name: string, size: 1 | 2 | 3 | 4, arr: Float32Array): void;
};

const FULLSCREEN_VS = `#version 300 es
in vec2 a_pos;
out vec2 v_uv;
void main() { v_uv = a_pos * 0.5 + 0.5; gl_Position = vec4(a_pos, 0.0, 1.0); }`;

/** The fixed step of the fluid and of the liquid's clock. */
const DT = 1 / 60;
/** The moment of the liquid that the composed still holds. */
const STILL_T = 8.5;
/** A ceiling on drawn pixels, so a 5K screen costs what a 1440p one does. */
const MAX_PIXELS = 5.4e6;
/** How many copy blocks the frame can hold calm at once. */
const CALM = 8;
// Quality steps, best first. The loop measures its own frame time and steps
// down when it misses frames; it tries one step back up after a long calm.
const LEVELS = [
  { dpr: 2, sim: 160, dye: 640 },
  { dpr: 1.5, sim: 144, dye: 520 },
  { dpr: 1.25, sim: 128, dye: 420 },
  { dpr: 1, sim: 112, dye: 360 },
];
const ease = (x: number) => {
  x = Math.max(0, Math.min(1, x));
  return x * x * (3 - 2 * x);
};
const clamp = (x: number, a: number, b: number) => Math.max(a, Math.min(b, x));
function program(gl: GL, vs: string, fs: string): Prog {
  const sh = (type: number, src: string) => {
    const s = gl.createShader(type);
    if (!s) throw new Error("shader");
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s) ?? "compile");
    return s;
  };
  const p = gl.createProgram();
  if (!p) throw new Error("program");
  gl.attachShader(p, sh(gl.VERTEX_SHADER, vs));
  gl.attachShader(p, sh(gl.FRAGMENT_SHADER, fs));
  gl.bindAttribLocation(p, 0, "a_pos");
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "link");
  const u: Prog["u"] = {};
  const n = gl.getProgramParameter(p, gl.ACTIVE_UNIFORMS) as number;
  for (let i = 0; i < n; i++) {
    const info = gl.getActiveUniform(p, i);
    if (info) u[info.name.replace(/\[0\]$/, "")] = gl.getUniformLocation(p, info.name);
  }
  return { p, u };
}

/** True when WebGL is drawn on the CPU (SwiftShader, llvmpipe): it would
 * draw the liquid slowly enough to drag the whole page with it. Chrome hides
 * the renderer behind the debug extension; Firefox and Safari name it in
 * RENDERER. */
function software(gl: GL): boolean {
  let name = String(gl.getParameter(gl.RENDERER) ?? "");
  if (/^webkit webgl$/i.test(name)) {
    const info = gl.getExtension("WEBGL_debug_renderer_info");
    if (info) name = String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL) ?? "");
  }
  return /swiftshader|llvmpipe|softpipe|software|basic render driver/i.test(name);
}

/**
 * Starts the journey on `canvas`, which fills `host`. It reads the page
 * itself: the header (.top), the hero's words (.hero-copy), the sections of
 * <main>, the closing headline and the footer, for where the acts fall and
 * what to hold calm.
 *
 * Returns null where WebGL2 is missing or the liquid fails to build, and the
 * poster stays.
 */
export function mount(host: HTMLElement, canvas: HTMLCanvasElement, initial: Options, onState: (s: State) => void): Handle | null {
  const ctx = canvas.getContext("webgl2", { alpha: false, antialias: false, depth: false, stencil: false, powerPreference: "high-performance" });
  if (!ctx) return null;
  const gl: GL = ctx;
  if (!gl.getExtension("EXT_color_buffer_float") && !gl.getExtension("EXT_color_buffer_half_float")) return null;
  // The liquid's surface is lit from its slope. Where full floats filter and
  // render, the slope is differenced from the height; otherwise the field's
  // own derivative is used (journey-liquid.ts). `?journey=half` forces the
  // second path, to test it on a machine that has the first.
  const forced = new URLSearchParams(window.location.search).get("journey");
  const float32 = forced !== "half" && !!gl.getExtension("OES_texture_float_linear") && !!gl.getExtension("EXT_color_buffer_float");

  let opts = { ...initial };
  type Progs = Record<"liq" | "liqn" | "compose" | "bright" | "blur" | "final" | keyof typeof SIM, Prog>;
  let prog!: Progs;
  let vao: WebGLVertexArrayObject | null = null;
  function build() {
    const P = (fs: string) => program(gl, FULLSCREEN_VS, fs);
    const S = (fs: string) => program(gl, SIM_VS, fs);
    prog = {
      liq: P(LIQ_FS), liqn: P(LIQN_FS), compose: P(COMPOSE), bright: P(BRIGHT), blur: P(BLUR), final: P(FINAL),
      splat: S(SIM.splat), advect: S(SIM.advect), forces: S(SIM.forces), curl: S(SIM.curl), vort: S(SIM.vort), div: S(SIM.div), clear: S(SIM.clear), press: S(SIM.press), grad: S(SIM.grad),
    };
    vao = gl.createVertexArray();
    gl.bindVertexArray(vao);
    const buf = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buf);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  }
  try {
    build();
  } catch {
    return null;
  }

  // ---- targets
  const live: Target[] = [];
  function target(w: number, h: number, n = 1, full = false): Target {
    const fb = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
    const texs: WebGLTexture[] = [];
    for (let i = 0; i < n; i++) {
      const t = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, t);
      if (full) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, w, h, 0, gl.RGBA, gl.FLOAT, null);
      else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0 + i, gl.TEXTURE_2D, t, 0);
      texs.push(t);
    }
    gl.drawBuffers(texs.map((_, i) => gl.COLOR_ATTACHMENT0 + i));
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    const t = { fb, texs, w, h };
    live.push(t);
    return t;
  }
  const pair = (w: number, h: number): Pair => ({ read: target(w, h), write: target(w, h) });
  const swap = (p: Pair) => {
    [p.read, p.write] = [p.write, p.read];
  };
  function free() {
    for (const t of live.splice(0)) {
      gl.deleteFramebuffer(t.fb);
      t.texs.forEach((x) => gl.deleteTexture(x));
    }
  }

  function pass(pr: Prog, out: { fb: WebGLFramebuffer | null; w: number; h: number }, set: (a: Setter) => void) {
    gl.useProgram(pr.p);
    let unit = 0;
    set({
      tex(name, t) {
        gl.activeTexture(gl.TEXTURE0 + unit);
        gl.bindTexture(gl.TEXTURE_2D, t);
        gl.uniform1i(pr.u[name] ?? null, unit);
        unit++;
      },
      f(name, ...v) {
        const at = pr.u[name];
        if (!at) return;
        if (v.length === 1) gl.uniform1f(at, v[0]);
        else if (v.length === 2) gl.uniform2f(at, v[0], v[1]);
        else if (v.length === 3) gl.uniform3f(at, v[0], v[1], v[2]);
        else gl.uniform4f(at, v[0], v[1], v[2], v[3]);
      },
      i(name, v) {
        const at = pr.u[name];
        if (at) gl.uniform1i(at, v);
      },
      fv(name, size, arr) {
        const at = pr.u[name];
        if (!at) return;
        if (size === 1) gl.uniform1fv(at, arr);
        else if (size === 2) gl.uniform2fv(at, arr);
        else if (size === 3) gl.uniform3fv(at, arr);
        else gl.uniform4fv(at, arr);
      },
    });
    if (pr.u.texel) gl.uniform2f(pr.u.texel, 1 / out.w, 1 / out.h);
    gl.bindFramebuffer(gl.FRAMEBUFFER, out.fb);
    gl.viewport(0, 0, out.w, out.h);
    gl.bindVertexArray(vao);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  // ---- state
  const cpu = software(gl);
  let level = cpu ? LEVELS.length - 1 : matchMedia("(pointer: coarse)").matches ? 1 : 0;
  let t = opts.still ? STILL_T : 0;
  let raf = 0;
  let last = 0;
  let acc = 0;
  let visible = true;
  let lost = false;
  let dirty = true;
  let state: State | null = null;
  let scrollV = 0;
  let lastScroll = window.scrollY;
  const size = { w: 0, h: 0, dpr: 1, W: 0, H: 0, aspect: 1 };
  const ptr = { x: 0, y: 0, dx: 0, dy: 0, moved: false };
  // Frame times, for stepping the quality
  const times: number[] = [];
  let changed = performance.now();
  const failed = new Set<number>();
  let R: {
    scene: Target; liq: Target; liqn: Target;
    b1: Pair; b2: Pair; b3: Pair; vel: Pair; dye: Pair; pres: Pair; div: Target; curl: Target;
  } | null = null;

  // ---- the page: where the acts fall, and what must stay calm
  const page = { top: null as Element | null, heroCopy: null as Element | null, hero: null as Element | null, closeH: null as Element | null, closeCta: null as Element | null, foot: null as Element | null, mid: [] as Element[], how: null as Element | null, hosting: null as Element | null };
  function findPage() {
    const q = (s: string) => document.querySelector(s);
    page.top = q(".top");
    page.hero = q("main > .hero");
    page.heroCopy = q("main .hero-copy");
    page.closeH = q(".close h2");
    page.closeCta = q(".close .cta-row");
    page.foot = q(".foot");
    page.how = q("main > .how");
    page.hosting = q("main > .hosting");
    page.mid = Array.from(document.querySelectorAll("main > section:not(.hero):not(.close)"));
  }
  /** The box of a section's own content: its children, without its padding. */
  function content(section: Element): DOMRect | null {
    let l = Infinity;
    let tp = Infinity;
    let r = -Infinity;
    let b = -Infinity;
    for (const child of Array.from(section.children)) {
      if (child.classList.contains("media-toggle")) continue;
      const c = child.getBoundingClientRect();
      if (c.width <= 0 || c.height <= 0) continue;
      l = Math.min(l, c.left);
      tp = Math.min(tp, c.top);
      r = Math.max(r, c.right);
      b = Math.max(b, c.bottom);
    }
    return r > l ? new DOMRect(l, tp, r - l, b - tp) : null;
  }

  function resize() {
    const lv = LEVELS[level];
    const w = host.clientWidth;
    const h = host.clientHeight;
    if (w < 2 || h < 2) return;
    let dpr = Math.min(window.devicePixelRatio || 1, lv.dpr);
    if (w * h * dpr * dpr > MAX_PIXELS) dpr = Math.sqrt(MAX_PIXELS / (w * h));
    // on the CPU one small frame proves the liquid draws, and no more is drawn
    if (cpu) dpr = Math.min(dpr, 120 / w);
    const W = Math.max(2, Math.round(w * dpr));
    const H = Math.max(2, Math.round(h * dpr));
    if (R && W === size.W && H === size.H) return;
    Object.assign(size, { w, h, dpr, W, H, aspect: w / h });
    canvas.width = W;
    canvas.height = H;
    free();
    const sw = size.aspect >= 1 ? Math.round(lv.sim * size.aspect) : lv.sim;
    const sh = size.aspect >= 1 ? lv.sim : Math.round(lv.sim / size.aspect);
    const dyeS = w < 720 ? Math.min(lv.dye, 420) : lv.dye;
    const dw = size.aspect >= 1 ? Math.round(dyeS * size.aspect) : dyeS;
    const dh = size.aspect >= 1 ? dyeS : Math.round(dyeS / size.aspect);
    const q = (n: number): [number, number] => [Math.ceil(W / n), Math.ceil(H / n)];
    // the liquid field: half the frame on a wide screen, all of it on a phone
    const lq = q(w < 720 || dpr < 1.4 ? 1 : 2);
    R = {
      scene: target(W, H),
      liq: target(lq[0], lq[1], 3, float32), liqn: target(lq[0], lq[1]),
      b1: pair(...q(4)), b2: pair(...q(8)), b3: pair(...q(16)),
      vel: pair(sw, sh), dye: pair(dw, dh), pres: pair(sw, sh), div: target(sw, sh), curl: target(sw, sh),
    };
    findPage();
    dirty = true;
  }

  // ---- scroll: where the page is, and what that asks of the picture
  function journey() {
    const vh = window.innerHeight;
    const ch = size.h;
    const y = window.scrollY;
    // The light is a place on the page. It stands in the hero, beside the
    // words on a wide screen and over them on a narrow one, and scrolls away
    // with the hero. It stands again at the foot of the page, under the
    // closing call to action, and comes up over the last screen of the scroll.
    const wide = size.w >= 1080;
    const max = Math.max(0, document.documentElement.scrollHeight - vh);
    const heroH = page.hero?.getBoundingClientRect().height ?? vh;
    const stage = page.heroCopy ? page.heroCopy.getBoundingClientRect().top + y : vh * 0.46;
    const upC = wide ? heroH * 0.52 : stage * 0.5;
    const hotUp = 1 - ease((y - vh * 0.25) / (vh * 0.95));
    // (low on the last screen, so the closing headline stands on quiet silk and the light comes up under it)
    const downC = max + vh * 0.78;
    const hotDown = ease((1 - (max - y) / (vh * 0.95)) / 0.6);
    const atClose = hotDown > hotUp;
    // (a little under full at the close: it fills the screen's width there)
    const hot = Math.max(hotUp, hotDown * 0.86);
    // its middle up the frame and its reach, then its middle across and its reach
    const hotBand = atClose ? [1 - (downC - y) / ch, 0.34] : [1 - (upC - y) / ch, wide ? 0.62 : Math.max(0.3, (stage / ch) * 0.62)];
    const hotSpan = atClose ? [0.5, wide ? 0.8 : 1.1] : wide ? [0.72, 0.42] : [0.5, 0.95];
    // ribbons: across the gap over "How it works" and over "Hosted or
    // self-hosted", drawn on by the scroll
    const bands = new Float32Array(16);
    let nb = 0;
    [page.how, page.hosting].forEach((el, i) => {
      const prev = el?.previousElementSibling;
      if (!el || !prev) return;
      const g1 = content(el)?.top;
      const g0 = content(prev)?.bottom;
      if (g0 === undefined || g1 === undefined || g1 < -vh * 0.25 || g0 > vh * 1.25) return;
      const prg = clamp((vh * 1.1 - g0) / (vh * 1.2 + (g1 - g0)), 0, 1);
      bands.set([1 - (g0 + g1) / 2 / ch, clamp(((g1 - g0) / ch) * 0.26, 0.03, 0.14), prg, i + 1], nb * 4);
      nb++;
    });
    // calm: the header, and every block of copy on screen
    const calm = new Float32Array(CALM * 4).fill(-9999);
    const feather = new Float32Array(CALM).fill(1);
    let nc = 0;
    const hold = (r: DOMRect | null | undefined, soft: number, pad: number) => {
      if (!r || nc >= CALM || r.bottom < 0 || r.top > ch || r.width <= 0) return;
      calm.set([(r.left - pad) * size.dpr, (ch - r.bottom - pad) * size.dpr, (r.right + pad) * size.dpr, (ch - r.top + pad) * size.dpr], nc * 4);
      feather[nc] = soft * size.dpr;
      nc++;
    };
    // a wide page lets the light go slowly under the words; on a narrow one it
    // stands right over them and must keep its strength
    const soft = size.w >= 1080 ? (opts.light ? 250 : 190) : opts.light ? 100 : 90;
    hold(page.top?.getBoundingClientRect(), 36, 0);
    hold(page.heroCopy?.getBoundingClientRect(), soft, 10);
    for (const s of page.mid) {
      const box = s.getBoundingClientRect();
      if (box.bottom > 0 && box.top < ch) hold(content(s), soft, 10);
    }
    hold(page.closeH?.getBoundingClientRect(), soft, 26);
    hold(page.closeCta?.getBoundingClientRect(), soft, 14);
    hold(page.foot?.getBoundingClientRect(), 110, 0);
    return { y, hot, hotBand, hotSpan, bands, calm, feather };
  }
  type Journey = ReturnType<typeof journey>;
  // What the last frame read of the page, so the fluid's step does not read it again
  let seen: Journey | null = null;

  function splat(x: number, y: number, vx: number, vy: number, dens: number, heat: number, radius: number) {
    if (!R) return;
    const r = R;
    pass(prog.splat, r.vel.write, (a) => { a.tex("uTarget", r.vel.read.texs[0]); a.f("aspect", size.aspect); a.f("point", x, y); a.f("radius", radius); a.f("color", vx, vy, 0, 0); });
    swap(r.vel);
    if (dens > 0 || heat > 0) {
      pass(prog.splat, r.dye.write, (a) => { a.tex("uTarget", r.dye.read.texs[0]); a.f("aspect", size.aspect); a.f("point", x, y); a.f("radius", radius * 1.15); a.f("color", dens, heat, 0, 0); });
      swap(r.dye);
    }
  }

  /** One step of the fluid and of the clock. */
  function step(dt: number) {
    if (!R) return;
    const r = R;
    const j = seen ?? journey();
    // where it is hot, light wells up from under the middle of the heat
    if (j.hot > 0.04) {
      const fx = j.hotSpan[0];
      for (let k = 0; k < 2; k++) {
        const x = fx + 0.2 * Math.sin(t * 0.5 + k * 2.6) + 0.07 * Math.sin(t * 1.3 + k);
        const on = j.hot * (0.5 + 0.5 * Math.sin(t * 1.1 + k * 3.1) ** 2);
        splat(x, j.hotBand[0] - j.hotBand[1] * 0.7, 0.5 * Math.sin(t * 0.4 + k * 1.7) * dt * on, 1.6 * dt * on, 0.5 * dt * on, 0.6 * dt * on, 0.0012);
      }
    }
    if (ptr.moved) {
      // the cursor stirs the liquid and leaves a little light in it
      splat(ptr.x / size.w, 1 - ptr.y / size.h, (ptr.dx / size.w) * 9, (-ptr.dy / size.h) * 9, 0.05, 0.045, 0.0011);
      ptr.dx = 0;
      ptr.dy = 0;
      ptr.moved = false;
    }
    pass(prog.forces, r.vel.write, (a) => { a.tex("uVelocity", r.vel.read.texs[0]); a.tex("uDye", r.dye.read.texs[0]); a.f("dt", dt); a.f("buoy", 0.3); a.f("push", scrollV * 0.9); a.f("uT", t); a.f("aspect", size.aspect); });
    swap(r.vel);
    pass(prog.curl, r.curl, (a) => a.tex("uVelocity", r.vel.read.texs[0]));
    pass(prog.vort, r.vel.write, (a) => { a.tex("uVelocity", r.vel.read.texs[0]); a.tex("uCurl", r.curl.texs[0]); a.f("curl", 5); a.f("dt", dt); });
    swap(r.vel);
    pass(prog.div, r.div, (a) => a.tex("uVelocity", r.vel.read.texs[0]));
    pass(prog.clear, r.pres.write, (a) => { a.tex("uTexture", r.pres.read.texs[0]); a.f("value", 0.8); });
    swap(r.pres);
    for (let i = 0; i < 12; i++) {
      pass(prog.press, r.pres.write, (a) => { a.tex("uPressure", r.pres.read.texs[0]); a.tex("uDivergence", r.div.texs[0]); });
      swap(r.pres);
    }
    pass(prog.grad, r.vel.write, (a) => { a.tex("uPressure", r.pres.read.texs[0]); a.tex("uVelocity", r.vel.read.texs[0]); });
    swap(r.vel);
    pass(prog.advect, r.vel.write, (a) => { a.tex("uVelocity", r.vel.read.texs[0]); a.tex("uSource", r.vel.read.texs[0]); a.f("dt", dt); a.f("decay", 0.7, 0.7, 0, 0); a.f("gain", 0); });
    swap(r.vel);
    pass(prog.advect, r.dye.write, (a) => { a.tex("uVelocity", r.vel.read.texs[0]); a.tex("uSource", r.dye.read.texs[0]); a.f("dt", dt); a.f("decay", 0.55, 0.95, 0.3, 0.3); a.f("gain", 0.7); });
    swap(r.dye);
    scrollV *= Math.exp(-dt * 5);
    t += dt;
  }

  /** Draws a frame. */
  function draw() {
    if (!R || lost) return;
    const r = R;
    const { W, H } = size;
    const j = journey();
    seen = j;
    pass(prog.liq, r.liq, (a) => {
      a.tex("u_dye", r.dye.read.texs[0]); a.f("u_dTexel", 1 / r.dye.read.w, 1 / r.dye.read.h); a.f("u_t", t); a.f("u_scroll", j.y / size.h); a.f("u_aspect", size.aspect); a.f("u_hot", j.hot);
      a.f("u_hotBand", j.hotBand[0], j.hotBand[1]); a.f("u_hotSpan", j.hotSpan[0], j.hotSpan[1]); a.fv("u_bands", 4, j.bands);
    });
    pass(prog.liqn, r.liqn, (a) => { a.tex("u_l0", r.liq.texs[0]); a.tex("u_l2", r.liq.texs[2]); a.f("u_tx", 1 / r.liq.w, 1 / r.liq.h); a.i("uMode", float32 ? 0 : 1); });
    pass(prog.compose, r.scene, (a) => {
      a.tex("u_l0", r.liq.texs[0]); a.tex("u_l1", r.liq.texs[1]); a.tex("u_ln", r.liqn.texs[0]); a.i("u_light", opts.light ? 1 : 0);
      // the liquid's relief, by the size of its own grid
      a.f("u_rel", (r.liq.h / 900) * (opts.light ? 160 : 260) * (r.liq.h > size.h * 1.2 ? 0.6 : 1));
    });
    // At night the bright pass feeds the bloom; by day all of the picture goes
    // through, and the blur is a diffusion filter over it (FINAL).
    {
      let src: Target = r.scene;
      [r.b1, r.b2, r.b3].forEach((b, i) => {
        if (i === 0) pass(prog.bright, b.write, (a) => { a.tex("uTex", src.texs[0]); a.f("thresh", opts.light ? -0.7 : 0.5); });
        else pass(prog.blur, b.write, (a) => { a.tex("uTex", src.texs[0]); a.f("dir", 0, 0); });
        swap(b);
        pass(prog.blur, b.write, (a) => { a.tex("uTex", b.read.texs[0]); a.f("dir", 1 / b.read.w, 0); });
        swap(b);
        pass(prog.blur, b.write, (a) => { a.tex("uTex", b.read.texs[0]); a.f("dir", 0, 1 / b.read.h); });
        swap(b);
        src = b.read;
      });
    }
    pass(prog.final, { fb: null, w: W, h: H }, (a) => {
      a.tex("uScene", r.scene.texs[0]); a.tex("uB1", r.b1.read.texs[0]); a.tex("uB2", r.b2.read.texs[0]); a.tex("uB3", r.b3.read.texs[0]);
      a.i("uLight", opts.light ? 1 : 0); a.f("uPx", W, H); a.f("uSeed", running() ? (t * 7.31) % 1 : 0.37); a.fv("uCalm", 4, j.calm); a.fv("uFeather", 1, j.feather); a.f("uBloom", 0.62);
    });
    dirty = false;
  }

  // ---- driving it
  const running = () => visible && !document.hidden && !opts.paused && !opts.still && !lost;
  function setState(next: State) {
    if (next === state) return;
    state = next;
    onState(next);
  }
  function schedule() {
    if (lost) return;
    if ((running() || dirty) && !raf) {
      last = performance.now();
      raf = requestAnimationFrame(tick);
    }
    setState(opts.still ? "still" : opts.paused ? "paused" : running() ? "running" : "away");
  }
  /** Asks for one frame: the page scrolled or changed while the clock is stopped. */
  function redraw() {
    dirty = true;
    schedule();
  }

  function adapt(ms: number) {
    const now = performance.now();
    times.push(ms);
    if (times.length > 90) times.shift();
    if (now - changed < 1500 || times.length < 45) return;
    const sorted = [...times].sort((a, b) => a - b);
    const median = sorted[Math.floor(sorted.length / 2)];
    const slow = sorted[Math.floor(sorted.length * 0.95)];
    if (median > 19 && level < LEVELS.length - 1) {
      failed.add(level);
      level++;
    } else if (slow < 17.5 && level > 0 && !failed.has(level - 1) && now - changed > 20000) {
      level--;
    } else return;
    changed = now;
    times.length = 0;
    resize();
  }

  let frames = 0;
  function tick(now: number) {
    raf = 0;
    const ms = now - last;
    last = now;
    const on = running();
    if (on) {
      acc += Math.min(0.05, ms / 1000);
      let n = 0;
      while (acc >= DT && n < 2) {
        step(DT);
        acc -= DT;
        n++;
      }
      if (acc > DT) acc = 0;
    }
    draw();
    if (on && ++frames > 30) adapt(ms);
    if (running() || dirty) raf = requestAnimationFrame(tick);
  }

  const onPointer = (e: PointerEvent) => {
    ptr.dx += e.clientX - ptr.x;
    ptr.dy += e.clientY - ptr.y;
    ptr.x = e.clientX;
    ptr.y = e.clientY;
    ptr.moved = true;
  };
  const onScroll = () => {
    const y = window.scrollY;
    scrollV = clamp(scrollV + ((y - lastScroll) / Math.max(1, size.h)) * 2.2, -3, 3);
    lastScroll = y;
    if (!running()) redraw();
  };
  window.addEventListener("pointermove", onPointer, { passive: true });
  window.addEventListener("scroll", onScroll, { passive: true });
  const io = new IntersectionObserver(([e]) => {
    visible = e.isIntersecting;
    schedule();
  });
  io.observe(host);
  // the page's own layout moves the acts (fonts arriving, the board loading)
  const ro = new ResizeObserver(() => {
    resize();
    findPage();
    redraw();
  });
  ro.observe(host);
  ro.observe(document.body);
  const onVisibility = () => schedule();
  document.addEventListener("visibilitychange", onVisibility);
  // A lost context leaves the poster showing; a restored one starts over
  const onLost = (e: Event) => {
    e.preventDefault();
    lost = true;
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    setState("fallback");
  };
  const onRestored = () => {
    try {
      live.length = 0;
      R = null;
      // A restored context comes back with its extensions off.
      if (!gl.getExtension("EXT_color_buffer_float") && !gl.getExtension("EXT_color_buffer_half_float")) throw new Error("no float targets");
      if (float32 && !gl.getExtension("OES_texture_float_linear")) throw new Error("no float filtering");
      build();
      lost = false;
      state = null;
      resize();
      redraw();
    } catch {
      setState("fallback");
    }
  };
  canvas.addEventListener("webglcontextlost", onLost);
  canvas.addEventListener("webglcontextrestored", onRestored);

  function destroy() {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    io.disconnect();
    ro.disconnect();
    window.removeEventListener("pointermove", onPointer);
    window.removeEventListener("scroll", onScroll);
    document.removeEventListener("visibilitychange", onVisibility);
    canvas.removeEventListener("webglcontextlost", onLost);
    canvas.removeEventListener("webglcontextrestored", onRestored);
    free();
    R = null;
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }

  try {
    findPage();
    resize();
    draw();
  } catch {
    destroy();
    return null;
  }
  // On a software renderer the one small frame above proves the liquid draws;
  // the poster stays up rather than the page paying for every frame after it
  if (cpu) {
    destroy();
    onState("software");
    return { update() {}, destroy() {} };
  }
  schedule();

  return {
    update(next) {
      const was = opts;
      opts = { ...opts, ...next };
      if (opts.still && !was.still) t = STILL_T;
      redraw();
    },
    destroy,
  };
}
