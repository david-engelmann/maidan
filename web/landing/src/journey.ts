// The journey: one fixed canvas behind the whole page, driven by scroll.
//
//  Act 1  a low view across a night square, in the spirit of Madison Square
//         Park: a fountain of liquid light on the axis, the flat-iron wedge
//         lit behind it, wet paving that mirrors both, people carrying small
//         lights about.
//  Act 2  scrolling lifts the camera through the fountain's plume, molten
//         folds closing round the frame, and out into the same liquid at
//         rest: indigo and umber silk behind the sections, stopped down
//         behind every block of copy, with a ribbon of light crossing two of
//         the gaps between them. The scroll and the cursor stir it.
//  Act 3  at the closing call to action the liquid drains back to the
//         fountain and the camera pulls back, so the square's eight rills
//         read as rays: the mark is in the plan, not laid on it.
//
// The city is ray-marched once per camera move (journey-city.ts); the liquid
// is a warped field and a fluid simulation (journey-liquid.ts). Raw WebGL2
// with no dependency, in its own chunk, loaded after the hero's words have
// painted; until it draws, and wherever it cannot, the page shows a CSS
// poster of the square in the same place (journey.css).
import { NW, STATIC_FS, SCENE_FS } from "./journey-city";
import { LIQ_FS, LIQN_FS, SIM_VS, SIM, BRIGHT, BLUR, FINAL } from "./journey-liquid";

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

/** The fixed step of the fluid and of the scene's clock. */
const DT = 1 / 60;
/** The composed still: the fountain at the height of its surge. */
const STILL_T = 8.5;
/** A ceiling on drawn pixels, so a 5K screen costs what a 1440p one does. */
const MAX_PIXELS = 5.4e6;
/** How many copy blocks the frame can hold calm at once. */
const CALM = 8;
/** The sharp city is marched in this many bands, one a frame, so no frame
 * pays for all of it. */
const BANDS = 8;

// Quality steps, best first. The loop measures its own frame time and steps
// down when it misses frames; it tries one step back up after a long calm.
const LEVELS = [
  { dpr: 2, sim: 160, dye: 640, steps: 160 },
  { dpr: 1.5, sim: 144, dye: 520, steps: 136 },
  { dpr: 1.25, sim: 128, dye: 420, steps: 116 },
  { dpr: 1, sim: 112, dye: 360, steps: 100 },
];
// While the camera moves the city is marched at one of these fractions of the
// frame, the largest the GPU keeps up with; at 1 there is no small city at all.
const MOVING = [0.4, 0.5, 0.6, 0.75, 1];

const ease = (x: number) => {
  x = Math.max(0, Math.min(1, x));
  return x * x * (3 - 2 * x);
};
const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
const clamp = (x: number, a: number, b: number) => Math.max(a, Math.min(b, x));
const norm = (v: number[]) => {
  const l = Math.hypot(...v);
  return v.map((x) => x / l);
};
const cross = (a: number[], b: number[]) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const surgeAt = (t: number) => {
  const s = t % 10;
  return ease((s - 7.4) / 0.9) * (1 - ease((s - 8.9) / 1.1));
};
function mulberry32(seed: number) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let r = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    r = (r + Math.imul(r ^ (r >>> 7), 61 | r)) ^ r;
    return ((r ^ (r >>> 14)) >>> 0) / 4294967296;
  };
}

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
 * march the city slowly enough to drag the whole page with it. Chrome hides
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

