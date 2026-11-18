// The place, for the journey (journey.ts): a night square in the spirit of
// Madison Square Park. A fountain on the axis, a flat-iron wedge of a building behind it
// with avenues running off either side, a clock tower over the roofs, trees
// and lamps to left and right. It is ray-marched once per camera move into
// three buffers; the frame itself only pays for the wet paving, the people,
// the fountain's liquid and the bloom.

export const NW = 30; // walkers

export const LIB = `
#define PI 3.14159265
uniform vec3 u_cam; uniform vec3 u_fwd; uniform vec3 u_right; uniform vec3 u_up; uniform float u_tanf; uniform vec2 u_res; uniform int u_light; uniform float u_pix; uniform float u_quiet;
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

// ---- the plan. z is up; the camera looks along +y.
const float APEXY = 63.5;                 // where the wedge's two faces would meet
const float PR = 1.7;                     // its rounded prow
const float WL = 86.0, WH = 67.0;         // its length and height
const vec2 WN = vec2(0.97437, -0.22495);  // a face's outward normal (the right one)
const vec2 WT = vec2(0.22495, 0.97437);
const float AVW = 24.0;                   // an avenue's width
const float FRONT = 58.0;                 // the far kerb of the cross street
const float PLAZA = 30.0;                 // half the paved width; lawns and trees beyond
const float KERB = 44.0;                  // the park's edge
const float LS = 58.0, LD = 44.0, LU = 30.0; // a block: row pitch, depth, lot width
const vec3 TOWER = vec3(70.0, 215.0, 110.0);
const vec3 SUN = vec3(-0.8, -0.47, 0.375); // late afternoon: low, from behind the viewer's left shoulder
float sdWedge2(vec2 p) {
  vec2 q = vec2(abs(p.x), p.y - APEXY);
  float d = dot(q, WT) < 0.0 ? length(q) : dot(q, WN);
  return max(d - PR, p.y - (APEXY - 7.6) - WL);
}
// The copy's side of the wide hero is kept quiet (u_quiet): no lamps, no lit
// arcade, the block behind the words left to the haze. 1 where a point is on it.
float quietAt(float x) { return u_quiet * smoothstep(-16.0, -26.0, x); }
// the park's lamps: two rows a side, every sixteen metres
vec3 nearLamp(vec2 w) {
  if (w.x < 0.0 && u_quiet > 0.5) return vec3(-9000.0, 0.0, 4.55);
  float row = abs(w.x) < 41.0 ? 27.0 : 55.0;
  return vec3(sign(w.x) * row, clamp(floor(w.y / 16.0 + 0.5), -5.0, 2.0) * 16.0, row < 40.0 ? 4.55 : 3.9);
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
struct Lot { vec2 id; float H; float H2; vec2 lp; float s; float u; float gu; float gab; };
Lot lotAt(vec2 xy) {
  Lot L;
  vec2 q = vec2(abs(xy.x), xy.y - APEXY);
  L.s = dot(q, WN) - PR - AVW; L.u = dot(q, WT);
  float side = xy.x < 0.0 ? 0.0 : 1.0;
  float cs = floor(L.s / LS), cu = floor(L.u / LU);
  L.lp = vec2(L.s - cs * LS, L.u - cu * LU);
  L.gu = min(L.lp.y, LU - L.lp.y);
  L.id = vec2(cs, cu) + vec2(side * 37.0, 11.0);
  float hh = h21(L.id + 3.7);
  L.H = 26.0 + 74.0 * hh * hh * hh + 12.0 * h21(L.id + 9.2);
  // the street wall facing the park is lower and even; so are the tower's neighbours
  vec2 cc = vec2(cs * LS + LD * 0.5, cu * LU + LU * 0.5);
  vec2 cw = vec2(0.0, APEXY) + (cc.x + PR + AVW) * WN + cc.y * WT;
  if (side < 0.5) cw.x = -cw.x;
  if (cw.y < FRONT + 34.0) L.H = 23.0 + 13.0 * h21(L.id + 1.1);
  if (length(cw - TOWER.xy) < 95.0) L.H = min(L.H, 30.0 + 10.0 * hh);
  L.H2 = L.H > 60.0 ? floor(L.H * 0.6) : L.H;
  // some roofs rise to a gable over the middle bays
  L.gab = h21(L.id + 6.6) < 0.45 && L.H < 60.0 ? 4.5 * max(0.0, 1.0 - abs(L.lp.y - LU * 0.5) / 8.0) : 0.0;
  return L;
}
float sdLots(vec3 p) {
  Lot L = lotAt(p.xy);
  float clip = FRONT - p.y;
  if (L.s < 0.0) { float d0 = max(-L.s, clip); g_soft = min(g_soft, d0); return d0 + 0.3; }
  float guard = min(LS - L.lp.x, L.gu) + 0.3;
  float bx = L.lp.x - LD;
  // the top storey stands back from the street behind a cornice
  float back = max(2.4 - L.lp.x, FRONT + 2.4 - p.y);
  float d = min(max(bx, p.z - (L.H2 - 3.7)), max(max(bx, back), (p.z - L.H2 - L.gab) * 0.9));
  if (L.H2 < L.H) {
    float inset = max(max(6.0 - L.lp.x, L.lp.x - (LD - 6.0)), 5.0 - L.gu);
    d = min(d, max(max(bx, inset), p.z - L.H));
  }
  d = max(d, clip);
  g_soft = min(g_soft, d);
  return max(min(d, guard), clip);
}
float sdWedge(vec3 p) {
  float w = sdWedge2(p.xy);
  float body = max(w, p.z - WH);
  float corn = max(w - 1.7, abs(p.z - (WH - 2.0)) - 0.55);
  float brk = max(w - 0.9, abs(p.z - (WH - 3.1)) - 0.55);
  float belt = max(w - 0.32, abs(p.z - 6.9) - 0.3);
  float belt2 = max(w - 0.3, abs(p.z - (WH - 12.6)) - 0.26);
  return min(min(body, corn), min(brk, min(belt, belt2)));
}
float sdTower(vec3 p) {
  vec2 q = abs(p.xy - TOWER.xy);
  float m = max(q.x, q.y);
  if (m > 40.0) return m - 30.0;
  float shaft = max(m - 7.0, p.z - TOWER.z);
  float loggia = max(m - 5.7, p.z - TOWER.z - 9.0);
  float pyr = max(m - 6.2 + (p.z - TOWER.z - 9.0) * 0.36, TOWER.z + 9.0 - p.z) * 0.9;
  float spire = max(m - 0.45, p.z - TOWER.z - 34.0);
  return min(min(shaft, loggia), min(pyr, spire));
}
float sdTrees(vec3 p) {
  if (p.z > 17.5) return p.z - 17.0;
  float d = 1e9, guard = 1e9;
  // a row of street trees on the far pavement, in front of the arcades
  {
    float gx = abs(p.x) - 28.0;
    if (gx < 0.0) guard = min(guard, -gx + 0.25);
    else {
      float c = min(floor(gx / 11.0), 9.0);
      float lx = gx - c * 11.0 - 5.5;
      guard = min(guard, 5.5 - abs(lx) + 0.25);
      vec2 id = vec2(c, p.x < 0.0 ? 71.0 : 53.0);
      vec2 d2 = vec2(lx - (h21(id) - 0.5) * 1.6, p.y - 56.3);
      float zc = 7.6 + 1.4 * h21(id + 2.0), R = 3.0 + 0.8 * h21(id + 5.0);
      float can = length(vec3(d2, (p.z - zc) * 1.2)) - R;
      if (can < 1.2) can += (noise3(p * 0.7) - 0.5) * 1.6 + (noise3(p * 2.1) - 0.5) * 0.6;
      d = min(max(length(d2) - 0.22, p.z - zc), can * 0.62);
    }
  }
  // the park's trees, either side of the paving
  vec2 g = vec2(abs(p.x) - 31.0, p.y + 104.0);
  float out2 = max(max(-g.x, -g.y), max(g.y - 140.0, g.x - 70.0));
  if (out2 > 0.0) guard = min(guard, out2 + 0.25);
  else {
    vec2 c = floor(g / 14.0), lp = g - c * 14.0 - 7.0;
    vec2 id = c + vec2(p.x < 0.0 ? 40.0 : 0.0, 3.0);
    guard = min(guard, min(7.0 - abs(lp.x), 7.0 - abs(lp.y)) + 0.25);
    if (h21(id) < (c.x < 0.5 ? 0.5 : 0.86)) {
      vec2 d2 = lp - (vec2(h21(id + 1.3), h21(id + 7.1)) - 0.5) * 1.2;
      float R = 4.0 + 1.0 * h21(id + 2.9), zc = 9.2 + 2.6 * h21(id + 4.4);
      float can = length(vec3(d2, (p.z - zc) * 1.2)) - R;
      if (can < 1.4) can += (noise3(p * 0.6) - 0.5) * 2.0 + (noise3(p * 1.9) - 0.5) * 0.7;
      d = min(d, min(max(length(d2) - 0.3, p.z - zc), can * 0.62));
    }
  }
  g_soft = min(g_soft, d);
  return min(d, guard);
}
float sdLamps(vec3 p) {
  vec3 l = nearLamp(p.xy);
  vec2 d2 = p.xy - l.xy;
  float d = min(max(length(d2) - 0.085, p.z - l.z + 0.25), length(vec3(d2, p.z - l.z)) - 0.3);
  // street lights down both avenues and along the cross street
  if (p.y > KERB - 2.0) {
    vec2 q = vec2(abs(p.x), p.y - APEXY);
    float s = dot(q, WN) - PR, u = dot(q, WT);
    float ls = abs(s - AVW * 0.5) - (AVW * 0.5 - 1.6);
    float lu = u - clamp(floor(u / 34.0 + 0.5), 0.0, 16.0) * 34.0;
    d = min(d, length(vec3(ls, lu, p.z - 8.6)) - 0.36);
    float cx = p.x - (clamp(floor((p.x - 15.0) / 30.0 + 0.5), -4.0, 3.0) * 30.0 + 15.0);
    d = min(d, length(vec3(cx, p.y - (KERB + 0.8), p.z - 8.6)) - 0.36);
  }
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
  vec2 res = vec2(sdWedge(p), 1.0);
  g_soft = res.x;
  float d = sdLots(p); if (d < res.x) res = vec2(d, 2.0);
  d = sdTower(p); g_soft = min(g_soft, d); if (d < res.x) res = vec2(d, 3.0);
  d = sdTrees(p); if (d < res.x) res = vec2(d, 4.0);
  d = sdLamps(p); if (d < res.x) res = vec2(d, 5.0);
  d = sdFountain(p); if (d < res.x) res = vec2(d, 6.0);
  return res;
}
float map(vec3 p) { return mapM(p).x; }
vec3 calcNormal(vec3 p, float t) {
  vec2 e = vec2(1.0, -1.0) * (0.012 + 0.0005 * t);
  return normalize(e.xyy * map(p + e.xyy) + e.yyx * map(p + e.yyx) + e.yxy * map(p + e.yxy) + e.xxx * map(p + e.xxx));
}
float shadow(vec3 ro, vec3 rd) {
  float res = 1.0, t = 0.3;
  for (int i = 0; i < 52; i++) {
    float d = map(ro + rd * t);
    res = min(res, 7.0 * g_soft / t);
    if (res < 0.01 || t > 300.0) break;
    t += clamp(d, 0.15, 16.0);
  }
  return clamp(res, 0.0, 1.0);
}

vec3 sky(vec3 rd) {
  bool day = u_light == 1;
  float el = rd.z / max(length(rd.xy), 1e-4);
  float h = max(rd.z, 0.0);
  float az = atan(rd.x, rd.y);
  vec3 c;
  if (!day) {
    // the city's glow low down, deep blue above, a little cloud catching the light
    c = mix(vec3(0.1, 0.082, 0.09), vec3(0.018, 0.026, 0.054), smoothstep(0.0, 0.5, h));
    c += vec3(0.1, 0.052, 0.022) * exp(-h * 8.0) * (0.55 + 0.45 * cos(az));
    float cl = fbm(vec2(az * 2.4, el * 5.0) + 3.0);
    c += vec3(0.07, 0.058, 0.066) * smoothstep(0.38, 0.8, cl) * smoothstep(0.03, 0.26, h) * (1.0 - smoothstep(0.3, 0.85, h));
  } else {
    c = mix(vec3(0.957, 0.937, 0.9), vec3(0.8, 0.855, 0.915), smoothstep(0.04, 0.8, h) * 0.7);
    c = mix(c, vec3(0.99, 0.95, 0.88), exp(-h * 7.0) * 0.5);
  }
  // the far city: layers of silhouettes with a scatter of lit windows
  for (int i = 0; i < 3; i++) {
    float fi = float(i);
    float sc = 44.0 + 30.0 * fi;
    float x = az * sc + fi * 17.3;
    float id = floor(x), fx = fract(x);
    float r = h11(id * 1.31 + fi * 7.7);
    float hh = (0.05 + 0.2 * r * r * r + 0.035 * h11(floor(x * 0.25) + fi * 3.0)) * (1.0 - 0.27 * fi);
    hh -= step(0.7, r) * 0.02 * step(0.28, abs(fx - 0.5));
    if (el < hh && el > -0.03) {
      float depth = 0.45 + 0.2 * fi;
      vec3 b = day ? mix(vec3(0.72, 0.71, 0.74), c, depth + 0.2) : mix(vec3(0.011, 0.013, 0.022), c, depth);
      vec2 wg = vec2(fx * (5.0 + floor(4.0 * h11(id + 2.0))), el * sc * 3.4);
      vec2 wi = floor(wg), wf = fract(wg);
      float lit = step(0.8 + 0.07 * fi, h21(wi + id * 3.1 + fi)) * step(0.22, wf.x) * step(wf.x, 0.78) * step(0.3, wf.y) * step(wf.y, 0.8);
      if (!day) b += mix(vec3(1.0, 0.62, 0.3), vec3(0.8, 0.86, 1.0), step(0.95, h21(wi + 4.4))) * lit * (0.42 - 0.1 * fi) * (0.35 + 0.65 * h21(wi + 1.7)) * (1.0 - 0.7 * u_quiet * smoothstep(-0.02, -0.2, az));
      else b = mix(b, b * 0.93, lit);
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
    float mull = 1.0 - 0.6 * detail * max(smoothstep(0.08, 0.03, abs(fract(ax / 0.85 + 0.5) - 0.5) * 0.85), smoothstep(0.1, 0.04, abs(z - spring)));
    em = mix(vec3(1.0, 0.5, 0.2), vec3(1.0, 0.68, 0.38), h21(vec2(bid, 9.0) + seed)) * (0.66 - 0.3 * z / base) * var * on * open * mull;
    // the pier between two openings, and the course above them
    alb *= 1.0 + 0.16 * smoothstep(bay * 0.4, bay * 0.47, abs(ax)) * detail;
    alb *= 1.0 - 0.8 * open;
    glass = open;
  } else if (z > top - 1.3) {
    // the cornice line
    alb *= 1.0 + 0.3 * smoothstep(1.3, 0.9, top - z) - 0.35 * smoothstep(0.5, 0.2, abs(top - z - 1.5));
  } else if (z > base) {
    vec2 g = vec2(s / bw, (z - base) / fh);
    vec2 id = floor(g), f = fract(g);
    float ax = clamp(fw / bw, 0.004, 0.28), ay = clamp(fw / fh, 0.004, 0.28);
    float w = smoothstep(0.22 - ax, 0.22 + ax, f.x) * smoothstep(0.78 + ax, 0.78 - ax, f.x) * smoothstep(0.2 - ay, 0.2 + ay, f.y) * smoothstep(0.8 + ay, 0.8 - ay, f.y);
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
    float pier = smoothstep(0.14, 0.04, min(f.x, 1.0 - f.x)), sill = smoothstep(0.035, 0.0, abs(f.y - 0.15)), head = smoothstep(0.06, 0.0, abs(f.y - 0.87));
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
  g_sink = (m > 1.5 && m < 2.5) || (m > 3.5 && m < 4.5 && p.y > 50.0) ? 0.55 * quietAt(p.x) * (day ? 0.8 : 1.0) : 0.0;
  float fw = t * u_pix;
  vec3 toF = vec3(0.0, 0.0, 6.0) - p;
  float fd2 = dot(toF, toF);
  float fN = max(dot(n, toF * inversesqrt(fd2)), 0.0);
  float fAtt = 1.0 / (1.0 + fd2 / (40.0 * 40.0));
  vec2 tg = normalize(vec2(-n.y, n.x) + 1e-5);
  float s = dot(p.xy, tg);
  bool wall = abs(n.z) < 0.5;
  vec3 alb = vec3(0.4), em = vec3(0.0), extra = vec3(0.0);
  float glass = 0.0, wrap = 0.0, street = wall ? 1.0 : 0.25, wf = 0.12;
  if (m < 1.5) {
    alb = day ? vec3(0.9, 0.85, 0.76) : vec3(0.6, 0.53, 0.44);
    wf = 1.0;
    if (wall) {
      alb *= 0.86 + 0.14 * vnoise(vec2(s * 0.6, p.z * 0.14)) + 0.05 * step(0.5, fract(p.z / 0.6)) * step(p.z, 6.6) * (1.0 - smoothstep(0.1, 0.4, fw));
      // the piers ripple, as the Flatiron's bays do
      alb *= 0.93 + 0.07 * sin(s * (6.2832 / 9.3));
      em = facade(s, p.z, WH - 3.4, vec2(3.0, 9.0), 3.1, 3.35, 6.9, 0.36, 1.0, 0.3, fw, alb, glass);
      // its crown is washed with light from the cornice
      extra = vec3(1.0, 0.72, 0.42) * 0.13 * smoothstep(WH - 16.0, WH - 3.5, p.z);
    } else if (n.z < -0.5) {
      extra = vec3(1.0, 0.72, 0.42) * 0.3;
    } else alb = day ? vec3(0.76, 0.46, 0.33) : alb * 0.35;
  } else if (m < 2.5) {
    Lot L = lotAt(p.xy);
    float k1 = h21(L.id + 5.5), k2 = h21(L.id + 8.8);
    vec3 stone = k1 < 0.45 ? vec3(0.52, 0.47, 0.4) : k1 < 0.8 ? vec3(0.4, 0.25, 0.18) : vec3(0.3, 0.33, 0.38);
    if (day) stone = k1 < 0.45 ? vec3(0.9, 0.85, 0.76) : k1 < 0.8 ? vec3(0.86, 0.7, 0.58) : vec3(0.84, 0.82, 0.78);
    alb = stone * (0.85 + 0.3 * k2);
    if (!day && k1 >= 0.8) alb *= 0.6;
    if (wall) {
      alb *= 0.88 + 0.12 * vnoise(vec2(s * 0.4, p.z * 0.1));
      // below the cornice the street wall; above it the attic, or a tower's shaft
      float corn = L.H2 - 3.7;
      float top = p.z > L.H2 + 0.3 ? L.H : p.z > corn ? L.H2 + 0.8 : corn;
      float fh = mix(3.3, 3.9, h21(L.id + 2.0));
      em = facade(s, p.z, top, L.id, mix(2.9, 4.4, k2), fh, p.z > corn && p.z < L.H2 + 0.3 ? corn + 0.7 : 5.4, mix(0.1, 0.17, k2), step(0.6, h21(L.id + 6.1)), 1.0, fw, alb, glass);
      // the cornice: a lit lip and the shadow under it
      alb *= 1.0 + 0.35 * smoothstep(0.5, 0.2, abs(p.z - corn + 0.15)) - 0.4 * smoothstep(0.9, 0.3, abs(p.z - corn + 1.2));
      // the block behind the hero's words stays out of it
      float qz = quietAt(p.x);
      em *= 1.0 - 0.96 * qz;
      street *= 1.0 - 0.9 * qz;
      // some crowns are lit
      extra = vec3(1.0, 0.7, 0.4) * 0.22 * step(0.72, h21(L.id + 4.2)) * smoothstep(L.H - 9.0, L.H - 1.0, p.z);
    } else alb = day ? (L.gab > 0.0 || h21(L.id + 7.3) < 0.5 ? vec3(0.74, 0.44, 0.31) : vec3(0.78, 0.75, 0.7)) * (0.9 + 0.2 * vnoise(p.xy * 0.8)) : alb * 0.3;
  } else if (m < 3.5) {
    alb = day ? vec3(0.9, 0.86, 0.78) : vec3(0.55, 0.5, 0.43);
    float zt = p.z - TOWER.z;
    if (wall && zt < 0.0) {
      em = facade(s, p.z, TOWER.z - 30.0, vec2(71.0, 3.0), 2.6, 3.6, 5.0, 0.1, 0.0, 0.8, fw, alb, glass);
      // the clock: a lit face on each side
      float cd = length(vec2(dot(p.xy - TOWER.xy, tg), p.z - (TOWER.z - 15.0)));
      float face = 1.0 - smoothstep(4.6 - fw, 4.6 + fw, cd);
      float ring = smoothstep(0.5, 0.2, abs(cd - 4.1));
      float ang = atan(dot(p.xy - TOWER.xy, tg), p.z - (TOWER.z - 15.0));
      float hands = max(smoothstep(0.34, 0.1, abs(sin(0.5 * (ang - 0.9))) * cd) * step(cd, 3.6), smoothstep(0.3, 0.1, abs(sin(0.5 * (ang + 2.2))) * cd) * step(cd, 2.5));
      em = mix(em, vec3(1.0, 0.86, 0.62) * (1.5 - 0.9 * max(ring * 0.6, hands)), face);
      extra = vec3(1.0, 0.72, 0.42) * 0.16 * smoothstep(TOWER.z - 34.0, TOWER.z, p.z);
    } else if (zt >= 0.0 && zt < 9.0) {
      // the loggia under the roof glows
      em = vec3(1.0, 0.7, 0.38) * 1.25 * step(abs(fract(s / 2.8) - 0.5), 0.3) * step(1.2, zt) * step(zt, 7.6);
      extra = vec3(1.0, 0.72, 0.42) * 0.4;
    } else {
      alb = day ? vec3(0.74, 0.43, 0.3) : vec3(0.3, 0.33, 0.3);
      extra = vec3(1.0, 0.72, 0.42) * 0.3 * exp(-(zt - 9.0) / 9.0);
      if (zt > 31.0) em = vec3(1.0, 0.75, 0.45) * 2.5;
    }
  } else if (m < 4.5) {
    float leaf = noise3(p * 1.4);
    bool trunk = p.z < 3.9;
    alb = trunk ? (day ? vec3(0.42, 0.37, 0.32) : vec3(0.07, 0.06, 0.05)) : (day ? vec3(0.6, 0.62, 0.48) * (0.82 + 0.3 * leaf) : vec3(0.055, 0.075, 0.035) * (0.45 + 1.1 * leaf));
    wrap = 0.5;
    street = 0.0;
    vec3 lp = nearLamp(p.xy) - p;
    float la = 1.0 / (1.0 + dot(lp, lp) / 30.0);
    extra = vec3(1.0, 0.64, 0.32) * la * (0.25 + 0.75 * max(dot(n, normalize(lp)), 0.0)) * 1.5;
    wf = 0.5;
    if (p.y > 50.0) extra += vec3(1.0, 0.58, 0.26) * 0.9 * exp(-max(p.z - 3.0, 0.0) / 3.0) * (0.4 + 0.6 * max(-n.z, 0.0)) * (1.0 - quietAt(p.x));
  } else if (m < 5.5) {
    vec3 l = nearLamp(p.xy);
    bool globe = (p.z > l.z - 0.31 && length(p.xy - l.xy) < 0.5) || p.z > 7.0;
    alb = vec3(0.03, 0.032, 0.036);
    if (day) alb = globe ? vec3(0.97, 0.95, 0.9) : vec3(0.2, 0.2, 0.2);
    else if (globe) em = (p.z > 7.0 ? vec3(1.0, 0.74, 0.44) * (1.0 - 0.92 * quietAt(p.x)) : vec3(1.0, 0.8, 0.54)) * 3.2;
    street = 0.0;
  } else {
    float r = length(p.xy);
    alb = (day ? vec3(0.9, 0.86, 0.78) : vec3(0.4, 0.38, 0.35)) * (0.85 + 0.3 * vnoise(vec2(atan(p.y, p.x) * 14.0, p.z * 3.0 + r)));
    street = 0.0;
    // lit from inside by the pool and the falling light
    float inside = r < 7.6 ? 1.0 : smoothstep(0.75, 0.97, p.z) * 0.5;
    extra = vec3(0.0);
    if (!day) { vec3 w0 = alb * vec3(1.0, 0.48, 0.18) * inside * (0.12 + 0.5 * exp(-max(p.z - 0.7, 0.0) / 1.4) + (n.z < -0.2 ? 0.35 : 0.0)); col = alb * vec3(0.02, 0.028, 0.045) * (0.7 + 0.3 * n.z); warm = w0 + alb * vec3(1.0, 0.5, 0.2) * 0.25 * (1.0 - inside); return; }
  }
  if (!day) {
    vec3 amb = vec3(0.03, 0.041, 0.072) * (0.7 + 0.3 * n.z);
    vec3 glow = vec3(1.0, 0.55, 0.24) * (m < 1.5 ? 0.2 * exp(-max(p.z, 0.0) / 17.0) : 0.05 * exp(-max(p.z, 0.0) / 6.0)) * street;
    col = alb * (amb + glow + extra) + em;
    // dark glass keeps a little of the sky
    col += vec3(0.012, 0.018, 0.032) * glass * step(dot(em, vec3(1.0)), 0.02);
    warm = alb * vec3(1.0, 0.5, 0.2) * mix(fN, 0.5 + 0.5 * fN, wrap) * fAtt * 2.3 * wf;
  } else {
    float ndl = max(dot(n, SUN), 0.0);
    ndl = mix(ndl, 0.45 + 0.55 * ndl, wrap);
    float sh = ndl > 0.001 ? shadow(p + n * 0.08, SUN) : 0.0;
    map(p + n * 1.2);
    float ao = clamp(0.5 + 0.5 * g_soft / 1.2, 0.0, 1.0);
    vec3 lightC = vec3(0.75, 0.745, 0.8) * (0.82 + 0.18 * n.z) * ao + vec3(0.42, 0.33, 0.19) * min(ndl * 1.6, 1.0) * sh + vec3(0.07, 0.05, 0.02) * max(-n.z + 0.3, 0.0);
    col = alb * lightC;
    // glass by day: the warm grey of a room behind it, and here and there the sky
    float gk = h21(floor(vec2(s / 3.1, p.z / 3.35)) + 3.0);
    vec3 gl = (gk < 0.22 ? vec3(0.82, 0.56, 0.42) : gk < 0.72 ? vec3(0.6, 0.57, 0.55) : vec3(0.78, 0.8, 0.83)) * (0.8 + 0.3 * sh * ndl);
    col = mix(col, gl, glass * 0.62);
    warm = vec3(0.0);
  }
}
vec3 fogged(vec3 c, float t, vec3 rd) {
  bool day = u_light == 1;
  float f = 1.0 - exp(-t / (day ? 800.0 : 760.0));
  f = mix(f, 1.0, g_sink);
  vec3 haze = day ? vec3(0.955, 0.936, 0.9) : vec3(0.03, 0.03, 0.04) + vec3(0.06, 0.032, 0.014) * exp(-max(rd.z, 0.0) * 6.0);
  return mix(c, haze, f);
}
void main() {
  bool day = u_light == 1;
  vec3 ro = u_cam, rd = rayDir(v_uv);
  float tG = rd.z < -1e-4 ? -ro.z / rd.z : 1e9;
  float tMax = min(tG, 900.0);
  float t = 0.05, m = 0.0;
  bool hit = false;
  for (int i = 0; i < 140; i++) {
    if (i >= u_steps) break;
    vec3 p = ro + rd * t;
    vec2 h = mapM(p);
    if (h.x < 0.004 + min(0.0012 * t, 0.2)) { hit = true; m = h.y; break; }
    t += h.x * 0.93;
    if (t > tMax || p.z > 190.0) break;
  }
  if (hit) {
    vec3 p = ro + rd * t;
    vec3 n = calcNormal(p, t);
    vec3 col, warm;
    shade(p, n, m, t, rd, col, warm);
    float f = mix(1.0 - exp(-t / (day ? 800.0 : 760.0)), 1.0, g_sink);
    col = fogged(col, t, rd);
    o0 = vec4(col, t); o1 = vec4(warm * (1.0 - f), 1.0); o2 = vec4(col, 0.0);
  } else if (tG < 1e8) {
    g_sink = 0.0;
    vec3 w = ro + rd * tG;
    map(w + vec3(0.0, 0.0, 0.7));
    float a1 = g_soft;
    map(w + vec3(0.0, 0.0, 2.4));
    float ao = clamp(0.28 + 0.72 * a1 / 0.9, 0.0, 1.0) * clamp(0.62 + 0.38 * g_soft / 2.8, 0.0, 1.0);
    if (!day) {
      vec3 r = vec3(rd.xy, -rd.z);
      vec3 q0 = w + vec3(0.0, 0.0, 0.04);
      float tr = 0.1, mr = 0.0;
      bool hr = false;
      for (int i = 0; i < 80; i++) {
        if (i * 2 >= u_steps + 20) break;
        vec3 p = q0 + r * tr;
        vec2 h = mapM(p);
        if (h.x < 0.006 + min(0.0014 * (tr + tG), 0.2)) { hr = true; mr = h.y; break; }
        tr += h.x * 0.95;
        if (tr > 640.0 || p.z > 190.0) break;
      }
      vec3 rc, rw = vec3(0.0);
      if (hr) {
        vec3 p = q0 + r * tr;
        vec3 n = calcNormal(p, tr + tG);
        shade(p, n, mr, tr + tG, r, rc, rw);
        float f = mix(1.0 - exp(-tr / 760.0), 1.0, g_sink);
        rc = fogged(rc, tr, r); rw *= 1.0 - f;
      } else rc = sky(r);
      o0 = vec4(0.0); o1 = vec4(rw, ao); o2 = vec4(rc, 0.0);
    } else {
      float sh = shadow(w + vec3(0.0, 0.0, 0.06), SUN);
      o0 = vec4(0.0); o1 = vec4(0.0, 0.0, 0.0, ao); o2 = vec4(sh, 0.0, 0.0, 0.0);
    }
  } else {
    vec3 c = sky(rd);
    o0 = vec4(c, -1.0); o1 = vec4(0.0); o2 = vec4(c, 0.0);
  }
}`;

