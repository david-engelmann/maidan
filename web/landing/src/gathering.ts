// The gathering: the hero's living picture, and its echo at the close.
//
// A maidan is open ground where everyone gathers. Tens of thousands of
// lights stand across a dark field, agents in ember and people in warm
// white. They join rivers that curl in toward one point, rise, and become
// the Maidan mark; it breathes, bursts out along its rays, settles back over
// the field to work, and gathers again.
//
// Every light's place is a function of its id and the time, worked out in
// the vertex shader, so nothing is simulated on the CPU and any moment can
// be drawn on its own: the still under reduced motion is one such frame.
// Raw WebGL2 with no dependency, in its own chunk, loaded after the hero's
// words have painted; until it draws, the page shows a CSS poster of the
// mark in the same place (gathering.css, .gather__mark).

export type Variant = "hero" | "close";
export type Options = { light: boolean; still: boolean; paused: boolean };
export type State = "running" | "paused" | "still" | "away" | "fallback" | "software";
export type Handle = { update(next: Partial<Options>): void; destroy(): void };

type GL = WebGL2RenderingContext;
type Prog = { p: WebGLProgram; u: (name: string) => WebGLUniformLocation | null };
type Target = { tex: WebGLTexture; fbo: WebGLFramebuffer; w: number; h: number };

/** One cycle of gathering, holding, bursting and settling, in seconds. */
const PERIOD = 30;
/** Where each picture starts in the cycle: the hero mid-gather, so the mark
 * completes in front of the reader; the close already whole. */
const START: Record<Variant, number> = { hero: 0.4 * PERIOD, close: 0.52 * PERIOD };
/** The composed still: everyone arrived, the mark whole. */
const STILL = 0.63 * PERIOD;

// Quality steps, best first. The loop measures its own frame time and steps
// down when it misses frames; it tries one step back up after a long calm.
const LEVELS = [
  { dpr: 2, n: 1, trails: true },
  { dpr: 1.5, n: 0.8, trails: true },
  { dpr: 1.25, n: 0.62, trails: false },
  { dpr: 1, n: 0.45, trails: false },
  { dpr: 0.75, n: 0.3, trails: false },
];
/** Lights at full quality, by picture and by how wide it is. */
function population(variant: Variant, cssWidth: number): number {
  if (variant === "close") return cssWidth < 720 ? 12000 : 22000;
  return cssWidth < 720 ? 26000 : cssWidth < 1080 ? 40000 : 60000;
}
/** A ceiling on drawn pixels, so a 5K screen costs what a 4K one does. */
const MAX_PIXELS = 4.2e6;

const HASH = `
uint hh(uint x) { x ^= x >> 16; x *= 0x7feb352du; x ^= x >> 15; x *= 0x846ca68bu; x ^= x >> 16; return x; }
float rnd(uint id, uint k) { return float(hh(id * 0x9E3779B9u ^ hh(k + 0x632BE5ABu))) * (1.0 / 4294967296.0); }
vec3 hash33(vec3 p) { p = fract(p * vec3(0.1031, 0.1030, 0.0973)); p += dot(p, p.yxz + 33.33); return fract((p.xxy + p.yxx) * p.zyx) * 2.0 - 1.0; }
float noise3(vec3 p) {
  vec3 i = floor(p); vec3 f = fract(p); vec3 u = f * f * (3.0 - 2.0 * f);
  return mix(mix(mix(dot(hash33(i), f), dot(hash33(i + vec3(1,0,0)), f - vec3(1,0,0)), u.x),
                 mix(dot(hash33(i + vec3(0,1,0)), f - vec3(0,1,0)), dot(hash33(i + vec3(1,1,0)), f - vec3(1,1,0)), u.x), u.y),
             mix(mix(dot(hash33(i + vec3(0,0,1)), f - vec3(0,0,1)), dot(hash33(i + vec3(1,0,1)), f - vec3(1,0,1)), u.x),
                 mix(dot(hash33(i + vec3(0,1,1)), f - vec3(0,1,1)), dot(hash33(i + vec3(1,1,1)), f - vec3(1,1,1)), u.x), u.y), u.z);
}
`;

// The mark's blade (docs/assets/maidan-mark.svg), centred, y up, at unit
// radius; the other seven are this one turned by multiples of 45 degrees.
const MARK = `
const vec2 B0 = vec2(6.95, -0.85) / 27.8;
const vec2 B1 = vec2(16.07, -5.53) / 27.8;
const vec2 B2 = vec2(27.65, -3.70) / 27.8;
const vec2 B3 = vec2(27.74, -2.95) / 27.8;
const vec2 B4 = vec2(16.69, 3.24) / 27.8;
float triA(vec2 a, vec2 b, vec2 c) { return abs((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)); }
vec2 inTri(vec2 a, vec2 b, vec2 c, float u, float v) { if (u + v > 1.0) { u = 1.0 - u; v = 1.0 - v; } return a + u * (b - a) + v * (c - a); }
// A point of the mark: most fill a blade, some walk its outline so the
// shape reads crisp, and the tip edge gathers a bright point.
vec2 markPoint(uint id, float r0, float r1, float r2, float r3, out float along) {
  float k = float(id % 8u);
  vec2 p;
  if (r3 < 0.28) {
    float t = r0 * 5.0;
    float e = floor(t);
    float f = fract(t);
    vec2 a = e < 1.0 ? B0 : e < 2.0 ? B1 : e < 3.0 ? B2 : e < 4.0 ? B3 : B4;
    vec2 b = e < 1.0 ? B1 : e < 2.0 ? B2 : e < 3.0 ? B3 : e < 4.0 ? B4 : B0;
    if (e > 1.5 && e < 2.5) f = r0;
    p = mix(a, b, f);
  } else {
    float a1 = triA(B0, B1, B2), a2 = triA(B0, B2, B3), a3 = triA(B0, B3, B4);
    float s = r2 * (a1 + a2 + a3);
    p = s < a1 ? inTri(B0, B1, B2, r0, r1) : s < a1 + a2 ? inTri(B0, B2, B3, r0, r1) : inTri(B0, B3, B4, r0, r1);
  }
  along = length(p);
  float an = k * 0.78539816;
  float c = cos(an), sn = sin(an);
  return vec2(c * p.x - sn * p.y, sn * p.x + c * p.y);
}
`;