// People: walks across the plaza that never cross the basin, a few who stand
// at its rim, and some down the park's paths. Where each is depends only on
// the clock.
type Walker = { a: number[]; b: number[]; len: number; sp: number; ph: number; bri: number };
function cast(): Walker[] {
  const out: Walker[] = [];
  const rnd = mulberry32(11);
  const inPlaza = () => {
    const ang = rnd() * Math.PI * 2;
    const r = 13 + rnd() * 18;
    return [Math.cos(ang) * r, Math.sin(ang) * r];
  };
  while (out.length < 22) {
    const a = inPlaza();
    const b = inPlaza();
    const dx = b[0] - a[0];
    const dy = b[1] - a[1];
    const len = Math.hypot(dx, dy);
    if (len < 14) continue;
    const k = clamp(-(a[0] * dx + a[1] * dy) / (len * len), 0, 1);
    if (Math.hypot(a[0] + dx * k, a[1] + dy * k) < 12.5) continue;
    out.push({ a, b, len, sp: 0.85 + rnd() * 0.6, ph: rnd() * 2, bri: 0.55 + rnd() * 0.6 });
  }
  while (out.length < 28) {
    const ang = rnd() * Math.PI * 2;
    const r = 10.6 + rnd() * 3.4;
    const a = [Math.cos(ang) * r, Math.sin(ang) * r];
    out.push({ a, b: [a[0] + rnd() - 0.5, a[1] + rnd() - 0.5], len: 1, sp: 0.05 + rnd() * 0.05, ph: rnd() * 2, bri: 0.5 + rnd() * 0.5 });
  }
  while (out.length < NW) {
    // down one of the eight paths, keeping to one side of it
    const k = Math.floor(rnd() * 8);
    const d = [Math.cos((k * Math.PI) / 4), Math.sin((k * Math.PI) / 4)];
    const side = (rnd() - 0.5) * 2.4;
    const r0 = 38 + rnd() * 20;
    const r1 = r0 + 25 + rnd() * 30;
    const a = [d[0] * r0 - d[1] * side, d[1] * r0 + d[0] * side];
    const b = [d[0] * r1 - d[1] * side, d[1] * r1 + d[0] * side];
    out.push({ a, b, len: r1 - r0, sp: 0.9 + rnd() * 0.5, ph: rnd() * 2, bri: 0.6 + rnd() * 0.5 });
  }
  return out;
}

type Cam = { pos: number[]; fwd: number[]; right: number[]; up: number[]; tanf: number };
type Shot = { p: number[]; fx: number; pitch: number; tanf: number };