// Each frame: the paving (wet, and mirroring what stands on it), the pool,
// the people, the fountain's liquid, and then the liquid sky over all of it.
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
  float jr = mix(0.5, 0.3, k) * (1.0 + 0.16 * sin(p.z * 3.1 - u_t * 9.0));
  float d = max(length(p.xy - wob) - jr, max(4.0 - p.z, p.z - H));
  d = min(d, length(p - vec3(0.0, 0.0, H)) - 0.66);
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
  float th = mix(0.44, 0.17, x) * (0.8 + (0.1 + 0.2 * x * x) * bead);
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
  float aq = dot(rd.xy, rd.xy), bq = dot(ro.xy, rd.xy), cq = dot(ro.xy, ro.xy) - 7.4 * 7.4;
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
  if (!day) glowC = (vec3(1.0, 0.46, 0.16) * glow * 0.02 + vec3(1.0, 0.4, 0.13) * veil * 0.16) * glowF;
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
  float colm = exp(-perp * perp / (2.0 * s1 * s1)) * smoothstep(u_fh + 1.0 + 0.5 * sp, u_fh - 1.5, zc) * (0.42 / s1);
  float umb = exp(-perp * perp / (2.0 * s2 * s2)) * smoothstep(u_fh + 1.0 + 0.5 * sp, u_fh - 4.0, zc) * 0.3;
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
float lampRefl(vec3 ro, vec2 w, vec3 r, float rough) {
  float acc = 0.0;
  for (int j = 0; j < 2; j++) {
    float hz = j == 0 ? 4.55 : 3.9, row = j == 0 ? 27.0 : 55.0;
    vec2 wl = ro.xy + (w - ro.xy) * ((ro.z + hz) / ro.z);
    float c = floor(wl.y / 16.0 + 0.5);
    for (int i = -1; i <= 1; i++) {
      float cc = c + float(i);
      if (cc > 2.5) continue;
      if (cc > -5.5) acc += lobe(r, vec3(row, cc * 16.0, hz) - vec3(w, 0.0), rough);
      if (cc > -5.5 && u_quiet < 0.5) acc += lobe(r, vec3(-row, cc * 16.0, hz) - vec3(w, 0.0), rough);
    }
  }
  return acc;
}
// rills of light in the paving, running out from the basin on eight lines
vec2 rill(vec2 w, float r) {
  float sec = PI / 4.0;
  float a = atan(w.y, w.x);
  float k = floor(a / sec + 0.5);
  float af = a - k * sec;
  float lat = r * sin(af), along = r * cos(af);
  float len = 26.5 + 6.0 * u_breath;
  float fwl = fwidth(lat);
  float m = lineL(abs(lat), 0.1 + 0.07 * u_breath, fwl) * smoothstep(9.0, 9.8, along) * (1.0 - smoothstep(len - 7.0, len, along));
  float flow = fbm(vec2(along * 0.4 - u_t * 0.8, k * 7.0));
  float ts = mod(u_t - 8.3, 10.0);
  float pulse = exp(-pow((along - 9.0 - 8.5 * ts) / 4.5, 2.0)) * exp(-ts / 2.3);
  float heat = 0.3 + 0.5 * flow + 1.3 * pulse + u_breath * (0.7 + 0.45 * sin(u_t * 0.9 - along * 0.2));
  float halo = exp(-lat * lat / 0.5) * smoothstep(9.0, 10.5, along) * (1.0 - smoothstep(len - 7.0, len, along));
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
      bool plaza = abs(w.x) < PLAZA && w.y < KERB;
      bool lawn = !plaza && w.y < KERB;
      vec3 alb;
      float wetK = 1.0, slabFade = 0.0;
      vec2 tilt = vec2(0.0);
      if (plaza) {
        // long slabs laid toward the wedge, so the joints run at the viewer
        float colx = floor(w.x / 1.6);
        float yy = w.y / 3.2 + 0.5 * mod(colx, 2.0);
        float dx = (0.5 - abs(fract(w.x / 1.6) - 0.5)) * 1.6, dyj = (0.5 - abs(fract(yy) - 0.5)) * 3.2;
        float fadeX = 1.0 - smoothstep(0.25, 0.9, fx), fadeY = 1.0 - smoothstep(0.5, 1.8, fy);
        float joint = max((1.0 - smoothstep(0.015, 0.04 + fx, dx)) * fadeX, (1.0 - smoothstep(0.015, 0.04 + fy, dyj)) * fadeY);
        vec2 sid = vec2(colx, floor(yy));
        float hv = mix(0.5, h21(sid), fadeX);
        slabFade = fadeX;
        tilt = (vec2(h21(sid + 1.7), h21(sid + 5.3)) - 0.5) * fadeX;
        alb = (day ? vec3(0.93, 0.89, 0.81) * (0.955 + 0.09 * hv) : vec3(0.21, 0.2, 0.195) * (0.84 + 0.32 * hv)) * (1.0 - (day ? 0.16 : 0.55) * joint);
        // travertine's bedding, or the grain of wet granite
        alb *= 0.93 + 0.14 * vnoise(vec2(w.x * 0.9, w.y * 7.0) + sid * 3.0) * fadeX;
        // a darker collar of stone round the basin
        float collar = smoothstep(12.4, 12.0, r);
        alb = mix(alb, (day ? vec3(0.8, 0.74, 0.66) : vec3(0.1, 0.1, 0.106)) * (0.85 + 0.3 * hv), collar);
        // the kerb
        float kb = max(smoothstep(PLAZA - 0.7, PLAZA - 0.5, abs(w.x)), smoothstep(KERB - 0.7, KERB - 0.5, w.y));
        alb = mix(alb, day ? vec3(0.84, 0.8, 0.74) : vec3(0.3, 0.3, 0.3), kb);
      } else if (lawn) {
        alb = (day ? vec3(0.62, 0.66, 0.5) : vec3(0.014, 0.022, 0.012)) * (0.75 + 0.5 * vnoise(w * 0.7));
        wetK = 0.05;
      } else {
        alb = (day ? vec3(0.7, 0.68, 0.66) : vec3(0.05, 0.052, 0.058)) * (0.8 + 0.4 * vnoise(w * 0.5));
        // a crossing in front of the prow
        float bar = step(abs(fract(w.x / 1.5) - 0.5), 0.26) * step(abs(w.x), 11.0) * step(abs(w.y - 51.0), 5.4) * (1.0 - smoothstep(0.3, 0.9, fx));
        alb = mix(alb, day ? vec3(0.93, 0.91, 0.87) : vec3(0.3, 0.3, 0.31), bar * 0.7);
      }
      float ao = s1.a;
      float fog = 1.0 - exp(-tG / (day ? 800.0 : 760.0));
      if (!day) {
        float pud = smoothstep(0.46, 0.66, fbm(w * 0.085 + 11.0));
        float cosv = max(-rd.z, 0.0);
        float F = 0.07 + 0.93 * pow(1.0 - cosv, 4.0);
        float refl = clamp(F * mix(0.55, 1.0, pud) * wetK * 1.1, 0.0, 1.0);
        float rough = mix(1.0, 0.1, pud);
        vec3 rr = normalize(vec3(rd.xy, -rd.z) + vec3(tilt * 0.01 * rough, 0.0));
        // what stands on the stone, smeared down it
        vec3 R = vec3(0.0), RW = vec3(0.0);
        float ws = 0.0;
        vec2 off = tilt * 0.0018 * rough;
        float jit = fract(52.9829189 * fract(dot(px, vec2(0.06711056, 0.00583715))));
        for (int k = 0; k < 6; k++) {
          float fk = (float(k) + jit) / 3.0 - 1.0;
          vec2 uv2 = clamp(v_uv + off + vec2(0.0, fk * 0.034 * rough), 0.0, 1.0);
          float wk = exp(-fk * fk * 2.2);
          if (u_sharp >= 0.5) { R += texture(u_s2, uv2).rgb * wk; RW += texture(u_s1, uv2).rgb * wk; }
          else { R += texture(u_f2, uv2 * u_fScale).rgb * wk; RW += texture(u_f1, uv2 * u_fScale).rgb * wk; }
          ws += wk;
        }
        R = (R + RW * glowF) / ws;
        vec3 L = vec3(0.03, 0.044, 0.08);
        vec2 dl = w - nearLamp(w).xy;
        L += vec3(1.0, 0.62, 0.3) * pow(1.0 + dot(dl, dl) / 22.0, -1.5) * 0.5;
        L += vec3(1.0, 0.48, 0.18) * glowF * 0.5 / (1.0 + r * r / (17.0 * 17.0)) * smoothstep(7.4, 10.0, r);
        L += vec3(1.0, 0.6, 0.28) * (0.4 * exp(-max(sdWedge2(w), 0.0) / 5.0) + 0.25 * exp(-max(FRONT - w.y, 0.0) / 5.0) * step(KERB, w.y) * (1.0 - quietAt(w.x)));
        float wr = 0.0;
        for (int i = 0; i < ${NW}; i++) {
          vec4 q = u_walk[i];
          vec2 d = w - q.xy;
          L += vec3(1.0, 0.66, 0.36) * q.z * 0.5 * exp(-dot(d, d) / 2.6);
          wr += q.z * lobe(rr, vec3(q.xy, 0.8) - vec3(w, 0.0), rough);
        }
        vec3 c = alb * L * ao;
        // the fountain itself, mirrored: its true image, broken by the lie of each slab
        vec3 fc, fg;
        vec3 rj = normalize(rr + vec3(0.0, 0.0, (jit - 0.5) * 0.05 * rough));
        bool fh = traceLiquid(vec3(w, 0.02), rj, 80.0, glowF, fc, fg);
        vec3 mirror = R + (fh ? fc : vec3(0.0)) + fg * 1.6 + vec3(1.0, 0.62, 0.3) * (fountainRefl(vec3(w, 0.0), rr, rough) * glowF * 0.22 + lampRefl(ro, w, rr, rough) * 3.0 + wr * 1.5);
        c = c * (1.0 - 0.7 * refl) + mirror * refl;
        if (r < 40.0) {
          vec2 rl = rill(w, r);
          c += ember(0.3 + 0.5 * rl.x) * rl.x * 2.2 + vec3(1.0, 0.45, 0.16) * rl.y * 0.16 * alb * 5.0;
        }
        col = mix(c, vec3(0.03, 0.03, 0.04) + vec3(0.06, 0.032, 0.014) * exp(-max(rd.z, 0.0) * 6.0), fog);
      } else {
        float sh = s2.r;
        // people's long shadows
        float ps = 0.0;
        for (int i = 0; i < ${NW}; i++) {
          vec2 a = u_walk[i].xy, b = a - SUN.xy / SUN.z * 1.75;
          vec2 pa = w - a, ba = b - a;
          float hq = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
          ps = max(ps, (1.0 - smoothstep(0.1, 0.3 + 0.3 * hq, length(pa - ba * hq))) * (1.0 - 0.5 * hq));
        }
        sh *= 1.0 - 0.95 * ps;
        vec3 c = alb * (vec3(0.72, 0.715, 0.8) * (0.8 + 0.2 * ao) + vec3(0.35, 0.3, 0.17) * sh);
        if (r < 40.0) {
          vec2 rl = rill(w, r);
          c = mix(c, vec3(0.74, 0.34, 0.17), clamp(rl.x * 1.2, 0.0, 0.85));
        }
        col = mix(c, vec3(0.955, 0.936, 0.9), fog);
      }
    }
    // the pool inside the basin
    if (rd.z < -1e-4) {
      float tP = (0.72 - ro.z) / rd.z;
      vec2 wp = ro.xy + rd.xy * tP;
      if (tP > 0.0 && tP < tHit && dot(wp, wp) < 7.5 * 7.5) { col = poolCol(wp, ro, rd, day); tHit = tP; }
    }
    // people: a figure and the light it carries
    for (int i = 0; i < ${NW}; i++) {
      vec4 q = u_walkScr[i];
      if (q.w <= 0.0 || q.w > tHit) continue;
      vec2 d = px - q.xy;
      float ppm = q.z;
      if (abs(d.x) > 3.0 * ppm || abs(d.y) > 3.0 * ppm) continue;
      // A slight figure, soft at the edges: at this distance a person is a
      // stroke with a head, not a drawing. The light hangs at its side.
      float sgn = fract(float(i) * 0.5) < 0.25 ? 1.0 : -1.0;
      vec2 fd = (d - vec2(-0.2 * sgn * ppm, 0.0)) / ppm;
      float yb = fd.y + 0.8;
      float lean = 0.035 * sin(u_t * 3.1 + float(i) * 1.7);
      float wdt = yb < 0.8 ? mix(0.06, 0.115, yb / 0.8) : mix(0.13, 0.085, clamp((yb - 0.8) / 0.55, 0.0, 1.0));
      float bd = max(abs(fd.x - lean * yb) - wdt, max(-yb, yb - 1.42));
      bd = min(bd, length(vec2(fd.x - lean * 1.5, yb - 1.53)) - 0.085);
      float soft = clamp(1.6 / ppm, 0.035, 0.3);
      float body = 1.0 - smoothstep(-soft, soft, bd);
      float bri = u_walk[i].z;
      if (day) { col = mix(col, vec3(0.36, 0.32, 0.31), body * 0.62); continue; }
      col = mix(col, vec3(0.012, 0.012, 0.018) + vec3(1.0, 0.56, 0.26) * 0.22 * bri * exp(-length(d) / (0.45 * ppm)), body * 0.8);
      float rc = max(0.055 * ppm, 0.8 * u_dpr);
      float dd = dot(d, d);
      col += vec3(1.0, 0.7, 0.4) * bri * (3.2 * exp(-dd / (2.0 * rc * rc)) * min(1.0, 0.055 * ppm / rc) + 0.2 * exp(-dd / (2.0 * 0.32 * 0.32 * ppm * ppm)));
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
