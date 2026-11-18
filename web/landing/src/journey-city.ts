// The place, for the journey (journey.ts): a city at night, seen from a
// rooftop some 260 m back and 46 m up across a park. In the park's heart a
// plaza, and in the plaza a fountain of liquid light; eight paths run out
// from it under the trees, an open lawn runs from it toward the viewer, and
// round the park stand avenues and streets of masonry blocks, a few slim
// towers and the water tanks on the roofs between them. Nothing in it is any
// one place. It is ray-marched once per camera move into three buffers; the
// frame itself only pays for the wet paving, the lamps, the people, the
// fountain's liquid and the bloom.

export const NW = 40; // walkers

export const LIB = `
#define PI 3.14159265
uniform vec3 u_cam; uniform vec3 u_fwd; uniform vec3 u_right; uniform vec3 u_up; uniform float u_tanf; uniform vec2 u_res; uniform int u_light; uniform float u_pix; uniform float u_quiet; uniform float u_qx;
float h11(float p) { p = fract(p * 0.1031); p *= p + 33.33; p *= p + p; return fract(p); }
float h21(vec2 p) { vec3 p3 = fract(vec3(p.xyx) * 0.1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
float h31(vec3 p) { p = fract(p * 0.1031); p += dot(p, p.zyx + 31.32); return fract((p.x + p.y) * p.z); }
float vnoise(vec2 p) { vec2 i = floor(p), f = fract(p); f = f * f * (3.0 - 2.0 * f); return mix(mix(h21(i), h21(i + vec2(1, 0)), f.x), mix(h21(i + vec2(0, 1)), h21(i + vec2(1, 1)), f.x), f.y); }
float fbm(vec2 p) { float a = 0.5, s = 0.0; for (int i = 0; i < 4; i++) { s += a * vnoise(p); p = mat2(1.6, 1.2, -1.2, 1.6) * p; a *= 0.5; } return s; }
float noise3(vec3 p) {
  vec3 i = floor(p), f = fract(p); f = f * f * (3.0 - 2.0 * f);
  return mix(mix(mix(h31(i), h31(i + vec3(1, 0, 0)), f.x), mix(h31(i + vec3(0, 1, 0)), h31(i + vec3(1, 1, 0)), f.x), f.y),
             mix(mix(h31(i + vec3(0, 0, 1)), h31(i + vec3(1, 0, 1)), f.x), mix(h31(i + vec3(0, 1, 1)), h31(i + vec3(1, 1, 1)), f.x), f.y), f.z);
}
vec3 rayDir(vec2 uv) { vec2 n = uv * 2.0 - 1.0; n.x *= u_res.x / u_res.y; return normalize(u_fwd + n.x * u_tanf * u_right + n.y * u_tanf * u_up); }
float lineL(float d, float w0, float fw) { float w = max(w0, fw * 0.75); return (w0 / w) * exp(-d * d / (2.0 * w * w)); }
float sdBox2(vec2 p, vec2 b) { vec2 q = abs(p) - b; return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0); }

// ---- the plan. z is up; the fountain is at the origin, and the camera
// stands south of the park looking north (+y).
const float PXH = 120.0;                // the park's half width
const float PY0 = -163.0, PY1 = 135.0;  // its near and far edges
const float PLR = 34.0;                 // the plaza round the fountain
const float PW = 2.6;                   // a path's half width
const float LAWN = 30.0;                // the open lawn toward the viewer, half width
const float AV0 = 133.0, AVS = 230.0, AVH = 13.0;  // avenues: the first one's centre, the spacing, the half width
const float ST0 = 146.0, STS = 80.0, STH = 10.0;   // cross streets the same
const float CITY = 1500.0;              // where the modelled city ends and the sky's skyline goes on
const vec3 SUN = vec3(-0.88, -0.24, 0.31); // late afternoon: low, from the west, so the trees' shadows run across the lawn

// The copy's side of the wide hero is kept quiet (u_quiet), in screen space:
// the picture left of u_qx, the copy's right edge, is given up to the night.
float quietAt(vec2 uv) { return u_quiet * (1.0 - smoothstep(u_qx - 0.1, u_qx + 0.02, uv.x)); }
bool inPark(vec2 p) { return abs(p.x) < PXH && p.y > PY0 && p.y < PY1; }
// the nearest of the park's eight paths: how far off it, and how far along it
float pathOff(vec2 p, out float along) {
  float k = floor(atan(p.y, p.x) / (PI / 4.0) + 0.5);
  vec2 d = vec2(cos(k * PI / 4.0), sin(k * PI / 4.0));
  along = dot(p, d);
  return abs(d.x * p.y - d.y * p.x);
}
// The park's lamps: down each path every 18 m from the plaza out, on one side then the other,
// and eight round the plaza between the paths. The nearest one to p.
vec2 parkLamp(vec2 p) {
  float k = floor(atan(p.y, p.x) / (PI / 4.0) + 0.5);
  vec2 d = vec2(cos(k * PI / 4.0), sin(k * PI / 4.0)), nr = vec2(-d.y, d.x);
  float i = max(0.0, floor((dot(p, d) - 42.0) / 18.0 + 0.5));
  vec2 a = d * (42.0 + 18.0 * i) + nr * (mod(i, 2.0) < 0.5 ? -3.4 : 3.4);
  if (!inPark(a)) a = vec2(9000.0);
  float k2 = floor(atan(p.y, p.x) / (PI / 4.0)) + 0.5;
  vec2 b = vec2(cos(k2 * PI / 4.0), sin(k2 * PI / 4.0)) * 31.0;
  return dot(p - a, p - a) < dot(p - b, p - b) ? a : b;
}
// The streets' lamps: down both kerbs of every avenue and street, every 30 m.
vec2 streetLamp(vec2 p) {
  if (max(abs(p.x), abs(p.y)) > CITY) return vec2(9000.0);
  float ax = abs(p.x);
  float xc = AV0 + max(0.0, floor((ax - AV0) / AVS + 0.5)) * AVS;
  vec2 a = vec2((p.x < 0.0 ? -1.0 : 1.0) * (xc + (ax < xc ? -1.0 : 1.0) * (AVH - 1.5)), floor(p.y / 30.0 + 0.5) * 30.0);
  float yc = ST0 + floor((p.y - ST0) / STS + 0.5) * STS;
  vec2 b = vec2(floor(p.x / 30.0) * 30.0 + 15.0, yc + (p.y < yc ? -1.0 : 1.0) * (STH - 1.3));
  // no street crosses the park
  if (abs(b.x) < AV0 && yc > PY0 && yc < PY1) b = vec2(9000.0);
  return dot(p - a, p - a) < dot(p - b, p - b) ? a : b;
}
// In a street: how far to the building line (negative inside a block or the park).
float streetFace(vec2 p) {
  float ax = abs(p.x);
  float av = AVH - abs(ax - (AV0 + max(0.0, floor((ax - AV0) / AVS + 0.5)) * AVS));
  float st = STH - abs(p.y - (ST0 + floor((p.y - ST0) / STS + 0.5) * STS));
  if (inPark(p)) return -1.0;
  return max(av, st);
}
// The night air holds the city's light, thicker low down: how much of it lies
// between ro and ro + rd t, and its colour.
float fogAmt(vec3 ro, vec3 rd, float t) {
  bool day = u_light == 1;
  float H = day ? 260.0 : 110.0, d0 = day ? 1.0 / 720.0 : 1.0 / 880.0;
  float k = rd.z / H;
  float od = d0 * exp(-ro.z / H) * (abs(k) < 1e-6 ? t : (1.0 - exp(-k * t)) / k);
  return 1.0 - exp(-od);
}
vec3 hazeCol(vec3 rd) {
  if (u_light == 1) return vec3(0.95, 0.9, 0.81) + vec3(0.03, 0.02, -0.03) * exp(-max(rd.z, 0.0) * 8.0);
  return vec3(0.058, 0.045, 0.046) + vec3(0.11, 0.055, 0.022) * exp(-max(rd.z, 0.0) * 14.0);
}
`;

const HEAD = `#version 300 es
precision highp float;
in vec2 v_uv;
`;