// Where a light is at time t. Shared by the points and by the trails, which
// draw each light's step from t - dt to t.
const PLACE = `
uniform float uR;
uniform vec3 uM, uRight, uUp;
const float P = ${PERIOD.toFixed(1)};
const float LANES = 14.0;
const float TAU = 6.2831853;
const float KAPPA = 0.62;
const float RMAX = 64.0;

// A river is one fixed curve, a log spiral into the centre: a light joins it
// wherever it stands and follows it in.
float river(float lane, float r) { return (lane + 0.5) / LANES * TAU + KAPPA * log(RMAX / r); }

vec3 fieldPoint(uint s, out float lane) {
  float th = rnd(s, 1u) * TAU;
  float rr = 2.4 * pow(26.0, rnd(s, 2u));
  lane = floor(mod((th - KAPPA * log(RMAX / rr)) / TAU * LANES, LANES));
  return vec3(cos(th) * rr, 0.0, sin(th) * rr);
}

vec3 journey(vec3 O, float lane, vec3 T, float q, float gate, float spin, float jr) {
  float x = clamp(q / 0.72, 0.0, 1.0);
  float w = x * x * (3.0 - 2.0 * x);
  float ro = max(length(O.xz), 0.01);
  float th0 = atan(O.z, O.x);
  float r = ro * pow(gate / ro, w);
  float d0 = river(lane, ro) - th0;
  d0 -= TAU * floor((d0 + 3.14159265) / TAU);
  // Tributaries: each starts off its river and joins it in the first stretch
  float th = river(lane, r) - d0 * (1.0 - smoothstep(0.0, 0.3, x)) + (jr - 0.5) * 0.05 * (1.0 - 0.6 * w);
  vec3 g = vec3(cos(th) * r, 0.04 + 0.05 * jr, sin(th) * r) + uM * vec3(1.0, 0.0, 1.0);
  float u = smoothstep(0.62, 1.0, q);
  float ue = u * u * (3.0 - 2.0 * u);
  vec3 p = mix(g, T, ue);
  p.y += sin(u * 3.14159) * (0.6 + 1.2 * jr);
  // The rise turns about the column and unwinds as each light arrives
  float a = (1.0 - ue) * ue * spin;
  vec2 c = p.xz - uM.xz;
  p.xz = uM.xz + mat2(cos(a), sin(a), -sin(a), cos(a)) * c;
  return p;
}

vec3 place(uint id, float t, out float lit, out float held, out float kind, out float trail) {
  kind = step(rnd(id, 3u), 0.42);
  bool stream = rnd(id, 4u) < 0.3;
  float j0 = rnd(id, 5u), j1 = rnd(id, 6u), j2 = rnd(id, 7u), j3 = rnd(id, 8u), j4 = rnd(id, 9u);
  float j5 = rnd(id, 10u), j6 = rnd(id, 11u), j7 = rnd(id, 12u), j8 = rnd(id, 13u);
  float along;
  vec2 mp = markPoint(id, j0, j1, j2, j3, along);
  float breathe = 1.0 + 0.03 * sin(t * 0.6);
  vec3 T = uM + (uRight * mp.x + uUp * mp.y) * uR * breathe + cross(uRight, uUp) * (j4 - 0.5) * 0.14 * uR;
  T += uR * 0.01 * vec3(sin(t * 1.7 + j0 * 40.0), cos(t * 1.3 + j1 * 40.0), sin(t * 1.1 + j2 * 40.0));
  float cyc = floor(t / P);
  float ph = fract(t / P);
  float lane;
  vec3 p;
  float rest = 0.0;
  held = 0.0;
  trail = 1.0;
  if (stream) {
    // A share of everyone is always arriving, whatever the mark is doing
    float s = t / (14.0 + 8.0 * j5) + j6 * 9.0;
    float sq = fract(s);
    vec3 O = fieldPoint(hh(id ^ uint(floor(s)) * 0x2545F491u), lane);
    p = journey(O, lane, uM * vec3(1.0, 0.0, 1.0), sq * 0.7, 0.3 + 0.8 * j7, 3.0, j8);
    lit = 0.9 * smoothstep(0.0, 0.1, sq) * (1.0 - smoothstep(0.86, 1.0, sq)) * (0.5 + smoothstep(0.08, 0.3, sq));
  } else {
    vec3 O = fieldPoint(hh(id ^ uint(cyc) * 0x2545F491u), lane);
    float q = clamp((ph - 0.02 - 0.24 * j5) / (0.3 + 0.05 * j6), 0.0, 1.0);
    if (ph < 0.72) {
      p = journey(O, lane, T, q, 0.3 + 0.9 * j7, 4.0 + 2.5 * j8, j8);
      held = smoothstep(0.9, 1.0, q);
      lit = mix(0.5, 1.25, smoothstep(0.5, 1.0, q)) * (1.0 + 0.9 * smoothstep(0.05, 0.3, q) * (1.0 - smoothstep(0.6, 0.9, q)));
      rest = 1.0 - smoothstep(0.0, 0.06, q);
      // Walking lights draw ribbons; the rise does not, or it would veil the mark
      trail = 1.0 - smoothstep(0.5, 0.66, q);
    } else {
      // The burst: every blade slides out along its own axis and stretches
      // into a spoke of light, eight of them, then drifts down to the field
      float qd = clamp((ph - 0.72 - 0.025 * j6 - 0.02 * along) / 0.22, 0.0, 1.0);
      float an = float(id % 8u) * 0.78539816 - 0.1197;
      vec3 ray = uRight * cos(an) + uUp * sin(an);
      float reach = uR * (0.2 + 0.5 * j7);
      float q1 = clamp(qd / 0.34, 0.0, 1.0);
      float e1 = 1.0 - pow(1.0 - q1, 3.0);
      vec3 B = T + ray * reach * e1;
      float laneN;
      vec3 On = fieldPoint(hh(id ^ uint(cyc + 1.0) * 0x2545F491u), laneN);
      float q2 = smoothstep(0.34, 1.0, qd);
      vec3 Bend = T + ray * reach;
      vec3 c1 = Bend + ray * uR * 0.7 - vec3(0.0, 0.3, 0.0);
      vec3 c2 = On + vec3(0.0, 1.6 + 2.0 * j8, 0.0);
      float i2 = 1.0 - q2;
      vec3 D = i2 * i2 * i2 * Bend + 3.0 * i2 * i2 * q2 * c1 + 3.0 * i2 * q2 * q2 * c2 + q2 * q2 * q2 * On;
      p = qd < 0.34 ? B : D;
      held = 1.0 - smoothstep(0.0, 0.1, qd);
      float flash = step(0.0001, qd) * exp(-qd * 7.0);
      lit = mix(1.25, 0.45, smoothstep(0.25, 0.95, qd)) + 1.1 * flash * (0.4 + along);
      rest = smoothstep(0.9, 1.0, qd);
      trail = qd < 0.34 ? 1.0 : 0.35 * (1.0 - q2);
    }
  }
  // At work in the field, everyone moves about a little
  p.xz += rest * 0.22 * vec2(sin(t * 0.37 + j0 * 30.0), cos(t * 0.29 + j1 * 30.0));
  // A heartbeat runs out along the rays while the mark is whole
  float pulse = pow(0.5 + 0.5 * sin(along * 11.0 - t * 2.1), 12.0);
  lit *= 1.0 + held * (0.15 + 1.6 * pulse);
  return p;
}
`;