/**
 * Starts the journey on `canvas`, which fills `host`. It reads the page
 * itself: the header (.top), the hero's words (.hero-copy), the sections of
 * <main>, the closing headline and the footer, for where the acts fall and
 * what to hold calm.
 *
 * Returns null where WebGL2 is missing or the scene fails to build, and the
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
  type Progs = Record<"stat" | "scene" | "liq" | "liqn" | "bright" | "blur" | "final" | keyof typeof SIM, Prog>;
  let prog!: Progs;
  let vao: WebGLVertexArrayObject | null = null;
  function build() {
    const P = (fs: string) => program(gl, FULLSCREEN_VS, fs);
    const S = (fs: string) => program(gl, SIM_VS, fs);
    prog = {
      stat: P(STATIC_FS), scene: P(SCENE_FS), liq: P(LIQ_FS), liqn: P(LIQN_FS), bright: P(BRIGHT), blur: P(BLUR), final: P(FINAL),
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
  const walkers = cast();
  const walkArr = new Float32Array(NW * 4);
  const walkScr = new Float32Array(NW * 4);
  let level = cpu ? LEVELS.length - 1 : matchMedia("(pointer: coarse)").matches ? 1 : 0;
  let moving = 1;
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
  // The city: the sharp one and the camera it was marched for, how many of its
  // bands are done, the small one, and the cross-fade from small to sharp.
  const city = { sharp: "", pending: "", band: 0, fast: "", rest: 0, fade: 1, first: true };
  // Frame times, for stepping the quality and the moving city's size
  const times: number[] = [];
  const movingTimes: number[] = [];
  let changed = performance.now();
  const failed = new Set<number>();
  let R: {
    sharp: Target; fast: Target; scene: Target; liq: Target; liqn: Target;
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
    // on the CPU one small frame proves the scene works, and no more is drawn
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
      sharp: target(W, H, 3), fast: target(Math.ceil(W * 0.75), Math.ceil(H * 0.75), 3), scene: target(W, H),
      liq: target(lq[0], lq[1], 3, float32), liqn: target(lq[0], lq[1]),
      b1: pair(...q(4)), b2: pair(...q(8)), b3: pair(...q(16)),
      vel: pair(sw, sh), dye: pair(dw, dh), pres: pair(sw, sh), div: target(sw, sh), curl: target(sw, sh),
    };
    city.sharp = "";
    city.fast = "";
    city.pending = "";
    city.first = true;
    findPage();
    dirty = true;
  }

  // ---- scroll: where the page is, and what that asks of the picture
  function journey() {
    const vh = window.innerHeight;
    const ch = size.h;
    const y = window.scrollY;
    const lift = ease(y / (vh * 0.95));
    // the close: its headline travels the last screen of the page
    let down = 0;
    let headAtEnd = vh * 0.17;
    const head = page.closeH?.getBoundingClientRect();
    if (head) {
      const max = Math.max(0, document.documentElement.scrollHeight - vh);
      headAtEnd = Math.max(vh * 0.08, head.top + y - max);
      down = ease(1 - (head.top - headAtEnd) / (vh * 0.95));
    }
    // how far the liquid has closed over the square
    const cov = Math.min(ease((lift - 0.16) / 0.6), 1 - ease((down - 0.2) / 0.62));
    // The plume is a place on the page: the gap under the hero, and the gap
    // over the closing headline. It is hot while the camera is in it.
    const heroBottom = (page.hero?.getBoundingClientRect().bottom ?? vh - y) + y;
    const first = page.mid[0] ? content(page.mid[0]) : null;
    const upC = Math.min((heroBottom + (first ? first.top + y : heroBottom + vh * 0.3)) / 2, vh * 1.25);
    const upH = first ? Math.max(0, first.top + y - heroBottom) : vh * 0.3;
    const lastMid = page.mid.length ? content(page.mid[page.mid.length - 1]) : null;
    const downTop = lastMid ? lastMid.bottom : head ? head.top - vh * 0.4 : vh;
    const downC = ((head ? head.top : vh) + downTop) / 2 + y;
    const downH = Math.max(0, (head ? head.top : vh) - downTop);
    const hotUp = ease((lift - 0.1) / 0.36) * (1 - ease((y - (upC - vh * 0.41)) / (vh * 0.5)));
    const hotDown = ease(down / 0.3) * (1 - ease((down - 0.62) / 0.36));
    const hotBand = down > 0 ? [1 - (downC - y) / ch, Math.max(0.42, (downH / ch) * 1.05)] : [1 - (upC - y) / ch, Math.max(0.42, (upH / ch) * 1.05)];
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
    const soft = opts.light ? 170 : 150;
    hold(page.top?.getBoundingClientRect(), 36, 0);
    hold(page.heroCopy?.getBoundingClientRect(), soft, 10);
    for (const s of page.mid) {
      const box = s.getBoundingClientRect();
      if (box.bottom > 0 && box.top < ch) hold(content(s), soft, 10);
    }
    hold(head, soft, 26);
    hold(page.closeCta?.getBoundingClientRect(), soft, 14);
    hold(page.foot?.getBoundingClientRect(), 60, 0);
    return { y, vh, lift, down, cov, hot: Math.max(hotUp, down > 0 ? hotDown : 0), hotBand, bands, calm, feather, breath: ease((down - 0.55) / 0.45), headAtEnd };
  }
  type Journey = ReturnType<typeof journey>;
  // What the last frame read of the page, so the fluid's step does not read it again
  let seen: Journey | null = null;

  /** The pitch that puts a point at height `z`, `dist` away, at `ndc` up the frame. */
  const pitchFor = (camZ: number, z: number, dist: number, ndc: number, tanf: number) => Math.atan2(z - camZ, dist) - Math.atan(ndc * tanf);

  /** A shot from `p` that puts the fountain's foot `fx` across and `ndc` up the frame. */
  const aim = (p: number[], fx: number, ndc: number, tanf: number): Shot => ({ p, fx, pitch: pitchFor(p[2], 1.0, Math.hypot(p[0], p[1]), ndc, tanf), tanf });

  // The establishing view: from a rooftop south of the park, some 260 m back
  // and 46 m up, the fountain a small bright point in the city's heart. The
  // close comes back to it.
  function camera(j: Journey): Cam {
    const a = size.aspect;
    const wide = size.w >= 1080;
    let hero: Shot;
    let end: Shot;
    if (wide) {
      // the fountain stands right of centre, clear of the words; the copy has the left
      const copy = page.heroCopy?.getBoundingClientRect();
      const fx = copy ? clamp((copy.right + size.w * 0.15) / size.w, 0.6, 0.74) : 0.64;
      const p = [-34, -258, 46];
      hero = aim(p, fx, -0.42, 0.34);
      end = aim(p, 0.5, -0.46, 0.34);
    } else {
      // One column: the fountain crowns the first screen and the words start
      // under it; at the close it stands under the words, over the footer.
      const copyTop = page.heroCopy ? page.heroCopy.getBoundingClientRect().top + j.y : size.h * 0.46;
      const footTop = size.h - (page.foot?.getBoundingClientRect().height ?? 80);
      // lower and nearer than on a wide screen, so a narrow frame still has
      // sky over the roofs and the fountain is not lost
      const tanf = a > 0.9 ? 0.42 : 0.55;
      const p = a > 0.9 ? [-22, -220, 34] : [-8, -170, 22];
      hero = aim(p, 0.5, 1 - (2 * (copyTop - size.h * 0.045)) / size.h, tanf);
      end = aim(p, 0.5, 1 - (2 * (footTop - size.h * 0.07)) / size.h, tanf);
    }
    // lifted: toward the fountain and up its plume, looking into it
    const lift = (s: Shot): Shot => ({ p: [s.p[0] * 0.55, s.p[1] * 0.55, s.p[2] + 30], fx: lerp(s.fx, 0.5, 0.5), pitch: s.pitch + 0.5, tanf: s.tanf });
    const up = lift(hero);
    const up2 = lift(end);
    const mix = (x: Shot, y: Shot, k: number): Shot => ({ p: x.p.map((v, i) => lerp(v, y.p[i], k)), fx: lerp(x.fx, y.fx, k), pitch: lerp(x.pitch, y.pitch, k), tanf: lerp(x.tanf, y.tanf, k) });
    const k = j.down > 0 ? mix(up2, end, j.down) : mix(hero, up, j.lift);
    const head = Math.atan2(-k.p[1], -k.p[0]) + Math.atan((2 * k.fx - 1) * k.tanf * a);
    const fwd = norm([Math.cos(head) * Math.cos(k.pitch), Math.sin(head) * Math.cos(k.pitch), Math.sin(k.pitch)]);
    const right = norm(cross(fwd, [0, 0, 1]));
    return { pos: k.p, fwd, right, up: cross(right, fwd), tanf: k.tanf };
  }
  function project(cam: Cam, p: number[]) {
    const d = [p[0] - cam.pos[0], p[1] - cam.pos[1], p[2] - cam.pos[2]];
    const z = d[0] * cam.fwd[0] + d[1] * cam.fwd[1] + d[2] * cam.fwd[2];
    if (z <= 0.5) return null;
    const x = (d[0] * cam.right[0] + d[1] * cam.right[1] + d[2] * cam.right[2]) / (z * cam.tanf * size.aspect);
    const y = (d[0] * cam.up[0] + d[1] * cam.up[1] + d[2] * cam.up[2]) / (z * cam.tanf);
    return { u: x * 0.5 + 0.5, v: y * 0.5 + 0.5, dist: Math.hypot(...d), ppm: size.H / (2 * cam.tanf * z) };
  }

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
    // in the plume, light wells up from the fountain's side of the frame
    if (j.hot > 0.04) {
      const f = project(camera(j), [0, 0, 6]);
      const fx = f ? clamp(f.u, 0.15, 0.85) : 0.6;
      for (let k = 0; k < 2; k++) {
        const x = fx + 0.2 * Math.sin(t * 0.5 + k * 2.6) + 0.07 * Math.sin(t * 1.3 + k);
        const on = j.hot * (0.5 + 0.5 * Math.sin(t * 1.1 + k * 3.1) ** 2);
        splat(x, j.hotBand[0] - j.hotBand[1] * 0.7, 0.5 * Math.sin(t * 0.4 + k * 1.7) * dt * on, 1.6 * dt * on, 0.5 * dt * on, 0.6 * dt * on, 0.0012);
      }
    }
    if (ptr.moved) {
      // the cursor stirs the liquid and leaves a little light in it
      splat(ptr.x / size.w, 1 - ptr.y / size.h, (ptr.dx / size.w) * 9, (-ptr.dy / size.h) * 9, 0.05 * j.cov, 0.045 * j.cov, 0.0011);
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

  /** Marches the city for `cam` into `out`: all of it, or the rows from `y0` up for `rows`. */
  function march(out: Target, w: number, h: number, cam: Cam, quiet: number[], y0 = 0, rows = h) {
    if (rows < h) {
      gl.enable(gl.SCISSOR_TEST);
      gl.scissor(0, y0, w, rows);
    }
    pass(prog.stat, { fb: out.fb, w, h }, (a) => {
      a.f("u_cam", cam.pos[0], cam.pos[1], cam.pos[2]); a.f("u_fwd", cam.fwd[0], cam.fwd[1], cam.fwd[2]); a.f("u_right", cam.right[0], cam.right[1], cam.right[2]); a.f("u_up", cam.up[0], cam.up[1], cam.up[2]);
      a.f("u_tanf", cam.tanf); a.f("u_res", size.W, size.H); a.i("u_light", opts.light ? 1 : 0); a.f("u_pix", (2 * cam.tanf) / h); a.f("u_quiet", quiet[0]); a.f("u_qx", quiet[1]); a.i("u_steps", LEVELS[level].steps);
    });
    gl.disable(gl.SCISSOR_TEST);
  }

  /** Draws a frame. `whole` marches the sharp city at once (the first frame, and a proof on the CPU). */
  function draw(whole = false) {
    if (!R || lost) return;
    const r = R;
    const { W, H } = size;
    const j = journey();
    seen = j;
    const cam = camera(j);
    const sceneOn = j.cov < 0.999;
    const surge = surgeAt(t);
    // the copy's side of the wide hero is kept quiet, up to the copy's right
    // edge; the close is symmetrical
    const copyR = page.heroCopy?.getBoundingClientRect().right ?? size.w * 0.4;
    const quiet = size.w >= 1080 && j.down <= 0 ? [1, Math.round(((copyR + 24) / size.w) * 100) / 100] : [0, 0];
    let sharpness = 1;
    if (sceneOn) {
      const key = [cam.pos, cam.fwd, cam.tanf, opts.light ? 1 : 0, quiet, level].flat().map((v) => v.toFixed(4)).join(",");
      const scale = MOVING[moving];
      if (key === city.sharp) {
        city.fade = Math.min(1, city.fade + 0.1);
      } else if (whole || city.first || scale >= 1) {
        march(r.sharp, W, H, cam, quiet);
        city.sharp = key;
        city.fade = 1;
        city.first = false;
      } else {
        // The camera has moved. March the small city for where it is now; once
        // it rests, march the sharp one a band a frame, then fade over to it.
        // The scale is part of the key: a buffer marched at another scale
        // holds the picture in a different corner of the target.
        const fastKey = `${key}@${scale}`;
        if (fastKey !== city.fast) {
          march(r.fast, Math.round(W * scale), Math.round(H * scale), cam, quiet);
          city.fast = fastKey;
          city.rest = 0;
        } else city.rest++;
        city.fade = 0;
        if (city.rest >= 2) {
          if (city.pending !== key) {
            city.pending = key;
            city.band = 0;
          }
          const rows = Math.ceil(H / BANDS);
          // From the first band on, the sharp buffer is a mix of two cameras,
          // so it stands for neither until every band is in.
          city.sharp = "";
          march(r.sharp, W, H, cam, quiet, city.band * rows, Math.min(rows, H - city.band * rows));
          city.band++;
          if (city.band >= BANDS) {
            city.sharp = key;
            city.pending = "";
          }
        }
      }
      sharpness = city.sharp === key ? city.fade : 0;
    }
    const focus = project(cam, [0, 0, 6]);
    pass(prog.liq, r.liq, (a) => {
      a.tex("u_dye", r.dye.read.texs[0]); a.f("u_dTexel", 1 / r.dye.read.w, 1 / r.dye.read.h); a.f("u_t", t); a.f("u_scroll", j.y / size.h); a.f("u_aspect", size.aspect); a.f("u_cov", j.cov); a.f("u_hot", j.hot);
      a.f("u_focus", clamp(focus?.u ?? 0.6, 0.1, 0.9), clamp(focus?.v ?? 0.4, 0.1, 0.9)); a.f("u_hotBand", j.hotBand[0], j.hotBand[1]); a.fv("u_bands", 4, j.bands);
    });
    pass(prog.liqn, r.liqn, (a) => { a.tex("u_l0", r.liq.texs[0]); a.tex("u_l2", r.liq.texs[2]); a.f("u_tx", 1 / r.liq.w, 1 / r.liq.h); a.i("uMode", float32 ? 0 : 1); });
    if (sceneOn) {
      walkers.forEach((w, i) => {
        const u = ((t * w.sp) / w.len + w.ph) % 2;
        const k = u < 1 ? u : 2 - u;
        walkArr.set([w.a[0] + (w.b[0] - w.a[0]) * k, w.a[1] + (w.b[1] - w.a[1]) * k, w.bri * (0.9 + 0.1 * Math.sin(t * 2.1 + i)), 0], i * 4);
      });
      walkScr.fill(0);
      for (let i = 0; i < NW; i++) {
        const q = project(cam, [walkArr[i * 4], walkArr[i * 4 + 1], 0.8]);
        if (q && q.dist > 5) walkScr.set([q.u * W, q.v * H, q.ppm, q.dist], i * 4);
      }
    }
    const fastScale = MOVING[moving];
    pass(prog.scene, r.scene, (a) => {
      a.f("u_cam", cam.pos[0], cam.pos[1], cam.pos[2]); a.f("u_fwd", cam.fwd[0], cam.fwd[1], cam.fwd[2]); a.f("u_right", cam.right[0], cam.right[1], cam.right[2]); a.f("u_up", cam.up[0], cam.up[1], cam.up[2]);
      a.f("u_tanf", cam.tanf); a.f("u_res", W, H); a.i("u_light", opts.light ? 1 : 0); a.f("u_pix", (2 * cam.tanf) / H); a.f("u_quiet", quiet[0]); a.f("u_qx", quiet[1]);
      a.tex("u_s0", r.sharp.texs[0]); a.tex("u_s1", r.sharp.texs[1]); a.tex("u_s2", r.sharp.texs[2]);
      a.tex("u_f0", r.fast.texs[0]); a.tex("u_f1", r.fast.texs[1]); a.tex("u_f2", r.fast.texs[2]);
      a.tex("u_l0", r.liq.texs[0]); a.tex("u_l1", r.liq.texs[1]); a.tex("u_ln", r.liqn.texs[0]);
      a.f("u_fScale", Math.round(W * fastScale) / r.fast.w, Math.round(H * fastScale) / r.fast.h); a.f("u_sharp", sharpness);
      // the liquid's relief, by the size of its own grid
      a.f("u_rel", (r.liq.h / 900) * (opts.light ? 160 : 260) * (r.liq.h > size.h * 1.2 ? 0.6 : 1));
      a.f("u_t", t); a.f("u_surge", surge); a.f("u_scene", sceneOn ? 1 : 0); a.f("u_breath", j.breath);
      a.f("u_fh", 12.0 + 4.6 * surge + 0.25 * Math.sin(t * 2.3)); a.f("u_fr", 6.4 + 0.8 * surge); a.f("u_dpr", size.dpr);
      a.fv("u_walk", 4, walkArr); a.fv("u_walkScr", 4, walkScr);
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
    // more to do without the clock: the sharp city is still on its way
    dirty = sceneOn && sharpness < 1;
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

  function adapt(ms: number, wasMoving: boolean) {
    const now = performance.now();
    // the moving city's size: the largest that keeps the frame
    if (wasMoving) {
      movingTimes.push(ms);
      if (movingTimes.length >= 20) {
        // a scroll is short, so a few missed frames in it are already a stutter:
        // step down on the slow fifth, not the middle
        const sorted = [...movingTimes].sort((a, b) => a - b);
        if (sorted[15] > 18.5 && moving > 0) moving--;
        else if (sorted[15] < 11 && moving < MOVING.length - 1) moving++;
        movingTimes.length = 0;
      }
      return;
    }
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
    const before = city.fast;
    draw();
    if (on && ++frames > 30) adapt(ms, city.fast !== before);
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
    draw(true);
  } catch {
    destroy();
    return null;
  }
  // On a software renderer the one small frame above proves the scene works;
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