// o0: the city's own colour, and in alpha the distance (0 = paving, -1 = sky).
// o1: how the surface answers the fountain's light. On paving, how the
//     mirrored city answers it; alpha is the contact shadow.
// o2: on paving at night, what the wet stone mirrors; by day, the sun's
//     shadow on it.
export const STATIC_FS = HEAD + `layout(location = 0) out vec4 o0; layout(location = 1) out vec4 o1; layout(location = 2) out vec4 o2;` + LIB + `
uniform int u_steps;
float g_sink; // how far the surface being shaded is given up to the haze
float g_soft; // the distance to the nearest thing, without the cell guards
// 1 when the step map() gave is a cell guard, not a surface: a guard that comes
// out under the hit threshold far off must not count as a hit
float g_guard;

// ---- the blocks. Every point belongs to the cell round one block, from
// street centre to street centre; the park's cell has none.
bool blockAt(vec2 p, out vec4 b, out vec2 bid) {
  float ax = abs(p.x);
  float k = floor((ax - AV0) / AVS);
  float j = floor((p.y - ST0) / STS);
  float y0 = ST0 + j * STS + STH, y1 = ST0 + (j + 1.0) * STS - STH;
  if (k < 0.0) {
    if (y1 > PY0 && y0 < PY1) return false;
    b = vec4(-(AV0 - AVH), y0, AV0 - AVH, y1);
    bid = vec2(0.0, j);
  } else {
    float x0 = AV0 + k * AVS + AVH, x1 = AV0 + (k + 1.0) * AVS - AVH;
    b = p.x < 0.0 ? vec4(-x1, y0, -x0, y1) : vec4(x0, y0, x1, y1);
    bid = vec2((k + 1.0) * (p.x < 0.0 ? -1.0 : 1.0), j);
  }
  return true;
}
// A block is cut into lots, two rows deep. kind 0 is masonry, 1 masonry that
// steps back as it climbs, 2 a slim tower; tank is where a water tank stands
// on the roof (x < -1e3: none).
struct Lot { vec2 id; vec2 c; vec2 hs; float H; float kind; vec2 tank; float facing; };
Lot lotAt(vec2 p, vec4 b, vec2 bid) {
  Lot L;
  float bw = b.z - b.x, dep = (b.w - b.y) * 0.5;
  float n = max(2.0, floor(bw / (19.0 + 12.0 * h21(bid + 4.1))));
  float lw = bw / n;
  float i = clamp(floor((p.x - b.x) / lw), 0.0, n - 1.0);
  float row = p.y - b.y < dep ? 0.0 : 1.0;
  L.c = vec2(b.x + (i + 0.5) * lw, b.y + (row + 0.5) * dep);
  L.hs = vec2(lw, dep) * 0.5;
  L.id = bid * vec2(31.0, 17.0) + vec2(i, row * 53.0);
  float r1 = h21(L.id + 0.37), r2 = h21(L.id + 5.1), r3 = h21(L.id + 9.7);
  // Mostly roofs below the viewer, so the eye runs out over them to the sky;
  // the city only climbs where its centre is, off to the north-east
  float core = 1.0 - smoothstep(150.0, 520.0, length(L.c - vec2(430.0, 980.0)));
  L.kind = 0.0;
  L.H = 12.0 + 22.0 * r1 * r1 + 10.0 * r3 * core;
  if (r2 < 0.05 + 0.3 * core) { L.kind = 1.0; L.H = 40.0 + 32.0 * r1 + 40.0 * core * r3; }
  // slim towers only out where the skyline is, never crowding the park
  if (r2 > 0.99 - 0.24 * core && lw > 21.0 && length(L.c) > 650.0) { L.kind = 2.0; L.H = 105.0 + 70.0 * r3 + 60.0 * core * r1; }
  // the walls round the park are a frame for it: even, a little taller, their
  // ground floors lit
  L.facing = 0.0;
  if ((bid.x == 0.0 && bid.y == 0.0 && row == 0.0) || (abs(bid.x) == 1.0 && bid.y > -4.5 && bid.y < -0.5 && (bid.x > 0.0 ? i == 0.0 : i == n - 1.0))) {
    L.facing = 1.0;
    L.kind = r2 < 0.22 ? 1.0 : 0.0;
    L.H = L.kind > 0.5 ? 42.0 + 18.0 * r1 : 21.0 + 12.0 * r1;
  }
  // the block the camera stands over stays below its view
  if (bid.y < -4.5 && bid.y > -5.5 && abs(bid.x) < 1.5) { L.kind = 0.0; L.H = min(L.H, abs(bid.x) > 0.5 ? 20.0 : 9.0); }
  L.tank = L.kind < 0.5 && L.H < 46.0 && r3 < 0.45 ? (vec2(h21(L.id + 2.2), h21(L.id + 3.3)) - 0.5) * max(L.hs - 4.0, 0.0) * 2.0 : vec2(-1e4);
  return L;
}
// the tiers of a setback building
vec3 tiers(Lot L) { return vec3(floor(L.H * 0.58), floor(L.H * 0.82), L.H); }
float sdLot(vec3 p, Lot L) {
  vec2 q = p.xy - L.c;
  float d;
  if (L.kind < 0.5) {
    d = max(sdBox2(q, L.hs), p.z - L.H);
    // the bulkhead over the stair
    vec2 bc = (vec2(h21(L.id + 6.6), h21(L.id + 7.7)) - 0.5) * max(L.hs - 5.0, 0.0);
    d = min(d, max(sdBox2(q - bc, vec2(3.0, 2.2)), p.z - L.H - 3.4));
    if (L.tank.x > -1e3) {
      vec2 tq = q - L.tank;
      float rr = length(tq);
      float tank = max(rr - 2.5, abs(p.z - (L.H + 4.7)) - 2.1);
      float cap = max(rr - 2.7 + (p.z - L.H - 6.8) * 1.1, max(L.H + 6.8 - p.z, p.z - L.H - 9.2)) * 0.67;
      float legs = max(sdBox2(abs(tq) - vec2(1.6), vec2(0.18)), p.z - L.H - 2.7);
      d = min(d, min(min(tank, cap), legs));
    }
  } else if (L.kind < 1.5) {
    vec3 T = tiers(L);
    float in2 = min(3.5, min(L.hs.x, L.hs.y) * 0.25), in3 = min(7.5, min(L.hs.x, L.hs.y) * 0.5);
    d = max(sdBox2(q, L.hs), p.z - T.x);
    d = min(d, max(sdBox2(q, L.hs - in2), p.z - T.y));
    d = min(d, max(sdBox2(q, L.hs - in3), p.z - T.z));
  } else {
    // a slim tower, its crown cut back at an angle
    float w = min(L.hs.x, L.hs.y) - 2.5;
    vec2 hs = vec2(w, min(L.hs.y - 2.0, w * 1.25));
    float cut = max(0.0, p.z - (L.H - 14.0)) * 0.42;
    d = max(sdBox2(q - vec2(cut * 0.5, 0.0), hs - vec2(cut * 0.5, 0.0)), p.z - L.H) * 0.92;
  }
  return d;
}
float sdCity(vec3 p) {
  g_guard = 0.0;
  if (p.z > 262.0) return p.z - 260.0;
  g_guard = 1.0;
  float cm = max(abs(p.x), abs(p.y));
  if (cm > CITY) { g_soft = min(g_soft, cm - CITY); return max(cm - CITY, p.z - 260.0) + 0.5; }
  vec4 b; vec2 bid;
  if (!blockAt(p.xy, b, bid)) {
    // the park's cell: no building nearer than the avenues and streets round it
    float d0 = min(AV0 + AVH - abs(p.x), min(ST0 + STH - p.y, p.y - (ST0 - 4.0 * STS - STH)));
    g_soft = min(g_soft, d0);
    return max(d0, p.z - 260.0) + 0.3;
  }
  float out2 = sdBox2(p.xy - (b.xy + b.zw) * 0.5, (b.zw - b.xy) * 0.5);
  if (out2 > 0.0) { g_soft = min(g_soft, out2); return max(out2, p.z - 260.0) + 0.3; }
  Lot L = lotAt(p.xy, b, bid);
  float d = max(sdLot(p, L), cm - CITY);
  g_soft = min(g_soft, d);
  // never step past the lot's own edge
  float guard = 0.4 - sdBox2(p.xy - L.c, L.hs);
  g_guard = guard < d ? 1.0 : 0.0;
  return min(d, guard);
}
// The park's trees, as one canopy: a heightfield of crowns with the plaza,
// the paths and the lawn left open.
// a crown: round-topped, one to a cell of the park's grid, jittered
float crowns(vec2 p) {
  vec2 g = p / 9.0, c = floor(g);
  float h = 0.0;
  for (int j = -1; j <= 1; j++) for (int i = -1; i <= 1; i++) {
    vec2 cc = c + vec2(float(i), float(j));
    vec2 o = cc + 0.2 + 0.6 * vec2(h21(cc), h21(cc + 7.3));
    float rr = (0.55 + 0.35 * h21(cc + 3.1));
    float d = length(g - o) / rr;
    h = max(h, sqrt(max(0.0, 1.0 - d * d)) * (0.7 + 0.6 * h21(cc + 5.9)));
  }
  return h;
}
float canopyH(vec2 p) {
  float al;
  float po = pathOff(p, al);
  float r = length(p);
  float open = min(min(r - PLR, al > 0.0 ? po - PW - 1.5 : 1e9), max(abs(p.x) - LAWN, p.y + 32.0));
  float edge = min(min(PXH - abs(p.x), p.y - PY0), PY1 - p.y);
  float m = smoothstep(0.0, 7.0, open) * smoothstep(1.0, 8.0, edge);
  float h = (7.0 + 5.0 * fbm(p * 0.045 + 3.0) + 4.0 * crowns(p)) * mix(0.72, 1.0, smoothstep(PLR, PLR + 45.0, r));
  return mix(-3.0, h, m);
}
float sdCanopy(vec3 p) {
  if (p.z > 23.0) return p.z - 22.0;
  float out2 = sdBox2(p.xy - vec2(0.0, (PY0 + PY1) * 0.5), vec2(PXH, (PY1 - PY0) * 0.5));
  if (out2 > 0.5) return max(out2, p.z - 22.0);
  float d = (p.z - canopyH(p.xy)) * 0.3;
  g_soft = min(g_soft, d);
  return d;
}
float sdFountain(vec3 p) {
  float r = length(p.xy);
  if (r > 10.6) return r - 9.7;
  float wall = max(abs(r - 7.9) - 0.42, p.z - 0.95);
  float plinth = max(abs(r - 8.75) - 0.62, p.z - 0.27);
  float ped = max(r - mix(1.05, 0.5, clamp(p.z / 3.0, 0.0, 1.0)), p.z - 3.0);
  float bowl = max(r - (0.9 + 1.9 * smoothstep(2.45, 3.35, p.z)), abs(p.z - 2.95) - 0.5);
  float fin = max(r - 0.32, p.z - 4.1);
  float d = min(min(wall, plinth), min(min(ped, bowl), fin));
  g_soft = min(g_soft, d);
  return d;
}
vec2 mapM(vec3 p) {
  g_soft = 1e9;
  vec2 res = vec2(sdCity(p), 1.0);
  float d = sdCanopy(p); if (d < res.x) { res = vec2(d, 3.0); g_guard = 0.0; }
  d = sdFountain(p); if (d < res.x) { res = vec2(d, 6.0); g_guard = 0.0; }
  return res;
}
float map(vec3 p) { return mapM(p).x; }
vec3 calcNormal(vec3 p, float t) {
  vec2 e = vec2(1.0, -1.0) * (0.012 + 0.0005 * t);
  return normalize(e.xyy * map(p + e.xyy) + e.yyx * map(p + e.yyx) + e.yxy * map(p + e.yxy) + e.xxx * map(p + e.xxx));
}
float shadow(vec3 ro, vec3 rd) {
  float res = 1.0, t = 0.3;
  for (int i = 0; i < 56; i++) {
    float d = map(ro + rd * t);
    res = min(res, 7.0 * g_soft / t);
    if (res < 0.01 || t > 320.0) break;
    t += clamp(d, 0.15, 18.0);
  }
  return clamp(res, 0.0, 1.0);
}

vec3 sky(vec3 rd) {
  bool day = u_light == 1;
  float el = rd.z / max(length(rd.xy), 1e-4);
  float h = max(rd.z, 0.0);
  float az = atan(rd.x, rd.y);
  vec3 c;
  // high cloud, lit from beneath by the city
  vec2 cp = rd.xy / (h + 0.08) * 0.55 + vec2(3.0, 1.0);
  float cl = smoothstep(0.36, 0.78, fbm(cp * 0.7 + 0.6 * vec2(fbm(cp * 1.3 + 7.0), fbm(cp * 1.1 + 2.0)))) * smoothstep(0.0, 0.06, h);
  if (!day) {
    c = mix(hazeCol(rd), vec3(0.05, 0.036, 0.062), smoothstep(0.0, 0.1, h));
    c = mix(c, vec3(0.009, 0.013, 0.032), smoothstep(0.08, 0.55, h));
    c = mix(c, mix(vec3(0.2, 0.12, 0.085), vec3(0.06, 0.05, 0.08), smoothstep(0.03, 0.3, h)), cl * 0.8);
    // a few stars, high up and faint
    float st = step(0.9965, h21(floor(vec2(az * 300.0, h * 300.0)))) * smoothstep(0.22, 0.6, h) * (1.0 - cl);
    c += vec3(0.55, 0.6, 0.75) * st * 0.3;
  } else {
    c = mix(hazeCol(rd), vec3(0.62, 0.74, 0.88), smoothstep(0.02, 0.55, h));
    // warmer toward the low sun, behind the viewer's left
    c = mix(c, vec3(0.98, 0.9, 0.78), (1.0 - smoothstep(-0.6, 0.4, rd.x)) * exp(-h * 5.0) * 0.35);
    c = mix(c, vec3(0.99, 0.975, 0.96), cl * 0.55);
  }
  // the far city, past the modelled blocks: layers of silhouettes, a slim
  // tower now and then, and a scatter of lit windows
  for (int i = 0; i < 3; i++) {
    float fi = float(i);
    float sc = 70.0 + 55.0 * fi;
    float x = az * sc + fi * 17.3;
    float id = floor(x), fx = fract(x);
    float r = h11(id * 1.31 + fi * 7.7);
    float hh = (0.006 + 0.02 * r * r * r + 0.007 * h11(floor(x * 0.25) + fi * 3.0)) * (1.0 - 0.25 * fi) - 0.012 - 0.004 * fi;
    hh += step(0.94, h11(id * 3.7 + fi)) * (0.025 + 0.03 * h11(id + 9.0)) * step(abs(fx - 0.5), 0.28);
    if (el < hh && el > -0.05) {
      float depth = 0.5 + 0.17 * fi;
      vec3 b = day ? mix(vec3(0.74, 0.73, 0.75), c, depth + 0.15) : mix(vec3(0.012, 0.012, 0.02), hazeCol(rd), depth);
      vec2 wg = vec2(fx * (5.0 + floor(4.0 * h11(id + 2.0))), el * sc * 4.0);
      vec2 wi = floor(wg), wf = fract(wg);
      float lit = step(0.83 + 0.05 * fi, h21(wi + id * 3.1 + fi)) * step(0.22, wf.x) * step(wf.x, 0.78) * step(0.3, wf.y) * step(wf.y, 0.8);
      if (!day) b += vec3(1.0, 0.66, 0.36) * lit * (0.3 - 0.07 * fi) * (0.35 + 0.65 * h21(wi + 1.7)) * (1.0 - 0.85 * quietAt(v_uv));
      else b = mix(b, b * 0.94, lit);
      return b;
    }
  }
  return c;
}

// A street wall: an open ground storey, rows of windows, a crown. Gives back
// what it emits at night; alb comes back darker where there is glass.
vec3 facade(float s, float z, float top, vec2 seed, float bw, float fh, float base, float litP, float arcade, float clus, float fw, inout vec3 alb, out float glass) {
  vec3 em = vec3(0.0);
  glass = 0.0;
  float detail = 1.0 - smoothstep(0.16, 0.6, fw / bw);
  if (z < base - 0.5) {
    float bay = bw * 1.5;
    float bid = floor(s / bay);
    float ax = (fract(s / bay) - 0.5) * bay;
    float hw = bay * 0.34, spring = base * 0.5;
    float dA = arcade > 0.5 ? (z < spring ? abs(ax) - hw : length(vec2(ax, z - spring)) - hw) : max(abs(ax) - bay * 0.41, abs(z - base * 0.4) - base * 0.33);
    float open = (1.0 - smoothstep(-fw, fw, dA)) * step(0.1, z);
    float on = step(h21(vec2(bid, 3.0) + seed), arcade > 0.5 ? 0.94 : 0.62);
    float var = 0.5 + 0.7 * h21(vec2(bid, 7.0) + seed);
    float mull = 1.0 - 0.6 * detail * max((1.0 - smoothstep(0.03, 0.08, abs(fract(ax / 0.85 + 0.5) - 0.5) * 0.85)), (1.0 - smoothstep(0.04, 0.1, abs(z - spring))));
    em = mix(vec3(1.0, 0.5, 0.2), vec3(1.0, 0.68, 0.38), h21(vec2(bid, 9.0) + seed)) * (0.66 - 0.3 * z / base) * var * on * open * mull;
    // the pier between two openings, and the course above them
    alb *= 1.0 + 0.16 * smoothstep(bay * 0.4, bay * 0.47, abs(ax)) * detail;
    alb *= 1.0 - 0.8 * open;
    glass = open;
  } else if (z > top - 1.3) {
    // the cornice line
    alb *= 1.0 + 0.3 * (1.0 - smoothstep(0.9, 1.3, top - z)) - 0.35 * (1.0 - smoothstep(0.2, 0.5, abs(top - z - 1.5)));
  } else if (z > base) {
    vec2 g = vec2(s / bw, (z - base) / fh);
    vec2 id = floor(g), f = fract(g);
    float ax = clamp(fw / bw, 0.004, 0.28), ay = clamp(fw / fh, 0.004, 0.28);
    float w = smoothstep(0.22 - ax, 0.22 + ax, f.x) * (1.0 - smoothstep(0.78 - ax, 0.78 + ax, f.x)) * smoothstep(0.2 - ay, 0.2 + ay, f.y) * (1.0 - smoothstep(0.8 - ay, 0.8 + ay, f.y));
    float r1 = h21(id + seed), r2 = h21(id * 1.7 + seed + 3.1), rf = h21(vec2(id.y, 5.0) + seed);
    // Lit rooms keep company: a few neighbouring windows, a floor working
    // late, and long stretches of dark between them. Never a pattern.
    float near = vnoise(id * vec2(0.42, 0.62) + seed * 3.7);
    float pl = litP * mix(rf > 0.9 ? 2.8 : rf < 0.3 ? 0.3 : 1.0, 5.5 * smoothstep(0.58, 0.8, near) + (rf > 0.93 ? 1.6 : 0.0) + 0.1, clus);
    float lit = step(r1, pl);
    vec3 wc = r2 < 0.6 ? vec3(1.0, 0.6, 0.28) : r2 < 0.97 ? vec3(1.0, 0.8, 0.54) : vec3(0.7, 0.8, 1.0);
    float bri = 0.25 + 1.05 * h21(id + seed * 1.3 + 9.0);
    float inner = (0.45 + 0.55 * smoothstep(0.25, 0.78, f.y)) * (h21(id + 2.2 + seed) < 0.32 ? 0.4 + 0.6 * step(f.x, 0.5) : 1.0);
    em = mix(vec3(1.0, 0.68, 0.38) * litP * 0.22, wc * bri * lit * inner * w, detail) * 1.1;
    // sills and piers: a little relief between the windows
    float pier = (1.0 - smoothstep(0.04, 0.14, min(f.x, 1.0 - f.x))), sill = (1.0 - smoothstep(0.0, 0.035, abs(f.y - 0.15))), head = (1.0 - smoothstep(0.0, 0.06, abs(f.y - 0.87)));
    alb *= 1.0 + detail * (0.13 * pier + 0.2 * sill - 0.22 * head);
    alb *= 1.0 - 0.78 * w * detail;
    glass = w * detail;
    // by day half the shutters are drawn: those windows nearly vanish into the wall
    if (u_light == 1) glass *= mix(1.0, smoothstep(0.35, 0.62, near + 0.25 * (r1 - 0.5)) * 0.85 + 0.15 * step(r2, 0.3), clus);
  }
  return em;
}

void shade(vec3 p, vec3 n, float m, float t, vec3 rd, out vec3 col, out vec3 warm) {
  bool day = u_light == 1;
  float q = quietAt(v_uv);
  float fw = t * u_pix;
  vec3 toF = vec3(0.0, 0.0, 6.0) - p;
  float fd2 = dot(toF, toF);
  float fN = max(dot(n, toF * inversesqrt(fd2)), 0.0);
  float fAtt = 1.0 / (1.0 + fd2 / (38.0 * 38.0));
  vec2 tg = normalize(vec2(-n.y, n.x) + 1e-5);
  float s = dot(p.xy, tg);
  bool wall = abs(n.z) < 0.5;
  vec3 alb = vec3(0.4), em = vec3(0.0), extra = vec3(0.0);
  float glass = 0.0, wrap = 0.0, street = wall ? 1.0 : 0.15, wf = 0.12;
  g_sink = 0.0;
  if (m < 2.5) {
    vec4 b; vec2 bid;
    blockAt(p.xy, b, bid);
    Lot L = lotAt(p.xy, b, bid);
    float k1 = h21(L.id + 5.5), k2 = h21(L.id + 8.8);
    if (L.kind > 1.5) {
      // a slim tower: dark glass, a few floors working late, a lit crown on some
      alb = day ? vec3(0.62, 0.67, 0.72) : vec3(0.06, 0.07, 0.09);
      if (wall) {
        float fl = p.z / 4.1;
        vec2 id = vec2(floor(s / 8.0), floor(fl));
        float detail = 1.0 - smoothstep(0.3, 1.2, fw);
        float spandrel = 1.0 - smoothstep(0.0, 0.12 + fw * 0.3, abs(fract(fl) - 0.1));
        float mull = 1.0 - smoothstep(0.0, 0.06 + fw * 0.4, abs(fract(s / 1.6) - 0.5) - 0.44);
        float near = vnoise(id * vec2(0.6, 0.35) + L.id);
        float lit = step(h21(id + L.id * 1.7), 0.05 + 0.6 * smoothstep(0.62, 0.85, near));
        vec3 wc = mix(vec3(1.0, 0.78, 0.52), vec3(1.0, 0.88, 0.7), h21(id + 3.3));
        em = wc * lit * (0.55 + 0.45 * h21(id + 1.1)) * mix(0.35, (1.0 - spandrel) * (1.0 - 0.6 * mull), detail) * 0.8;
        glass = mix(0.5, 1.0 - spandrel, detail);
        // the crown
        if (h21(L.id + 3.0) < 0.55) em += vec3(1.0, 0.82, 0.6) * 0.9 * step(L.H - 6.0, p.z) * step(p.z, L.H - 2.2);
        alb *= 1.0 - 0.3 * spandrel * detail;
      }
      street = wall ? 0.6 : 0.0;
    } else {
      // masonry: limestone, brownstone or brick, with a cornice at each tier
      vec3 stone = k1 < 0.4 ? vec3(0.5, 0.45, 0.39) : k1 < 0.75 ? vec3(0.4, 0.26, 0.19) : vec3(0.33, 0.3, 0.29);
      if (day) stone = k1 < 0.4 ? vec3(0.88, 0.82, 0.72) : k1 < 0.75 ? vec3(0.8, 0.6, 0.48) : vec3(0.78, 0.76, 0.72);
      alb = stone * (0.85 + 0.3 * k2);
      vec2 tq = p.xy - L.c - L.tank;
      bool tank = L.tank.x > -1e3 && length(tq) < 3.0 && p.z > L.H + 0.5;
      if (tank) {
        // a water tank: weathered staves, a dark cap
        alb = (day ? vec3(0.5, 0.42, 0.34) : vec3(0.12, 0.1, 0.085)) * (0.8 + 0.3 * step(0.5, fract(atan(tq.y, tq.x) * 6.0)));
        if (p.z > L.H + 6.8) alb *= 0.6;
        street = 0.0;
      } else if (wall && p.z < L.H + 0.3) {
        alb *= 0.88 + 0.12 * vnoise(vec2(s * 0.4, p.z * 0.1));
        vec3 T = L.kind > 0.5 ? tiers(L) : vec3(L.H);
        float top = p.z < T.x + 0.3 ? T.x : p.z < T.y + 0.3 ? T.y : T.z;
        float base = p.z < T.x + 0.3 ? 5.2 : (p.z < T.y + 0.3 ? T.x : T.y) + 0.6;
        em = facade(s, p.z, top, L.id, mix(2.9, 4.2, k2), mix(3.3, 3.9, h21(L.id + 2.0)), base, mix(0.08, 0.15, k2) + 0.06 * L.facing, L.facing > 0.5 ? 1.0 : step(0.7, h21(L.id + 6.1)), 1.0, fw, alb, glass);
        // some crowns are lit from below
        extra = vec3(1.0, 0.7, 0.4) * 0.25 * step(0.8, h21(L.id + 4.2)) * smoothstep(top - 8.0, top - 1.0, p.z);
      } else alb = day ? (k1 < 0.5 ? vec3(0.55, 0.52, 0.5) : vec3(0.66, 0.42, 0.32)) * (0.9 + 0.2 * vnoise(p.xy * 0.8)) : alb * 0.3;
    }
    // the side the copy stands on is let go into the night
    em *= 1.0 - 0.95 * q;
    extra *= 1.0 - q;
    street *= 1.0 - 0.85 * q;
    g_sink = 0.62 * q * (day ? 0.75 : 1.0);
  } else if (m < 3.5) {
    // the trees: a dark mass, lit from beneath where the path lamps stand,
    // and round the plaza by the fountain
    float lf = noise3(p * 0.85) * 0.6 + noise3(p * 2.6) * 0.4;
    alb = day ? mix(vec3(0.46, 0.5, 0.36), vec3(0.58, 0.6, 0.42), lf) * (0.88 + 0.2 * lf) : vec3(0.05, 0.068, 0.045) * (0.45 + 1.1 * lf);
    wrap = day ? 0.85 : 0.6;
    street = 0.0;
    wf = 1.0;
    if (!day) {
      vec2 lp = parkLamp(p.xy);
      vec3 dl = vec3(lp, 4.2) - p;
      float la = 1.0 / (1.0 + dot(dl, dl) / 36.0);
      extra = vec3(1.0, 0.6, 0.27) * la * (0.25 + 0.75 * max(dot(n, normalize(dl)), 0.0)) * 3.2 * (1.0 - 0.9 * q);
      // the crowns' tops catch the city's glow on the cloud
      extra += vec3(0.11, 0.1, 0.13) * smoothstep(0.2, 0.9, n.z) * (0.6 + 0.4 * lf);
      // and the streets' light on the trees along the avenues
      vec2 sl = streetLamp(p.xy);
      vec3 ds = vec3(sl, 8.0) - p;
      extra += vec3(1.0, 0.62, 0.3) * (1.0 / (1.0 + dot(ds, ds) / 60.0)) * 1.8 * (1.0 - 0.9 * q);
    }
  } else {
    float r = length(p.xy);
    alb = (day ? vec3(0.9, 0.86, 0.78) : vec3(0.4, 0.38, 0.35)) * (0.85 + 0.3 * vnoise(vec2(atan(p.y, p.x) * 14.0, p.z * 3.0 + r)));
    street = 0.0;
    // lit from inside by the pool and the falling light
    float inside = r < 7.6 ? 1.0 : smoothstep(0.75, 0.97, p.z) * 0.5;
    if (!day) { vec3 w0 = alb * vec3(1.0, 0.48, 0.18) * inside * (0.12 + 0.5 * exp(-max(p.z - 0.7, 0.0) / 1.4) + (n.z < -0.2 ? 0.35 : 0.0)); col = alb * vec3(0.02, 0.028, 0.045) * (0.7 + 0.3 * n.z); warm = w0 + alb * vec3(1.0, 0.5, 0.2) * 0.25 * (1.0 - inside); return; }
  }
  if (!day) {
    vec3 amb = vec3(0.022, 0.03, 0.055) * (0.65 + 0.35 * n.z);
    // the street's light, from below
    vec3 glow = vec3(1.0, 0.55, 0.24) * 0.07 * exp(-max(p.z, 0.0) / 9.0) * street;
    col = alb * (amb + glow + extra) + em;
    // dark glass keeps a little of the sky
    col += vec3(0.01, 0.014, 0.026) * glass * step(dot(em, vec3(1.0)), 0.02);
    warm = alb * vec3(1.0, 0.5, 0.2) * mix(fN, 0.5 + 0.5 * fN, wrap) * fAtt * 2.6 * wf;
  } else {
    vec3 sun = normalize(SUN);
    float ndl = max(dot(n, sun), 0.0);
    ndl = mix(ndl, 0.45 + 0.55 * ndl, wrap);
    float sh = ndl > 0.001 ? shadow(p + n * 0.1, sun) : 0.0;
    map(p + n * 1.5);
    float ao = clamp(0.5 + 0.5 * g_soft / 1.5, 0.0, 1.0);
    sh = 0.4 + 0.6 * sh;
    vec3 lightC = vec3(0.78, 0.76, 0.76) * (0.86 + 0.14 * n.z) * mix(1.0, ao, 0.6) + vec3(0.5, 0.37, 0.2) * min(ndl * 1.5, 1.0) * sh + vec3(0.08, 0.06, 0.03) * max(-n.z + 0.3, 0.0);
    col = alb * lightC;
    // glass by day: the warm grey of a room behind it, and here and there the sky
    float gk = h21(floor(vec2(s / 3.1, p.z / 3.35)) + 3.0);
    vec3 gl = (gk < 0.22 ? vec3(0.66, 0.6, 0.56) : gk < 0.72 ? vec3(0.5, 0.52, 0.55) : vec3(0.74, 0.79, 0.84)) * (0.7 + 0.35 * sh * ndl);
    col = mix(col, gl * mix(1.0, length(alb) * 0.9, 0.3), glass * 0.2 * (1.0 - smoothstep(0.25, 0.7, fw)));
    warm = vec3(0.0);
  }
}
vec3 fogged(vec3 c, vec3 ro, vec3 rd, float t) {
  return mix(c, hazeCol(rd), mix(fogAmt(ro, rd, t), 1.0, g_sink));
}
void main() {
  bool day = u_light == 1;
  vec3 ro = u_cam, rd = rayDir(v_uv);
  float tG = rd.z < -1e-4 ? -ro.z / rd.z : 1e9;
  // past the modelled city the ground is the sky's: its skyline stands there
  if (tG > 1650.0) tG = 1e9;
  float tMax = min(tG, 2600.0);
  float t = 0.05, m = 0.0;
  bool hit = false;
  for (int i = 0; i < 160; i++) {
    if (i >= u_steps) break;
    vec3 p = ro + rd * t;
    vec2 h = mapM(p);
    if (h.x < 0.004 + min(0.0012 * t, 0.3) && g_guard < 0.5) { hit = true; m = h.y; break; }
    t += h.x * 0.95;
    if (t > tMax || (p.z > 262.0 && rd.z > 0.0)) break;
  }
  if (hit) {
    vec3 p = ro + rd * t;
    vec3 n = calcNormal(p, t);
    vec3 col, warm;
    shade(p, n, m, t, rd, col, warm);
    float f = mix(fogAmt(ro, rd, t), 1.0, g_sink);
    col = fogged(col, ro, rd, t);
    o0 = vec4(col, t); o1 = vec4(warm * (1.0 - f), 1.0); o2 = vec4(col, 0.0);
  } else if (tG < 1e8) {
    g_sink = 0.0;
    vec3 w = ro + rd * tG;
    map(w + vec3(0.0, 0.0, 0.7));
    float a1 = g_soft;
    map(w + vec3(0.0, 0.0, 2.6));
    float ao = clamp(0.28 + 0.72 * a1 / 0.9, 0.0, 1.0) * clamp(0.6 + 0.4 * g_soft / 3.0, 0.0, 1.0);
    if (!day) {
      vec3 r = vec3(rd.xy, -rd.z);
      vec3 q0 = w + vec3(0.0, 0.0, 0.04);
      float tr = 0.1, mr = 0.0;
      bool hr = false;
      for (int i = 0; i < 90; i++) {
        if (i * 2 >= u_steps + 30) break;
        vec3 p = q0 + r * tr;
        vec2 h = mapM(p);
        if (h.x < 0.006 + min(0.0014 * (tr + tG), 0.3) && g_guard < 0.5) { hr = true; mr = h.y; break; }
        tr += h.x * 0.95;
        if (tr > 900.0 || p.z > 262.0) break;
      }
      vec3 rc, rw = vec3(0.0);
      if (hr) {
        vec3 p = q0 + r * tr;
        vec3 n = calcNormal(p, tr + tG);
        shade(p, n, mr, tr + tG, r, rc, rw);
        float f = mix(fogAmt(q0, r, tr), 1.0, g_sink);
        rc = fogged(rc, q0, r, tr); rw *= 1.0 - f;
      } else rc = sky(r);
      o0 = vec4(0.0); o1 = vec4(rw, ao); o2 = vec4(rc, 0.0);
    } else {
      float sh = shadow(w + vec3(0.0, 0.0, 0.06), normalize(SUN));
      o0 = vec4(0.0); o1 = vec4(0.0, 0.0, 0.0, ao); o2 = vec4(sh, 0.0, 0.0, 0.0);
    }
  } else {
    vec3 c = sky(rd);
    o0 = vec4(c, -1.0); o1 = vec4(0.0); o2 = vec4(c, 0.0);
  }
}`;