const POINTS_VS = `#version 300 es
precision highp float;
uniform mat4 uVP;
uniform float uTime, uDt, uFocus, uAperture, uPx, uLight, uBright, uHeld, uStride;
uniform vec2 uRes;
uniform vec4 uPtr;
uniform vec3 uTrail[8];
out vec3 vCol;
out float vSharp;
${HASH}
${MARK}
${PLACE}
vec3 tint(float kind) {
  vec3 agent = uLight > 0.5 ? vec3(0.0, 0.66, 1.15) : vec3(1.0, 0.2, 0.02) * 1.4;
  vec3 person = uLight > 0.5 ? vec3(0.6, 0.6, 0.58) : vec3(1.0, 0.93, 0.84);
  return mix(person, agent, kind);
}
// The hand parts the lights: a push away from the pointer and from the
// last few places it passed, in screen space.
vec2 part(vec2 sp, out float excite) {
  vec2 push = vec2(0.0);
  excite = 0.0;
  vec2 d = sp - uPtr.xy;
  float L = length(d) + 1e-3;
  float f = uPtr.z * exp(-L * L / (uPtr.w * uPtr.w));
  push += d / L * f * uPtr.w * 0.6;
  excite += f;
  for (int i = 0; i < 8; i++) {
    vec2 e = sp - uTrail[i].xy;
    float Le = length(e) + 1e-3;
    float rad = uPtr.w * 0.75;
    float g = uTrail[i].z * exp(-Le * Le / (rad * rad));
    push += e / Le * g * rad * 0.45;
    excite += g * 0.4;
  }
  return sp + push;
}
void main() {
#ifdef LINES
  uint id = uint(gl_VertexID / 2) * uint(uStride);
  float t = uTime - float(gl_VertexID % 2) * uDt;
#else
  uint id = uint(gl_VertexID) * uint(uStride);
  float t = uTime;
#endif
  float lit, held, kind, trail;
  vec3 p = place(id, t, lit, held, kind, trail);
  vec4 clip = uVP * vec4(p, 1.0);
  if (clip.w < 0.2) { gl_Position = vec4(2.0, 2.0, 2.0, 1.0); vCol = vec3(0.0); vSharp = 1.0; return; }
  float vz = clip.w;
  float excite;
  vec2 sp = part((clip.xy / clip.w * 0.5 + 0.5) * uRes, excite);
  clip.xy = (sp / uRes * 2.0 - 1.0) * clip.w;
  float atm = mix(1.0, exp(-max(vz - uFocus, 0.0) * 0.022), 0.85);
  // The mark is the brightest thing in the picture
  vec3 col = tint(kind) * lit * uBright * mix(1.0, uHeld, held) * atm * (1.0 + excite * 2.5);
#ifdef LINES
  // A held light barely moves, so its trail would only pile up on the mark
  vCol = col * trail * 0.14;
  vSharp = 1.0;
#else
  // Depth of field: lights nearer than the mark open into soft discs whose
  // light spreads over their area
  float j4 = rnd(id, 9u);
  float base = uPx * (1.0 + 1.1 * j4 * j4) * (uLight > 0.5 ? 1.25 : 1.0);
  float coc = uAperture * uRes.y * (vz < uFocus ? (uFocus - vz) / vz : 0.12 * (vz - uFocus) / vz);
  float size = max(base, coc);
  gl_PointSize = min(size, 120.0 * uPx);
  vSharp = clamp(1.0 - (size - base) / (5.0 * uPx), 0.0, 1.0);
  vCol = col * pow(base / size, 1.35);
#endif
  gl_Position = clip;
}`;

const POINTS_FS = `#version 300 es
precision highp float;
in vec3 vCol;
in float vSharp;
out vec4 o;
void main() {
#ifdef LINES
  o = vec4(vCol, 1.0);
#else
  vec2 c = gl_PointCoord * 2.0 - 1.0;
  float d = dot(c, c);
  if (d > 1.0) discard;
  float g = exp(-d * 4.5);
  float disc = smoothstep(1.0, 0.7, d) * (0.65 + 0.35 * smoothstep(0.2, 0.92, d));
  o = vec4(vCol * mix(disc, g, vSharp), 1.0);
#endif
}`;