// Each frame: the paving (wet, and mirroring what stands on it), the lamps,
// the pool, the people, the fountain's liquid, and then the liquid sky over
// all of it.
export const SCENE_FS = HEAD + `out vec4 o;` + LIB + `
uniform sampler2D u_s0; uniform sampler2D u_s1; uniform sampler2D u_s2; uniform sampler2D u_f0; uniform sampler2D u_f1; uniform sampler2D u_f2;
uniform sampler2D u_l0; uniform sampler2D u_l1; uniform sampler2D u_ln;
uniform vec2 u_fScale; uniform float u_sharp; uniform float u_rel;
// What a texel of the city holds: a wall (1), paving (0) or sky (2).
float kind(float a) { return a > 0.0 ? 1.0 : a < -0.5 ? 2.0 : 0.0; }
// While the camera moves the city is marched small. Enlarging it, each texel
// counts by how near it is and only if it is the same kind of thing as the
// nearest one, so a wall never bleeds into the paving beside it.
void smallCity(vec2 uv, out vec4 a0, out vec4 a1, out vec4 a2) {
  vec2 sz = u_fScale * vec2(textureSize(u_f0, 0));
  vec2 p = uv * sz - 0.5;
  ivec2 b = ivec2(floor(p)), hi = ivec2(sz) - 1;
  vec2 f = p - floor(p);
  float kn = kind(texelFetch(u_f0, clamp(ivec2(uv * sz), ivec2(0), hi), 0).a);
  a0 = vec4(0.0); a1 = vec4(0.0); a2 = vec4(0.0);
  float ws = 0.0;
  for (int j = 0; j < 2; j++) for (int i = 0; i < 2; i++) {
    ivec2 q = clamp(b + ivec2(i, j), ivec2(0), hi);
    vec4 t0 = texelFetch(u_f0, q, 0);
    float w = ((i == 0 ? 1.0 - f.x : f.x) * (j == 0 ? 1.0 - f.y : f.y) + 1e-4) * (kind(t0.a) == kn ? 1.0 : 0.0);
    a0 += t0 * w; a1 += texelFetch(u_f1, q, 0) * w; a2 += texelFetch(u_f2, q, 0) * w; ws += w;
  }
  a0 /= ws; a1 /= ws; a2 /= ws;
}
uniform float u_t; uniform float u_surge; uniform float u_scene; uniform float u_breath; uniform float u_fh; uniform float u_fr; uniform float u_dpr;
uniform vec4 u_walk[${NW}]; uniform vec4 u_walkScr[${NW}];
vec3 ember(float x) {
  vec3 c0 = vec3(0.07, 0.02, 0.009), c1 = vec3(0.32, 0.065, 0.018), c2 = vec3(1.0, 0.415, 0.17), c3 = vec3(1.0, 0.64, 0.28), c4 = vec3(1.0, 0.85, 0.56);
  x = clamp(x, 0.0, 1.0);
  if (x < 0.22) return mix(c0, c1, x / 0.22);
  if (x < 0.5) return mix(c1, c2, (x - 0.22) / 0.28);
  if (x < 0.78) return mix(c2, c3, (x - 0.5) / 0.28);
  return mix(c3, c4, (x - 0.78) / 0.22);
}
float g_lheat, g_veil;
float sdLiquid(vec3 p) {
  float r = length(p.xy);
  float H = u_fh, RL = u_fr;
  // the jet, and the crown where it turns over
  float k = clamp((p.z - 4.0) / (H - 4.0), 0.0, 1.0);
  vec2 wob = 0.1 * k * vec2(sin(p.z * 1.3 - u_t * 5.0), cos(p.z * 1.1 - u_t * 4.3));
  float jr = mix(0.72, 0.42, k) * (1.0 + 0.16 * sin(p.z * 3.1 - u_t * 9.0));
  float d = max(length(p.xy - wob) - jr, max(4.0 - p.z, p.z - H));
  d = min(d, length(p - vec3(0.0, 0.0, H)) - 0.95);
  g_lheat = 0.97;
  // eight streams arcing over and falling into the pool
  float a = atan(p.y, p.x), sec = PI / 4.0;
  float kk = floor(a / sec + 0.5);
  float af = a - kk * sec;
  vec2 q = vec2(r * cos(af), r * sin(af));
  float c1 = 1.15, c2 = (H - 0.72 + c1 * RL) / (RL * RL);
  float zp = H + c1 * q.x - c2 * q.x * q.x;
  float sl = c1 - 2.0 * c2 * q.x;
  float dz = (p.z - zp) / sqrt(1.0 + sl * sl);
  float x = clamp(q.x / RL, 0.0, 1.0);
  // each stream swells and thins as it falls, and breaks into beads near the pool
  float bead = sin(q.x * 2.6 - u_t * 7.0 + kk * 1.9) + 0.4 * sin(q.x * 6.1 - u_t * 11.0 + kk * 4.1);
  float th = mix(0.62, 0.25, x) * (0.8 + (0.1 + 0.2 * x * x) * bead);
  float lat = q.y - 0.07 * x * sin(q.x * 2.1 - u_t * 5.0 + kk * 2.7);
  float da = max(length(vec2(dz, lat)) - th, max(q.x - RL - 0.1, 0.7 - p.z)) * 0.85;
  if (da < d) { d = da; g_lheat = 0.94 - 0.42 * x; }
  // the veil between the streams near the crown, for the glow only
  g_veil = abs(p.z - (H + c1 * r - c2 * r * r)) / sqrt(1.0 + pow(c1 - 2.0 * c2 * r, 2.0)) + 6.0 * smoothstep(RL * 0.35, RL, r) + max(0.72 - p.z, 0.0) * 9.0;
  return d;
}
// The fountain along a ray: its surface if the ray meets it, and the glow round it.
bool traceLiquid(vec3 ro, vec3 rd, float tmax, float glowF, out vec3 c, out vec3 glowC) {
  bool day = u_light == 1;
  c = vec3(0.0); glowC = vec3(0.0);
  float aq = dot(rd.xy, rd.xy), bq = dot(ro.xy, rd.xy), cq = dot(ro.xy, ro.xy) - 8.2 * 8.2;
  float disc = bq * bq - aq * cq;
  if (disc <= 0.0 || aq < 1e-6) return false;
  float sq = sqrt(disc);
  float tt = max((-bq - sq) / aq, 0.0), t1 = min((-bq + sq) / aq, tmax);
  float glow = 0.0, veil = 0.0;
  bool hit = false;
  for (int i = 0; i < 64; i++) {
    if (tt > t1) break;
    vec3 p = ro + rd * tt;
    float d = sdLiquid(p);
    float stp = max(d * 0.72, 0.04);
    glow += exp(-max(d, 0.0) * 2.6) * min(stp, 0.5);
    veil += exp(-g_veil * 5.0) * min(stp, 0.5);
    if (d < 0.006 + 0.0008 * tt) { hit = true; break; }
    tt += stp;
  }
  if (!day) glowC = (vec3(1.0, 0.46, 0.16) * glow * 0.032 + vec3(1.0, 0.4, 0.13) * veil * 0.22) * glowF;
  else glowC = vec3(veil * 0.1);
  if (!hit) return false;
  vec3 p = ro + rd * tt;
  float heat = g_lheat;
  vec2 e = vec2(1.0, -1.0) * 0.025;
  vec3 n = normalize(e.xyy * sdLiquid(p + e.xyy) + e.yyx * sdLiquid(p + e.yyx) + e.yxy * sdLiquid(p + e.yxy) + e.xxx * sdLiquid(p + e.xxx));
  float nv = max(dot(n, -rd), 0.0);
  float flow = fbm(vec2(atan(p.y, p.x) * 3.0 + p.z * 0.4, p.z * 1.1 + length(p.xy) * 1.2 - u_t * 3.0));
  vec3 Ld = normalize(vec3(-0.45, -0.5, 0.74));
  float diff = max(dot(n, Ld), 0.0);
  float spec = pow(max(dot(reflect(-Ld, n), -rd), 0.0), 36.0);
  if (!day) {
    // molten: bright where the eye looks into its depth, ember at the grazing edge
    float temp = clamp(heat * (0.6 + 0.5 * flow) + 0.16 * u_surge, 0.0, 1.0);
    c = ember(temp * 0.72 + 0.26 * nv * nv) * (0.3 + 1.0 * nv) * (0.42 + 0.9 * temp * temp) + vec3(1.0, 0.9, 0.72) * (spec * 0.9 + 0.1 * pow(1.0 - nv, 3.0));
  } else {
    float thr = fbm(vec2(atan(p.y, p.x) * 5.0, p.z * 0.3 + length(p.xy) * 0.35 - u_t * 1.1));
    float ink = smoothstep(0.52, 0.64, thr) * (0.55 + 0.45 * flow);
    c = mix(vec3(0.88, 0.94, 0.95), vec3(0.52, 0.66, 0.72), pow(1.0 - nv, 1.5)) * (0.84 + 0.18 * diff);
    c = mix(c, mix(vec3(0.88, 0.5, 0.31), vec3(0.7, 0.3, 0.15), thr), ink * 0.88) + vec3(1.0) * spec * 0.8;
  }
  return true;
}
// the fountain mirrored in wet stone: where the reflected ray passes its axis
float fountainRefl(vec3 w, vec3 r, float rough) {
  float l = length(r.xy);
  vec2 rh = r.xy / max(l, 1e-4);
  float along = dot(-w.xy, rh);
  if (along < 0.0) return 0.0;
  float perp = abs(w.x * rh.y - w.y * rh.x);
  float zc = w.z + r.z / max(l, 1e-4) * along;
  float sp = rough * along;
  float s1 = 0.42 + 0.05 * sp, s2 = 2.4 + 0.06 * sp;
  float colm = exp(-perp * perp / (2.0 * s1 * s1)) * (1.0 - smoothstep(u_fh - 1.5, u_fh + 1.0 + 0.5 * sp, zc)) * (0.42 / s1);
  float umb = exp(-perp * perp / (2.0 * s2 * s2)) * (1.0 - smoothstep(u_fh - 4.0, u_fh + 1.0 + 0.5 * sp, zc)) * 0.3;
  return (colm + umb) * smoothstep(-0.5, 1.2, zc);
}
float lobe(vec3 r, vec3 l, float rough) {
  float dl = length(l);
  l /= dl;
  vec2 rh = normalize(r.xy), lh = normalize(l.xy);
  float cr = rh.x * lh.y - rh.y * lh.x, dz = r.z - l.z;
  float sa = 0.011 + 0.028 * rough, se = 0.018 + 0.2 * rough;
  return step(0.0, dot(rh, lh)) * exp(-cr * cr / (2.0 * sa * sa) - dz * dz / (2.0 * se * se)) * 0.0011 / (sa * se) / (1.0 + dl * dl * 0.0005);
}
// The lamps mirrored in wet stone: the nearest of each kind to where the
// mirrored ray passes their height.
float lampRefl(vec2 w, vec3 r, float rough) {
  if (r.z < 1e-3) return 0.0;
  vec2 pl = parkLamp(w + r.xy * (4.2 / r.z));
  vec2 sl = streetLamp(w + r.xy * (8.0 / r.z));
  return lobe(r, vec3(pl, 4.2) - vec3(w, 0.0), rough) + 1.3 * lobe(r, vec3(sl, 8.0) - vec3(w, 0.0), rough);
}
// A lamp's globe where the ray passes it, sized to at least a pixel there.
float lampHead(vec3 ro, vec3 rd, vec3 L, float tHit) {
  vec3 v = L - ro;
  float tl = dot(v, rd);
  if (tl <= 0.0 || tl > tHit + 0.6) return 0.0;
  float d = length(v - rd * tl);
  float rg = max(0.24, tl * u_pix * 0.75);
  return ((0.24 * 0.24) / (rg * rg) * exp(-d * d / (2.0 * rg * rg)) + 0.04 * exp(-d / (rg * 5.0))) * (0.3 + 0.7 * smoothstep(100.0, 230.0, tl));
}
float lamps(vec3 ro, vec3 rd, float tHit) {
  if (rd.z > -1e-4) return 0.0;
  float a = lampHead(ro, rd, vec3(parkLamp((ro + rd * ((4.2 - ro.z) / rd.z)).xy), 4.2), tHit);
  float b = lampHead(ro, rd, vec3(streetLamp((ro + rd * ((8.0 - ro.z) / rd.z)).xy), 8.0), tHit);
  return a + 1.5 * b;
}
// Warm air: the fountain's and the lamps' light held in the mist low over
// the park and along the avenues, gathered at a few jittered points where the
// ray passes under 26 m.
vec3 airGlow(vec3 ro, vec3 rd, float tHit, float glowF, float jit) {
  if (rd.z > -1e-4) return vec3(0.0);
  float t0 = max(0.0, (26.0 - ro.z) / rd.z), t1 = min(tHit, -ro.z / rd.z);
  if (t1 <= t0) return vec3(0.0);
  vec3 acc = vec3(0.0);
  float dt = (t1 - t0) / 6.0;
  for (int i = 0; i < 6; i++) {
    vec3 p = ro + rd * (t0 + (float(i) + jit) * dt);
    float dens = exp(-p.z / 8.0);
    vec3 f = p - vec3(0.0, 0.0, 5.0);
    float fl = glowF * 3.2 / (1.0 + dot(f, f) / (18.0 * 18.0));
    vec3 dl = p - vec3(parkLamp(p.xy), 4.2);
    float ll = inPark(p.xy) ? 1.1 / (1.0 + dot(dl, dl) / 80.0) : 0.0;
    vec3 ds = p - vec3(streetLamp(p.xy), 8.0);
    float sl = 0.8 / (1.0 + dot(ds, ds) / 110.0);
    acc += (vec3(1.0, 0.48, 0.18) * fl + vec3(1.0, 0.6, 0.28) * (ll + sl)) * dens * dt;
  }
  return acc * 0.0042;
}
// Rills of light in the paving, running out from the basin on eight lines.
// At the close they breathe, and run on down the paths.
vec2 rill(vec2 w, float r) {
  float sec = PI / 4.0;
  float a = atan(w.y, w.x);
  float k = floor(a / sec + 0.5);
  float af = a - k * sec;
  float lat = r * sin(af), along = r * cos(af);
  float len = PLR - 3.0 + 64.0 * u_breath;
  float fwl = fwidth(lat);
  float run = smoothstep(9.0, 9.8, along) * (1.0 - smoothstep(len - 12.0, len, along)) * exp(-max(along - PLR, 0.0) / 45.0);
  float m = lineL(abs(lat), 0.1 + 0.22 * u_breath, fwl) * run;
  float flow = fbm(vec2(along * 0.4 - u_t * 0.8, k * 7.0));
  float ts = mod(u_t - 8.3, 10.0);
  float pulse = exp(-pow((along - 9.0 - 8.5 * ts) / 4.5, 2.0)) * exp(-ts / 2.3);
  float heat = 0.3 + 0.5 * flow + 1.3 * pulse + u_breath * (0.75 + 0.45 * sin(u_t * 0.9 - along * 0.15));
  float halo = exp(-lat * lat / (0.6 + 7.0 * u_breath)) * run;
  return vec2(m * heat, halo * heat);
}
vec3 poolCol(vec2 w, vec3 ro, vec3 rd, bool day) {
  float r = length(w);
  float n = fbm(w * 0.6 + vec2(0.0, u_t * 0.14)), n2 = fbm(w * 1.7 - vec2(u_t * 0.2, 0.0));
  float rip = sin((r - u_fr) * 5.0 - u_t * 6.0) * exp(-abs(r - u_fr) * 0.8) + 0.7 * sin((r - 2.8) * 6.0 - u_t * 7.0) * exp(-abs(r - 2.8) * 1.1);
  vec3 rr = vec3(rd.xy, -rd.z);
  float fr = fountainRefl(vec3(w, 0.72), rr + vec3(0.06 * rip, 0.06 * (n2 - 0.5), 0.0), 0.08);
  if (day) {
    vec3 c = mix(vec3(0.78, 0.86, 0.87), vec3(0.9, 0.94, 0.94), 0.5 + 0.5 * rip * 0.6);
    float ink = smoothstep(0.42, 0.7, n * 0.7 + n2 * 0.4 + 0.12 * rip);
    c = mix(c, mix(vec3(0.84, 0.45, 0.26), vec3(0.62, 0.25, 0.12), smoothstep(0.5, 0.9, n)), ink * 0.85);
    return c + vec3(1.0) * fr * 0.25;
  }
  float heat = 0.42 + 0.34 * n + 0.14 * rip + 0.3 * u_surge + 0.22 * exp(-r * 0.3);
  float spk = pow(vnoise(w * 9.0 + vec2(u_t * 7.0, -u_t * 5.0)), 9.0) * exp(-abs(r - u_fr) * 1.6) * 14.0;
  return ember(heat * 0.9) * (0.45 + 0.6 * heat) + vec3(1.0, 0.86, 0.62) * (fr * 0.8 + spk);
}
void main() {
  bool day = u_light == 1;
  vec3 col = vec3(0.0);
  vec2 px = v_uv * u_res;
  vec4 lq = texture(u_l0, v_uv);
  if (u_scene > 0.5 && lq.w < 0.995) {
    vec3 ro = u_cam, rd = rayDir(v_uv);
    float q = quietAt(v_uv);
    // the city: sharp once the camera rests, the small one while it moves, and a
    // cross-fade from one to the other
    vec4 s0, s1, s2;
    if (u_sharp < 1.0) smallCity(v_uv, s0, s1, s2);
    if (u_sharp > 0.0) {
      ivec2 sp = min(ivec2(gl_FragCoord.xy), textureSize(u_s0, 0) - 1);
      vec4 h0 = texelFetch(u_s0, sp, 0), h1 = texelFetch(u_s1, sp, 0), h2 = texelFetch(u_s2, sp, 0);
      float k = u_sharp >= 1.0 ? 1.0 : kind(h0.a) == kind(s0.a) ? u_sharp : step(0.5, u_sharp);
      if (u_sharp >= 1.0) { s0 = h0; s1 = h1; s2 = h2; } else { s0 = mix(s0, h0, k); s1 = mix(s1, h1, k); s2 = mix(s2, h2, k); }
    }
    float glowF = 1.0 + 1.25 * u_surge;
    float tG = -ro.z / min(rd.z, -1e-4);
    vec2 w = ro.xy + rd.xy * tG;
    float fx = fwidth(w.x), fy = fwidth(w.y);
    float tHit = 1e9;
    if (s0.a > 0.0) { col = s0.rgb + s1.rgb * glowF; tHit = s0.a; }
    else if (s0.a < -0.5 || rd.z > -1e-4) col = s0.rgb;
    else {
      tHit = tG;
      float r = length(w);
      bool park = inPark(w);
      float al;
      float po = pathOff(w, al);
      bool plaza = r < PLR;
      bool path = park && !plaza && al > 0.0 && po < PW;
      bool lawn = park && !plaza && !path;
      float face = streetFace(w);
      vec3 alb;
      float wetK = 1.0;
      vec2 tilt = vec2(0.0);
      float fade = 1.0 - smoothstep(0.25, 0.9, max(fx, fy));
      if (plaza || path) {
        // stone laid in rings round the basin, and in long slabs down the paths
        vec2 g = plaza ? vec2(r / 1.8, atan(w.y, w.x) * r / 2.4) : vec2(al / 3.0, (dot(w, vec2(-sin(floor(atan(w.y, w.x) / (PI / 4.0) + 0.5) * PI / 4.0), cos(floor(atan(w.y, w.x) / (PI / 4.0) + 0.5) * PI / 4.0)))) / 1.3 + 7.0);
        vec2 sid = floor(g);
        vec2 jd = (0.5 - abs(fract(g) - 0.5));
        float joint = (1.0 - smoothstep(0.02, 0.06 + max(fx, fy) * 0.6, min(jd.x, jd.y))) * fade;
        float hv = mix(0.5, h21(sid), fade);
        tilt = (vec2(h21(sid + 1.7), h21(sid + 5.3)) - 0.5) * fade;
        alb = (day ? vec3(0.93, 0.89, 0.81) * (0.955 + 0.09 * hv) : vec3(0.2, 0.19, 0.185) * (0.84 + 0.32 * hv)) * (1.0 - (day ? 0.14 : 0.5) * joint);
        // a darker collar of stone round the basin
        float collar = 1.0 - smoothstep(12.0, 12.4, r);
        alb = mix(alb, (day ? vec3(0.8, 0.74, 0.66) : vec3(0.1, 0.1, 0.106)) * (0.85 + 0.3 * hv), collar);
      } else if (lawn) {
        alb = (day ? mix(vec3(0.56, 0.6, 0.42), vec3(0.64, 0.64, 0.44), vnoise(w * 0.12)) : vec3(0.012, 0.02, 0.011)) * (0.92 + 0.16 * vnoise(w * 1.3));
        wetK = 0.04;
      } else if (face > 0.0 && face < 4.5) {
        // the pavement
        alb = (day ? vec3(0.8, 0.78, 0.74) : vec3(0.1, 0.1, 0.105)) * (0.88 + 0.24 * vnoise(w * 0.6));
        wetK = 0.7;
      } else {
        // the road: dark asphalt, wetter than anything
        alb = (day ? vec3(0.46, 0.46, 0.47) : vec3(0.035, 0.036, 0.04)) * (0.85 + 0.3 * vnoise(w * 0.3));
        wetK = 1.25;
      }
      float ao = s1.a;
      float fog = fogAmt(ro, rd, tG);
      if (!day) {
        float pud = smoothstep(0.46, 0.66, fbm(w * 0.06 + 11.0));
        float cosv = max(-rd.z, 0.0);
        float F = 0.07 + 0.93 * pow(1.0 - cosv, 4.0);
        float refl = clamp(F * mix(0.55, 1.0, pud) * wetK, 0.0, 1.0);
        float rough = mix(1.0, 0.15, pud);
        vec3 rr = normalize(vec3(rd.xy, -rd.z) + vec3(tilt * 0.01 * rough, 0.0));
        // what stands on the stone, smeared down it
        vec3 R = vec3(0.0), RW = vec3(0.0);
        float ws = 0.0;
        vec2 off = tilt * 0.0018 * rough;
        float jit = fract(52.9829189 * fract(dot(px, vec2(0.06711056, 0.00583715))));
        for (int k = 0; k < 6; k++) {
          float fk = (float(k) + jit) / 3.0 - 1.0;
          vec2 uv2 = clamp(v_uv + off + vec2(0.0, fk * 0.02 * rough), 0.0, 1.0);
          float wk = exp(-fk * fk * 2.2);
          if (u_sharp >= 0.5) { R += texture(u_s2, uv2).rgb * wk; RW += texture(u_s1, uv2).rgb * wk; }
          else { R += texture(u_f2, uv2 * u_fScale).rgb * wk; RW += texture(u_f1, uv2 * u_fScale).rgb * wk; }
          ws += wk;
        }
        R = (R + RW * glowF) / ws;
        vec3 L = vec3(0.022, 0.03, 0.055);
        float lampOn = 1.0 - 0.9 * q;
        vec2 dp = w - parkLamp(w);
        if (park) L += vec3(1.0, 0.62, 0.3) * pow(1.0 + dot(dp, dp) / 26.0, -1.5) * 0.55 * lampOn;
        vec2 ds = w - streetLamp(w);
        L += vec3(1.0, 0.6, 0.27) * pow(1.0 + dot(ds, ds) / 50.0, -1.5) * 0.75 * lampOn;
        // the fountain's light on the plaza, and the shop windows' on the pavement
        L += vec3(1.0, 0.48, 0.18) * glowF * 0.55 / (1.0 + r * r / (16.0 * 16.0)) * smoothstep(7.4, 10.0, r);
        if (face > 0.0) L += vec3(1.0, 0.58, 0.26) * 0.3 * exp(-max(face, 0.0) / 3.0) * lampOn;
        if (park) L += vec3(1.0, 0.5, 0.2) * glowF * 0.18 / (1.0 + r * r / (55.0 * 55.0));
        float wr = 0.0;
        for (int i = 0; i < ${NW}; i++) {
          vec4 qw = u_walk[i];
          vec2 d = w - qw.xy;
          L += vec3(1.0, 0.66, 0.36) * qw.z * 0.5 * exp(-dot(d, d) / 2.6);
          wr += qw.z * lobe(rr, vec3(qw.xy, 0.8) - vec3(w, 0.0), rough);
        }
        vec3 c = alb * L * ao;
        vec3 mirror = R + vec3(1.0, 0.62, 0.3) * (lampRefl(w, rr, rough) * 2.6 * lampOn + wr * 1.5);
        if (r < PLR + 30.0) {
          // the fountain itself, mirrored: its true image, broken by the lie of each slab
          vec3 fc, fg;
          vec3 rj = normalize(rr + vec3(0.0, 0.0, (jit - 0.5) * 0.05 * rough));
          bool fh = traceLiquid(vec3(w, 0.02), rj, 120.0, glowF, fc, fg);
          mirror += (fh ? fc : vec3(0.0)) + fg * 1.6 + vec3(1.0, 0.62, 0.3) * fountainRefl(vec3(w, 0.0), rr, rough) * glowF * 0.22;
        }
        c = c * (1.0 - 0.7 * refl) + mirror * refl;
        if (park && r < PLR + 70.0) {
          vec2 rl = rill(w, r);
          c += (ember(0.3 + 0.5 * rl.x) * rl.x * 2.2 + vec3(1.0, 0.45, 0.16) * rl.y * 0.16 * max(alb, vec3(0.06)) * 5.0) * (lawn ? 0.6 : 1.0);
        }
        col = mix(c, hazeCol(rd), fog);
      } else {
        float sh = s2.r;
        // people's long shadows
        float ps = 0.0;
        vec3 sun = normalize(SUN);
        for (int i = 0; i < ${NW}; i++) {
          vec2 a = u_walk[i].xy, b = a - sun.xy / sun.z * 1.75;
          vec2 pa = w - a, ba = b - a;
          float hq = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
          ps = max(ps, (1.0 - smoothstep(0.1, 0.3 + 0.3 * hq, length(pa - ba * hq))) * (1.0 - 0.5 * hq));
        }
        sh *= 1.0 - 0.95 * ps;
        vec3 c = alb * (vec3(0.78, 0.76, 0.77) * (0.86 + 0.14 * ao) + vec3(0.42, 0.32, 0.17) * (0.4 + 0.6 * sh));
        if (park && r < PLR + 70.0) {
          vec2 rl = rill(w, r);
          c = mix(c, vec3(0.74, 0.34, 0.17), clamp(rl.x * 1.2 + rl.y * 0.25 * u_breath, 0.0, 0.85));
        }
        col = mix(c, hazeCol(rd), fog);
      }
    }
    // the lamps' globes, and the warm air round them and the fountain
    if (!day) {
      col += vec3(1.0, 0.72, 0.42) * lamps(ro, rd, tHit) * 2.4 * (1.0 - 0.92 * q) * (1.0 - 0.5 * fogAmt(ro, rd, min(tHit, 2000.0)));
      float ign = fract(52.9829189 * fract(dot(px, vec2(0.06711056, 0.00583715))));
      col += airGlow(ro, rd, tHit, glowF, ign) * (1.0 - 0.85 * q);
    }
    // the pool inside the basin
    if (rd.z < -1e-4) {
      float tP = (0.72 - ro.z) / rd.z;
      vec2 wp = ro.xy + rd.xy * tP;
      if (tP > 0.0 && tP < tHit && dot(wp, wp) < 7.5 * 7.5) { col = poolCol(wp, ro, rd, day); tHit = tP; }
    }
    // people: at this distance a stroke with a head, and the light it carries
    for (int i = 0; i < ${NW}; i++) {
      vec4 qw = u_walkScr[i];
      if (qw.w <= 0.0 || qw.w > tHit) continue;
      vec2 d = px - qw.xy;
      float ppm = qw.z;
      if (abs(d.x) > 3.0 * ppm + 6.0 || abs(d.y) > 3.0 * ppm + 6.0) continue;
      float sgn = fract(float(i) * 0.5) < 0.25 ? 1.0 : -1.0;
      vec2 fd = (d - vec2(-0.2 * sgn * ppm, 0.0)) / ppm;
      float yb = fd.y + 0.8;
      float wdt = yb < 0.8 ? mix(0.06, 0.115, yb / 0.8) : mix(0.13, 0.085, clamp((yb - 0.8) / 0.55, 0.0, 1.0));
      float bd = max(abs(fd.x) - wdt, max(-yb, yb - 1.42));
      bd = min(bd, length(vec2(fd.x, yb - 1.53)) - 0.085);
      float soft = clamp(1.6 / ppm, 0.035, 0.6);
      float body = (1.0 - smoothstep(-soft, soft, bd)) * smoothstep(1.5, 5.0, ppm);
      float bri = u_walk[i].z;
      if (day) { col = mix(col, vec3(0.36, 0.32, 0.31), body * 0.6); continue; }
      col = mix(col, vec3(0.012, 0.012, 0.018), body * 0.6);
      float rc = max(0.055 * ppm, 0.75 * u_dpr);
      float dd = dot(d, d);
      col += vec3(1.0, 0.7, 0.4) * bri * (2.6 * exp(-dd / (2.0 * rc * rc)) * min(1.0, 0.055 * ppm / rc + 0.25) + 0.12 * exp(-dd / (2.0 * 0.35 * 0.35 * ppm * ppm + 4.0)));
    }
    // the fountain's liquid
    {
      vec3 lc, lg;
      bool hitL = traceLiquid(ro, rd, tHit, glowF, lc, lg);
      if (day) { if (hitL) col = mix(col, lc, 0.93); else col = mix(col, vec3(0.9, 0.94, 0.94), clamp(lg.r, 0.0, 0.5)); }
      else { col += lg; if (hitL) col = lc; }
    }
  }
  // ---- the liquid sky: silk at rest, molten where the plume passes
  vec4 l1 = texture(u_l1, v_uv);
  vec3 n = normalize(vec3(texture(u_ln, v_uv).xy * u_rel, 1.0));
  vec3 Ld = normalize(vec3(-0.5, 0.6, 0.62));
  float diff = max(dot(n, Ld), 0.0);
  float rv = max(dot(reflect(-Ld, n), vec3(0.0, 0.0, 1.0)), 0.0);
  float spec = pow(rv, 24.0), sheen = pow(rv, 5.0);
  float E = max(lq.y, 0.0), temp = clamp(lq.z, 0.0, 1.0), cov = lq.w, tone = l1.x, vein = l1.y;
  vec3 liquid;
  if (!day) {
    vec3 base = mix(vec3(0.026, 0.035, 0.094), vec3(0.078, 0.036, 0.02), tone);
    vec3 silk = base * (0.2 + 1.55 * diff * diff) + mix(vec3(0.13, 0.17, 0.33), vec3(0.36, 0.2, 0.105), tone) * (0.22 * sheen + 0.34 * spec);
    silk += vec3(1.0, 0.36, 0.1) * (vein * 0.8 + l1.z * 0.07);
    float I = 1.0 - exp(-E * 1.3);
    vec3 hot = ember(temp * 0.85 + I * 0.15) * I * (0.3 + 0.85 * diff) * (0.34 + 1.3 * pow(temp, 1.3)) + vec3(1.0, 0.76, 0.5) * spec * I * (0.12 + 0.55 * temp);
    liquid = silk * (1.0 - 0.9 * I) + hot;
  } else {
    vec3 cream = mix(vec3(0.957, 0.94, 0.908), vec3(0.93, 0.9, 0.86), tone * 0.6);
    liquid = mix(vec3(0.76, 0.72, 0.725), cream, 0.15 + 0.85 * smoothstep(0.15, 0.9, diff)) + vec3(1.0) * spec * 0.04;
    // a blush of terracotta where the stone is warm
    liquid = mix(liquid, vec3(0.94, 0.78, 0.68), tone * 0.3 * smoothstep(0.3, 0.8, diff));
    liquid = mix(liquid, mix(vec3(0.72, 0.6, 0.54), vec3(0.84, 0.52, 0.36), tone), clamp(l1.z, 0.0, 1.0) * 0.6);
    liquid = mix(liquid, mix(vec3(0.86, 0.5, 0.31), vec3(0.68, 0.3, 0.15), tone), clamp(vein * 2.6, 0.0, 0.92));
    float A = 1.0 - exp(-E * 1.25);
    vec3 ink = temp > 0.45 ? mix(vec3(0.62, 0.22, 0.08), vec3(0.93, 0.38, 0.15), (temp - 0.45) / 0.55) : mix(vec3(0.24, 0.12, 0.07), vec3(0.62, 0.22, 0.08), temp / 0.45);
    ink = mix(vec3(0.93, 0.38, 0.15), ink, smoothstep(0.25, 0.9, A));
    liquid = mix(liquid, ink * (0.92 + 0.12 * diff), A * 0.92) + vec3(1.0, 0.98, 0.94) * spec * A * 0.2;
  }
  col = mix(col, liquid, cov);
  o = vec4(col, 1.0);
}`;