const TRI_VS = `#version 300 es
out vec2 vUv;
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  vUv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;

// The ground and the low light over the gathering, then the trails on top.
const GROUND_FS = `#version 300 es
precision highp float;
in vec2 vUv;
uniform mat4 uInv;
uniform vec3 uCam, uM;
uniform float uGlow, uLight, uTrails;
uniform sampler2D uAcc;
out vec4 o;
${HASH}
void main() {
  vec2 n = vUv * 2.0 - 1.0;
  vec4 a = uInv * vec4(n, -1.0, 1.0);
  vec4 b = uInv * vec4(n, 1.0, 1.0);
  vec3 ro = a.xyz / a.w;
  vec3 rd = normalize(b.xyz / b.w - ro);
  vec3 warm = uLight > 0.5 ? vec3(0.0, 0.2, 0.38) : vec3(1.0, 0.34, 0.09);
  vec3 toM = normalize(uM - uCam);
  float az = max(dot(normalize(rd.xz), normalize(toM.xz)), 0.0);
  vec3 col = warm * exp(-abs(rd.y) * 55.0) * 0.03 * pow(az, 30.0) * (0.3 + uGlow);
  col += warm * 0.006 * pow(az, 12.0) * exp(-abs(rd.y) * 9.0) * (0.25 + uGlow);
  if (rd.y < 0.0) {
    float t = -ro.y / rd.y;
    vec3 p = ro + rd * t;
    float d = length(p.xz - uM.xz);
    float pool = 1.0 / (1.0 + d * d * 0.35);
    float tex = 0.7 + 0.3 * noise3(vec3(p.xz * 1.3, 0.0)) + 0.15 * noise3(vec3(p.xz * 6.0, 1.0));
    col += warm * (0.05 * pool * (0.25 + uGlow) + 0.002 / (1.0 + d * 0.06)) * tex * exp(-t * 0.006);
  }
  if (uTrails > 0.5) col += texture(uAcc, vUv).rgb;
  o = vec4(col, 1.0);
}`;

const FADE_FS = `#version 300 es
precision highp float;
in vec2 vUv;
uniform sampler2D uSrc;
uniform float uK;
out vec4 o;
void main() { o = vec4(texture(uSrc, vUv).rgb * uK, 1.0); }`;

const DOWN_FS = `#version 300 es
precision highp float;
in vec2 vUv;
uniform sampler2D uSrc;
uniform vec2 uTexel;
out vec4 o;
void main() {
  vec2 t = uTexel;
  vec3 r = texture(uSrc, vUv).rgb * 0.125;
  r += (texture(uSrc, vUv + t * vec2(-2, 2)).rgb + texture(uSrc, vUv + t * vec2(2, 2)).rgb + texture(uSrc, vUv + t * vec2(-2, -2)).rgb + texture(uSrc, vUv + t * vec2(2, -2)).rgb) * 0.03125;
  r += (texture(uSrc, vUv + t * vec2(0, 2)).rgb + texture(uSrc, vUv + t * vec2(-2, 0)).rgb + texture(uSrc, vUv + t * vec2(2, 0)).rgb + texture(uSrc, vUv + t * vec2(0, -2)).rgb) * 0.0625;
  r += (texture(uSrc, vUv + t * vec2(-1, 1)).rgb + texture(uSrc, vUv + t * vec2(1, 1)).rgb + texture(uSrc, vUv + t * vec2(-1, -1)).rgb + texture(uSrc, vUv + t * vec2(1, -1)).rgb) * 0.125;
  o = vec4(r, 1.0);
}`;

const UP_FS = `#version 300 es
precision highp float;
in vec2 vUv;
uniform sampler2D uSrc;
uniform vec2 uTexel;
out vec4 o;
void main() {
  vec2 t = uTexel;
  vec3 r = texture(uSrc, vUv).rgb * 4.0;
  r += (texture(uSrc, vUv + vec2(-t.x, 0)).rgb + texture(uSrc, vUv + vec2(t.x, 0)).rgb + texture(uSrc, vUv + vec2(0, t.y)).rgb + texture(uSrc, vUv + vec2(0, -t.y)).rgb) * 2.0;
  r += texture(uSrc, vUv + vec2(-t.x, -t.y)).rgb + texture(uSrc, vUv + vec2(t.x, -t.y)).rgb + texture(uSrc, vUv + vec2(-t.x, t.y)).rgb + texture(uSrc, vUv + vec2(t.x, t.y)).rgb;
  o = vec4(r / 16.0, 1.0);
}`;

// Onto the page. On dark the light is added to the paper; on paper the same
// buffer is read as ink the paper absorbs. The canvas is transparent and the
// page's own ground and grain show through, so its edges never show. Behind
// the words the light dims, and on a wide screen the words' side of the
// picture falls away.
const COMPOSITE_FS = `#version 300 es
precision highp float;
in vec2 vUv;
uniform sampler2D uScene, uBloom;
uniform float uBloomK, uExposure, uLight, uTime, uFeather;
uniform float uShadeS[10];
uniform vec3 uPaper;
uniform vec4 uShade[10];
uniform int uShadeN;
uniform vec2 uSide;
uniform float uFoot;
out vec4 o;
float hash(vec2 p) { vec3 p3 = fract(vec3(p.xyx) * 0.1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
float box(vec2 p, vec4 r) { vec2 c = (r.xy + r.zw) * 0.5; vec2 h = (r.zw - r.xy) * 0.5; vec2 d = abs(p - c) - h; return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0); }
void main() {
  vec3 l = texture(uScene, vUv).rgb + texture(uBloom, vUv).rgb * uBloomK;
  vec2 px = gl_FragCoord.xy;
  float m = 0.0;
  for (int i = 0; i < 10; i++) {
    if (i >= uShadeN) break;
    m = max(m, uShadeS[i] * (1.0 - smoothstep(0.0, uFeather, box(px, uShade[i]))));
  }
  l *= 1.0 - m;
  if (uSide.y > 0.0) l *= mix(0.22, 1.0, smoothstep(uSide.x - uSide.y, uSide.x + uSide.y * 0.5, px.x));
  l *= smoothstep(0.0, uFoot, px.y);
  float dither = (hash(px + fract(uTime) * 517.0) - 0.5) / 255.0;
  if (uLight > 0.5) {
    vec3 T = exp(-l * uExposure);
    float lo = min(T.r, min(T.g, T.b));
    o = vec4(uPaper * (T - lo) + dither, 1.0 - lo + dither);
  } else {
    // A filmic curve on the brightest channel keeps embers orange however
    // many pile up, and lets only the hottest cores go white
    float mx = max(l.r, max(l.g, l.b));
    float tm = 1.0 - exp(-1.2 * mx);
    vec3 c = l * (tm / max(mx, 1e-5));
    c = mix(c, vec3(tm), 0.35 * smoothstep(0.6, 1.0, tm));
    o = vec4(c + dither, max(c.r, max(c.g, c.b)) + dither);
  }
  o = clamp(o, 0.0, 1.0);
  o.rgb = min(o.rgb, vec3(o.a));
}`;

function program(gl: GL, vs: string, fs: string): Prog {
  const shader = (type: number, src: string) => {
    const s = gl.createShader(type);
    if (!s) throw new Error("shader");
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS) && !gl.isContextLost()) throw new Error(gl.getShaderInfoLog(s) ?? "shader");
    return s;
  };
  const p = gl.createProgram();
  if (!p) throw new Error("program");
  gl.attachShader(p, shader(gl.VERTEX_SHADER, vs));
  gl.attachShader(p, shader(gl.FRAGMENT_SHADER, fs));
  gl.linkProgram(p);
  if (!gl.getProgramParameter(p, gl.LINK_STATUS) && !gl.isContextLost()) throw new Error(gl.getProgramInfoLog(p) ?? "link");
  const cache = new Map<string, WebGLUniformLocation | null>();
  return {
    p,
    u: (name) => {
      if (!cache.has(name)) cache.set(name, gl.getUniformLocation(p, name));
      return cache.get(name) ?? null;
    },
  };
}

function target(gl: GL, w: number, h: number, hdr: boolean): Target {
  const tex = gl.createTexture();
  const fbo = gl.createFramebuffer();
  if (!tex || !fbo) throw new Error("target");
  gl.bindTexture(gl.TEXTURE_2D, tex);
  if (hdr) gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
  else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  gl.clearColor(0, 0, 0, 1);
  gl.clear(gl.COLOR_BUFFER_BIT);
  return { tex, fbo, w, h };
}

// Column-major 4x4: just enough for one camera.
type M4 = Float32Array;
function perspective(fovy: number, aspect: number, near: number, far: number): M4 {
  const f = 1 / Math.tan(fovy / 2);
  const m = new Float32Array(16);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = (far + near) / (near - far);
  m[11] = -1;
  m[14] = (2 * far * near) / (near - far);
  return m;
}
function lookAt(eye: number[], at: number[]): M4 {
  const z = norm([eye[0] - at[0], eye[1] - at[1], eye[2] - at[2]]);
  const x = norm([z[2], 0, -z[0]]);
  const y = [z[1] * x[2] - z[2] * x[1], z[2] * x[0] - z[0] * x[2], z[0] * x[1] - z[1] * x[0]];
  const m = new Float32Array(16);
  m[0] = x[0]; m[4] = x[1]; m[8] = x[2];
  m[1] = y[0]; m[5] = y[1]; m[9] = y[2];
  m[2] = z[0]; m[6] = z[1]; m[10] = z[2];
  m[12] = -(x[0] * eye[0] + x[1] * eye[1] + x[2] * eye[2]);
  m[13] = -(y[0] * eye[0] + y[1] * eye[1] + y[2] * eye[2]);
  m[14] = -(z[0] * eye[0] + z[1] * eye[1] + z[2] * eye[2]);
  m[15] = 1;
  return m;
}
function mul(a: M4, b: M4): M4 {
  const o = new Float32Array(16);
  for (let c = 0; c < 4; c++)
    for (let r = 0; r < 4; r++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
      o[c * 4 + r] = s;
    }
  return o;
}
function project(m: M4, p: number[]): [number, number] {
  const w = m[3] * p[0] + m[7] * p[1] + m[11] * p[2] + m[15];
  return [(m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12]) / w, (m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13]) / w];
}
function invert(m: M4): M4 {
  // The camera has no roll and a pure perspective, so a general inverse is
  // the simplest honest answer; 16 cofactors, once per frame.
  const a = m;
  const inv = new Float32Array(16);
  inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15] + a[9] * a[7] * a[14] + a[13] * a[6] * a[11] - a[13] * a[7] * a[10];
  inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15] - a[8] * a[7] * a[14] - a[12] * a[6] * a[11] + a[12] * a[7] * a[10];
  inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15] + a[8] * a[7] * a[13] + a[12] * a[5] * a[11] - a[12] * a[7] * a[9];
  inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14] - a[8] * a[6] * a[13] - a[12] * a[5] * a[10] + a[12] * a[6] * a[9];
  inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15] - a[9] * a[3] * a[14] - a[13] * a[2] * a[11] + a[13] * a[3] * a[10];
  inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15] + a[8] * a[3] * a[14] + a[12] * a[2] * a[11] - a[12] * a[3] * a[10];
  inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15] - a[8] * a[3] * a[13] - a[12] * a[1] * a[11] + a[12] * a[3] * a[9];
  inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14] + a[8] * a[2] * a[13] + a[12] * a[1] * a[10] - a[12] * a[2] * a[9];
  inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15] + a[5] * a[3] * a[14] + a[13] * a[2] * a[7] - a[13] * a[3] * a[6];
  inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15] - a[4] * a[3] * a[14] - a[12] * a[2] * a[7] + a[12] * a[3] * a[6];
  inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15] + a[4] * a[3] * a[13] + a[12] * a[1] * a[7] - a[12] * a[3] * a[5];
  inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14] - a[4] * a[2] * a[13] - a[12] * a[1] * a[6] + a[12] * a[2] * a[5];
  inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11] - a[5] * a[3] * a[10] - a[9] * a[2] * a[7] + a[9] * a[3] * a[6];
  inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11] + a[4] * a[3] * a[10] + a[8] * a[2] * a[7] - a[8] * a[3] * a[6];
  inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11] - a[4] * a[3] * a[9] - a[8] * a[1] * a[7] + a[8] * a[3] * a[5];
  inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10] + a[4] * a[2] * a[9] + a[8] * a[1] * a[6] - a[8] * a[2] * a[5];
  const det = 1 / (a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12]);
  for (let i = 0; i < 16; i++) inv[i] *= det;
  return inv;
}
function norm(v: number[]) {
  const l = Math.hypot(v[0], v[1], v[2]) || 1;
  return [v[0] / l, v[1] / l, v[2] / l];
}

/** Whether WebGL is drawn on the CPU (SwiftShader, llvmpipe): it works, but
 * slowly enough to drag the whole page with it. Chrome hides the renderer
 * behind the debug extension; Firefox and Safari name it in RENDERER. */
function software(gl: GL): boolean {
  let name = String(gl.getParameter(gl.RENDERER) ?? "");
  if (/^webkit webgl$/i.test(name)) {
    const info = gl.getExtension("WEBGL_debug_renderer_info");
    if (info) name = String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL) ?? "");
  }
  return /swiftshader|llvmpipe|softpipe|software|basic render driver/i.test(name);
}

/** The page's paper colour, linear, from the theme's own token. */
function paperOf(light: boolean): [number, number, number] {
  const raw = getComputedStyle(document.documentElement).getPropertyValue("--paper").trim();
  const hex = /^#([0-9a-f]{6})$/i.exec(raw)?.[1] ?? (light ? "f2eee6" : "0c0d0f");
  return [0, 2, 4].map((i) => Math.pow(parseInt(hex.slice(i, i + 2), 16) / 255, 2.2)) as [number, number, number];
}

/**
 * Starts the picture on `canvas`, filling `host`. The mark forms over
 * `anchor` (the poster's box), and the light dims behind each element
 * `shade` returns. Returns null where WebGL2 is missing or fails, and the
 * poster stays.
 */
export function mount(
  host: HTMLElement,
  canvas: HTMLCanvasElement,
  anchor: HTMLElement,
  shade: () => Element[],
  variant: Variant,
  initial: Options,
  onState: (s: State) => void,
): Handle | null {
  const ctx = canvas.getContext("webgl2", { alpha: true, premultipliedAlpha: true, antialias: false, depth: false, stencil: false, powerPreference: "high-performance" });
  if (!ctx) return null;
  const gl: GL = ctx;
  let opts = { ...initial };
  let res: ReturnType<typeof build> | null = null;
  try {
    res = build(gl);
  } catch {
    return null;
  }

  let t = opts.still ? STILL : START[variant];
  const cpu = software(gl);
  let level = cpu ? LEVELS.length - 1 : matchMedia("(pointer: coarse)").matches ? 1 : 0;
  let raf = 0;
  let last = 0;
  let visible = true;
  let lost = false;
  let state: State | null = null;
  let frames = 0;
  let geometry = measure();
  let paper = paperOf(opts.light);
  // Quality: recent frame times, when the level last changed, and the levels
  // that have failed (never retried twice)
  const times: number[] = [];
  let changed = performance.now();
  const failed = new Set<number>();
  // The pointer, in the canvas's CSS pixels, and the wake it leaves
  const ptr = { x: -1e4, y: -1e4, vx: 0, vy: 0, on: 0, cx: -1e4, cy: -1e4, trail: [] as { x: number; y: number; s: number }[] };

  function build(g: GL) {
    const hdr = !!(g.getExtension("EXT_color_buffer_float") || g.getExtension("EXT_color_buffer_half_float"));
    return {
      hdr,
      points: program(g, POINTS_VS, POINTS_FS),
      lines: program(g, POINTS_VS.replace("#version 300 es", "#version 300 es\n#define LINES 1"), POINTS_FS.replace("#version 300 es", "#version 300 es\n#define LINES 1")),
      ground: program(g, TRI_VS, GROUND_FS),
      fade: program(g, TRI_VS, FADE_FS),
      down: program(g, TRI_VS, DOWN_FS),
      up: program(g, TRI_VS, UP_FS),
      composite: program(g, TRI_VS, COMPOSITE_FS),
      vao: g.createVertexArray(),
      scene: null as Target | null,
      mips: [] as Target[],
      acc: [] as Target[],
    };
  }

  function free() {
    if (!res) return;
    for (const tg of [res.scene, ...res.mips, ...res.acc]) {
      if (!tg) continue;
      gl.deleteTexture(tg.tex);
      gl.deleteFramebuffer(tg.fbo);
    }
    res.scene = null;
    res.mips = [];
    res.acc = [];
  }

  function measure() {
    const box = host.getBoundingClientRect();
    const a = anchor.getBoundingClientRect();
    return {
      box,
      mark: { x: a.left + a.width / 2 - box.left, y: a.top + a.height / 2 - box.top, r: a.width / 2 },
      // Small words get the light behind them cleared entirely (on paper the
      // densest ink absorbs so much that even a few percent of it would grey
      // them); a headline keeps a faint field behind it and reads well over 3:1
      shade: shade()
        .map((e) => ({ r: e.getBoundingClientRect(), big: parseFloat(getComputedStyle(e).fontSize) >= 24 }))
        .filter(({ r }) => r.width > 0 && r.height > 0)
        .map(({ r, big }) => [r.left - box.left, r.top - box.top, r.right - box.left, r.bottom - box.top, big ? 0.93 : 1]),
      // Where the words in the picture's own section end, for the wide
      // screen's fade (the header's links, also shaded, reach the far edge)
      words: Math.max(0, ...shade().filter((e) => host.parentElement?.contains(e)).map((e) => e.getBoundingClientRect().right - box.left)),
    };
  }

  function size() {
    const css = geometry.box;
    const lv = LEVELS[level];
    let dpr = Math.min(window.devicePixelRatio || 1, lv.dpr);
    const px = css.width * css.height * dpr * dpr;
    if (px > MAX_PIXELS) dpr *= Math.sqrt(MAX_PIXELS / px);
    const w = Math.max(1, Math.round(css.width * dpr));
    const h = Math.max(1, Math.round(css.height * dpr));
    if (!res) return dpr;
    if (canvas.width !== w || canvas.height !== h || !res.scene) {
      canvas.width = w;
      canvas.height = h;
      free();
      res.scene = target(gl, w, h, res.hdr);
      let mw = w;
      let mh = h;
      for (let i = 0; i < 6 && mw > 8 && mh > 8; i++) {
        mw = Math.max(1, mw >> 1);
        mh = Math.max(1, mh >> 1);
        res.mips.push(target(gl, mw, mh, res.hdr));
      }
    }
    if (lv.trails && res.acc.length === 0) res.acc = [target(gl, w, h, res.hdr), target(gl, w, h, res.hdr)];
    return dpr;
  }

  function tri() {
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  }

  function draw(dt: number, warm = 0) {
    if (!res || lost) return;
    const r = res;
    const dpr = size();
    const scene = r.scene;
    if (!scene) return;
    const w = scene.w;
    const h = scene.h;
    const lv = LEVELS[level];
    const light = opts.light;
    const cssW = geometry.box.width;
    const wide = cssW >= 1080;

    // The camera stands back from the gathering and looks a little down on
    // it; the lens is shifted so the mark lands on its anchor
    const R = 2.4;
    const M = [0, R * (wide || variant === "close" ? 1.32 : 1.2), 0];
    const eye = [0, 5.2, 19];
    const at = [0, 3.0, 0];
    const view = lookAt(eye, at);
    const dist = Math.hypot(M[0] - eye[0], M[1] - eye[1], M[2] - eye[2]);
    const rpx = Math.max(8, geometry.mark.r * dpr);
    const f = ((2 * rpx) / h) * (dist / R);
    const proj = perspective(2 * Math.atan(1 / f), w / h, 0.1, 400);
    const c = project(mul(proj, view), M);
    proj[8] = -(((geometry.mark.x * dpr) / w) * 2 - 1 - c[0]);
    proj[9] = -((1 - (geometry.mark.y * dpr) / h) * 2 - 1 - c[1]);
    const vp = mul(proj, view);
    const ph = (t / PERIOD) % 1;
    const glow = ph < 0.3 ? (ph / 0.3) * 0.4 : ph < 0.6 ? 0.4 + ((ph - 0.3) / 0.3) * 0.6 : ph < 0.75 ? 1 : Math.max(0, 1 - (ph - 0.75) / 0.2);
    const n = Math.round(population(variant, cssW) * lv.n);
    const trails = lv.trails && r.acc.length === 2;

    gl.bindVertexArray(r.vao);
    const setPlace = (pg: Prog) => {
      gl.uniformMatrix4fv(pg.u("uVP"), false, vp);
      gl.uniform1f(pg.u("uR"), R);
      gl.uniform3f(pg.u("uM"), M[0], M[1], M[2]);
      gl.uniform3f(pg.u("uRight"), view[0], view[4], view[8]);
      gl.uniform3f(pg.u("uUp"), view[1], view[5], view[9]);
      gl.uniform1f(pg.u("uFocus"), dist);
      gl.uniform1f(pg.u("uAperture"), 0.011);
      gl.uniform1f(pg.u("uPx"), dpr);
      gl.uniform1f(pg.u("uLight"), light ? 1 : 0);
      gl.uniform1f(pg.u("uBright"), light ? 0.5 : 0.3);
      gl.uniform1f(pg.u("uHeld"), light ? 1.5 : 1.9);
      gl.uniform2f(pg.u("uRes"), w, h);
      const on = ptr.on * (opts.still ? 0 : 1);
      gl.uniform4f(pg.u("uPtr"), ptr.x * dpr, h - ptr.y * dpr, on, 110 * dpr);
      const trail = new Float32Array(24);
      ptr.trail.forEach((p, i) => trail.set([p.x * dpr, h - p.y * dpr, p.s * (opts.still ? 0 : 1)], i * 3));
      gl.uniform3fv(pg.u("uTrail"), trail);
    };

    // The trails: last frame's exposure, faded, plus every moving light's
    // step since then
    if (trails) {
      const steps = warm > 0 ? warm : 1;
      for (let s = 0; s < steps; s++) {
        const stepDt = warm > 0 ? 1 / 30 : dt;
        const tt = warm > 0 ? t - (warm - 1 - s) / 30 : t;
        const [src, dst] = r.acc;
        gl.bindFramebuffer(gl.FRAMEBUFFER, dst.fbo);
        gl.viewport(0, 0, w, h);
        gl.disable(gl.BLEND);
        gl.useProgram(r.fade.p);
        gl.activeTexture(gl.TEXTURE0);
        gl.bindTexture(gl.TEXTURE_2D, src.tex);
        gl.uniform1i(r.fade.u("uSrc"), 0);
        gl.uniform1f(r.fade.u("uK"), Math.pow(0.5, stepDt / 0.45));
        tri();
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.ONE, gl.ONE);
        gl.useProgram(r.lines.p);
        setPlace(r.lines);
        gl.uniform1f(r.lines.u("uTime"), tt);
        gl.uniform1f(r.lines.u("uDt"), Math.max(stepDt, 1 / 240));
        // A third of the lights draw trails; the stride is prime to the
        // mark's eight blades, so every blade gets its share
        gl.uniform1f(r.lines.u("uStride"), 3);
        gl.drawArrays(gl.LINES, 0, Math.floor(n / 3) * 2);
        r.acc = [dst, src];
      }
    }

    gl.bindFramebuffer(gl.FRAMEBUFFER, scene.fbo);
    gl.viewport(0, 0, w, h);
    gl.disable(gl.BLEND);
    gl.useProgram(r.ground.p);
    gl.uniformMatrix4fv(r.ground.u("uInv"), false, invert(vp));
    gl.uniform3f(r.ground.u("uCam"), eye[0], eye[1], eye[2]);
    gl.uniform3f(r.ground.u("uM"), M[0], M[1], M[2]);
    gl.uniform1f(r.ground.u("uGlow"), glow);
    gl.uniform1f(r.ground.u("uLight"), light ? 1 : 0);
    gl.uniform1f(r.ground.u("uTrails"), trails ? 1 : 0);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, trails ? r.acc[0].tex : r.mips[0].tex);
    gl.uniform1i(r.ground.u("uAcc"), 0);
    tri();

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE);
    gl.useProgram(r.points.p);
    setPlace(r.points);
    gl.uniform1f(r.points.u("uTime"), t);
    gl.uniform1f(r.points.u("uStride"), 1);
    gl.drawArrays(gl.POINTS, 0, n);
    gl.disable(gl.BLEND);

    // Bloom: a chain of halvings, then back up, each level added to the next
    gl.useProgram(r.down.p);
    gl.uniform1i(r.down.u("uSrc"), 0);
    let from = scene;
    for (const m of r.mips) {
      gl.bindFramebuffer(gl.FRAMEBUFFER, m.fbo);
      gl.viewport(0, 0, m.w, m.h);
      gl.bindTexture(gl.TEXTURE_2D, from.tex);
      gl.uniform2f(r.down.u("uTexel"), 1 / from.w, 1 / from.h);
      tri();
      from = m;
    }
    gl.useProgram(r.up.p);
    gl.uniform1i(r.up.u("uSrc"), 0);
    gl.enable(gl.BLEND);
    gl.blendFunc(gl.ONE, gl.ONE);
    for (let i = r.mips.length - 1; i > 0; i--) {
      const a = r.mips[i];
      const b = r.mips[i - 1];
      gl.bindFramebuffer(gl.FRAMEBUFFER, b.fbo);
      gl.viewport(0, 0, b.w, b.h);
      gl.bindTexture(gl.TEXTURE_2D, a.tex);
      gl.uniform2f(r.up.u("uTexel"), 1 / a.w, 1 / a.h);
      tri();
    }
    gl.disable(gl.BLEND);

    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.viewport(0, 0, w, h);
    gl.useProgram(r.composite.p);
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, scene.tex);
    gl.activeTexture(gl.TEXTURE1);
    gl.bindTexture(gl.TEXTURE_2D, r.mips[0].tex);
    const cu = r.composite.u;
    gl.uniform1i(cu("uScene"), 0);
    gl.uniform1i(cu("uBloom"), 1);
    gl.uniform1f(cu("uBloomK"), light ? 0.3 : 0.22);
    gl.uniform1f(cu("uExposure"), light ? 2.6 : 1);
    gl.uniform1f(cu("uLight"), light ? 1 : 0);
    gl.uniform1f(cu("uTime"), t);
    gl.uniform3f(cu("uPaper"), ...paper);
    const rects = new Float32Array(40);
    const strength = new Float32Array(10);
    const shades = geometry.shade.slice(0, 10);
    shades.forEach((s, i) => {
      rects.set([s[0] * dpr, h - s[3] * dpr, s[2] * dpr, h - s[1] * dpr], i * 4);
      strength[i] = s[4];
    });
    gl.uniform4fv(cu("uShade[0]"), rects);
    gl.uniform1fv(cu("uShadeS[0]"), strength);
    gl.uniform1i(cu("uShadeN"), shades.length);
    gl.uniform1f(cu("uFeather"), 28 * dpr);
    // On a wide screen the words' side falls away toward the left edge
    const side = wide && variant === "hero" ? geometry.words : 0;
    gl.uniform2f(cu("uSide"), side * dpr, side ? 0.22 * w : 0);
    gl.uniform1f(cu("uFoot"), (variant === "close" ? 0.3 : 0.16) * h);
    tri();
    gl.activeTexture(gl.TEXTURE0);
  }

  function setState(next: State) {
    if (next === state) return;
    state = next;
    onState(next);
  }

  function schedule() {
    if (lost) return;
    const run = visible && !document.hidden && !opts.paused && !opts.still;
    if (run && !raf) {
      last = performance.now();
      raf = requestAnimationFrame(tick);
    } else if (!run && raf) {
      cancelAnimationFrame(raf);
      raf = 0;
    }
    setState(opts.still ? "still" : opts.paused ? "paused" : run ? "running" : "away");
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
  }

  function tick(now: number) {
    raf = requestAnimationFrame(tick);
    const ms = now - last;
    last = now;
    const dt = Math.min(0.05, ms / 1000);
    t += dt;
    stepPointer(dt);
    if (++frames % 60 === 0) geometry = measure();
    draw(dt);
    if (frames > 30) adapt(ms);
  }

  // A still or paused picture is drawn once whenever what it shows changes
  function redraw() {
    if (lost) return;
    geometry = measure();
    draw(1 / 60, LEVELS[level].trails ? 90 : 0);
  }

  function stepPointer(dt: number) {
    const live = host.getBoundingClientRect();
    const inside = ptr.cx >= live.left && ptr.cx <= live.right && ptr.cy >= live.top && ptr.cy <= live.bottom;
    const x = ptr.cx - live.left;
    const y = ptr.cy - live.top;
    if (inside && ptr.x > -1e3) {
      ptr.vx = ptr.vx * 0.7 + (x - ptr.x) * 0.3;
      ptr.vy = ptr.vy * 0.7 + (y - ptr.y) * 0.3;
    }
    ptr.x = x;
    ptr.y = y;
    ptr.on = Math.max(0, ptr.on - dt * 0.35);
    const speed = Math.hypot(ptr.vx, ptr.vy);
    if (speed > 0.5 && ptr.on > 0) ptr.trail.unshift({ x, y, s: Math.min(1, speed / 18) });
    for (const p of ptr.trail) p.s *= Math.pow(0.12, dt);
    ptr.trail = ptr.trail.filter((p) => p.s > 0.02).slice(0, 8);
    ptr.vx *= Math.pow(0.05, dt);
    ptr.vy *= Math.pow(0.05, dt);
  }

  const onPointer = (e: PointerEvent) => {
    ptr.cx = e.clientX;
    ptr.cy = e.clientY;
    const b = host.getBoundingClientRect();
    if (e.clientX >= b.left && e.clientX <= b.right && e.clientY >= b.top && e.clientY <= b.bottom) ptr.on = 1;
  };
  window.addEventListener("pointermove", onPointer, { passive: true });
  window.addEventListener("pointerdown", onPointer, { passive: true });

  const io = new IntersectionObserver(([e]) => {
    visible = e.isIntersecting;
    schedule();
  });
  io.observe(host);
  const ro = new ResizeObserver(() => {
    geometry = measure();
    if (!raf) redraw();
  });
  ro.observe(host);
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
      res = build(gl);
      lost = false;
      state = null;
      redraw();
      schedule();
    } catch {
      setState("fallback");
    }
  };
  canvas.addEventListener("webglcontextlost", onLost);
  canvas.addEventListener("webglcontextrestored", onRestored);

  try {
    redraw();
  } catch {
    destroy();
    return null;
  }
  // On a software renderer the one frame above proves the scene works; the
  // poster stays up rather than the page paying for every frame after it
  if (cpu) {
    destroy();
    onState("software");
    return { update() {}, destroy() {} };
  }
  schedule();

  function destroy() {
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    io.disconnect();
    ro.disconnect();
    window.removeEventListener("pointermove", onPointer);
    window.removeEventListener("pointerdown", onPointer);
    document.removeEventListener("visibilitychange", onVisibility);
    canvas.removeEventListener("webglcontextlost", onLost);
    canvas.removeEventListener("webglcontextrestored", onRestored);
    free();
    gl.getExtension("WEBGL_lose_context")?.loseContext();
  }

  return {
    update(next) {
      const was = opts;
      opts = { ...opts, ...next };
      if (opts.light !== was.light) paper = paperOf(opts.light);
      if (opts.still && !was.still) t = STILL;
      if (!opts.still && was.still) t = START[variant];
      schedule();
      if (!raf) redraw();
    },
    destroy,
  };
}
