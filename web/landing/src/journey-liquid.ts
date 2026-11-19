// The liquid, for the journey. Two things make it:
//  - water seen from above: wave trains crossing, and the net of light they
//    throw (in shade at rest, sunlit where the page holds its light, in the
//    hero and under the closing call to action);
//  - a stable-fluids simulation that the cursor and the scroll stir. Its dye
//    is light poured into the water.
// The field is written at half resolution (height, light, temperature) and
// lit as a surface where the frame is composed (COMPOSE).

const HEAD = `#version 300 es
precision highp float;
in vec2 v_uv;
`;
const NOISE = `
float h21(vec2 p) { vec3 p3 = fract(vec3(p.xyx) * 0.1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
float vnoise(vec2 p) { vec2 i = floor(p), f = fract(p); f = f * f * f * (f * (f * 6.0 - 15.0) + 10.0); return mix(mix(h21(i), h21(i + vec2(1, 0)), f.x), mix(h21(i + vec2(0, 1)), h21(i + vec2(1, 1)), f.x), f.y); }
float fbm(vec2 p) { float a = 0.5, s = 0.0; for (int i = 0; i < 5; i++) { s += a * vnoise(p); p = mat2(1.6, 1.2, -1.2, 1.6) * p; a *= 0.5; } return s; }
float fbm3(vec2 p) { float a = 0.5, s = 0.0; for (int i = 0; i < 3; i++) { s += a * vnoise(p); p = mat2(1.6, 1.2, -1.2, 1.6) * p; a *= 0.5; } return s; }
`;

// o0: height, the net of light, how much the page's light stands here.
// o1: tone, the light between the net's lines, light poured in by the cursor.  o2: the height's slope, taken here at full precision.
export const LIQ_FS = HEAD + `layout(location = 0) out vec4 o0; layout(location = 1) out vec4 o1; layout(location = 2) out vec4 o2;
uniform sampler2D u_dye; uniform float u_t; uniform float u_scroll; uniform float u_aspect; uniform float u_hot;
uniform vec2 u_hotBand; uniform vec2 u_hotSpan; uniform vec2 u_dTexel; uniform float u_hotSquare;` + NOISE + `
void main() {
  vec4 dy = texture(u_dye, v_uv);
  vec2 sp = vec2(v_uv.x * u_aspect, v_uv.y);
  // where the flow has carried this point, taken broadly so the folds bend and never crumple
  vec2 dsp = dy.zw * 0.2;
  for (int i = 0; i < 4; i++) {
    vec2 o4 = (i < 2 ? vec2(1.0, 0.0) : vec2(0.0, 1.0)) * (mod(float(i), 2.0) < 0.5 ? 1.0 : -1.0);
    dsp += (texture(u_dye, v_uv + o4 * u_dTexel * 3.0).zw + texture(u_dye, v_uv + o4.yx * u_dTexel * 6.0 + o4 * u_dTexel * 6.0).zw) * 0.1;
  }
  vec2 p = sp - dsp * vec2(u_aspect, 1.0);
  vec2 P = vec2(p.x, p.y - u_scroll * 0.42) * 1.05 * clamp(1.15 / u_aspect, 1.0, 1.9);
  // Water, seen from above. Its surface is a handful of wave trains crossing
  // one another, bent a little so they never line up. The light that falls
  // through it gathers where the surface curves: the bright net on the floor
  // of a pool. That net is worked out from the surface itself (how much each
  // patch of light is squeezed, one over the determinant below), so it moves
  // as the water does.
  float tt = u_t * 0.5;
  vec2 bend = vec2(fbm3(P * 0.8 + vec2(0.0, tt * 0.05)), fbm3(P * 0.8 + vec2(5.2, 1.3) - vec2(tt * 0.04, 0.0))) - 0.5;
  vec2 W = P + 0.16 * bend;
  // The light that reaches a point on the floor left the surface a little
  // way off, pushed by the slope there. Find that place (two steps toward
  // it), then ask how much the surface squeezes the light at it.
  const float reach = 0.6;
  vec2 src = W;
  float hgt = 0.0;
  vec3 hs = vec3(0.0);
  for (int it = 0; it < 3; it++) {
    vec2 g = vec2(0.0);
    hgt = 0.0; hs = vec3(0.0);
    for (int i = 0; i < 7; i++) {
      float fi = float(i);
      float ang = 0.6 + fi * 1.9416;                    // a half turn over the golden ratio: the crests face every way, so the net has no grain
      vec2 d = vec2(cos(ang), sin(ang));
      float k = 30.0 * pow(1.3, fi);
      float w = 1.0 / (1.0 + 0.3 * fi);
      float ph = k * dot(d, src) + tt * (0.9 + 0.32 * fi) + fi * 1.7;
      float sn = sin(ph);
      hgt += w * sn / (k * k);
      g += w * d * cos(ph) / k;
      hs -= w * vec3(d.x * d.x, d.x * d.y, d.y * d.y) * sn;
    }
    if (it < 2) src = W - reach * g;
  }
  float det = (1.0 + reach * hs.x) * (1.0 + reach * hs.z) - reach * reach * hs.y * hs.y;
  // thin and bright along the folds, as light through water is, with a soft fall-off either side
  float caus = 0.085 / (abs(det) + 0.085);
  caus = caus * caus * 0.75 + 0.25 * pow(0.3 / (abs(det) + 0.3), 2.0);
  // and the light between the lines: bright near a fold, dim where the surface spreads it
  float soft = 0.5 / (abs(det) + 0.5);
  float tone = fbm3(P * 0.9 + vec2(3.1, tt * 0.03));
  // where the light stands on the page: a middle and a reach up the frame
  // (u_hotBand) and across it (u_hotSpan). Squared by hand: pow() of a
  // negative base is undefined.
  vec2 hz = (v_uv - vec2(u_hotSpan.x, u_hotBand.x)) / vec2(u_hotSpan.y, u_hotBand.y);
  vec2 hz2 = hz * hz;
  // round by night; by day squarer, so the pool has straight shores and a round corner
  float zone = exp(-mix(hz2.x + hz2.y, hz2.x * hz2.x + hz2.y * hz2.y, u_hotSquare)) * u_hot;
  // light poured in by the cursor, and the swell it leaves
  float pour = max(dy.x, 0.0);
  float height = hgt * 90.0 + 0.03 * pour;
  o0 = vec4(height, caus, zone, 1.0);
  o1 = vec4(tone, soft, pour, 1.0);
  o2 = vec4(dFdx(height), dFdy(height), 0.0, 1.0);
}`;

// The frame: the water lit from above. By night it is deep blue, lighter and
// clearer where the page holds its light, with a white net of light on it.
// By day it is a pale pool there, and the page's paper elsewhere.
export const COMPOSE = HEAD + `out vec4 o; uniform sampler2D u_l0; uniform sampler2D u_l1; uniform sampler2D u_ln; uniform float u_rel; uniform int u_light;
void main() {
  vec4 lq = texture(u_l0, v_uv);
  vec4 l1 = texture(u_l1, v_uv);
  vec3 n = normalize(vec3(texture(u_ln, v_uv).xy * u_rel, 1.0));
  vec3 Ld = normalize(vec3(-0.5, 0.6, 0.62));
  float diff = max(dot(n, Ld), 0.0);
  float caus = lq.y, heat = clamp(lq.z, 0.0, 1.0), tone = l1.x, soft = l1.y, pour = l1.z;
  // What is seen is the light on the floor of the water: bright along the
  // net, dimmer inside each cell. The surface's own relief is all but left
  // out; shaded like a solid, water reads as foil or cloth.
  float relief = 0.96 + 0.08 * diff;
  vec3 col;
  float sun = 0.0;
  if (u_light == 0) {
    // By night a deep blue in shade, with the net a dim cyan on it; where
    // the page holds its light the water is sunlit, a clear bright blue
    // under a white net. Never amber: warm light on dark water reads as smoke.
    float lit = smoothstep(0.08, 0.6, heat);
    vec3 body = mix(vec3(0.012, 0.062, 0.135), vec3(0.016, 0.08, 0.165), tone);
    body = mix(body, vec3(0.01, 0.17, 0.3), lit * 0.85);
    col = body * (0.5 + 1.25 * soft) * relief;
    // in shade the net comes and goes across the page, as light does through deep water: one even net is wallpaper
    float patchy = mix(0.22, 1.25, smoothstep(0.36, 0.62, tone));
    col += mix(vec3(0.2, 0.55, 0.85) * 0.42 * patchy, vec3(0.78, 0.95, 1.0) * 1.5, lit) * caus;
    col += vec3(0.8, 0.95, 1.0) * pour * 0.3;
    sun = lit;
  } else {
    // By day a pool seen from above: the page's paper for a shore, pale
    // shallows, and a deeper blue where the page holds its light. The shore
    // wanders, so the water never ends on a straight line.
    float h = clamp(heat + (tone - 0.5) * 0.9 * smoothstep(0.0, 0.3, heat), 0.0, 1.0);
    // a waterline, not a haze: the water ends within a few pixels, on a
    // pale wet edge, and deepens away from its shore
    float lit = smoothstep(0.244, 0.258, h);
    float rim = lit * (1.0 - smoothstep(0.258, 0.3, h));
    vec3 water = mix(vec3(0.72, 0.89, 0.92), vec3(0.39, 0.72, 0.85), smoothstep(0.26, 0.9, h));
    col = mix(vec3(0.949, 0.933, 0.902), water * (0.86 + 0.3 * soft) * relief, lit);
    col = mix(col, vec3(1.0), clamp(caus * 1.15 * lit + rim * 0.45, 0.0, 0.95));
    sun = lit;
    col = mix(col, vec3(1.0), clamp(pour * 0.25, 0.0, 0.9));
  }
  // how sunlit the water is here goes with the picture: the last pass stops sunlit water down behind copy
  o = vec4(col, sun);
}`;

// ---- the simulation (stable fluids)
// The slope of that surface, smoothed on the field's own grid so the light on
// it is smooth. Where float textures filter, the height is kept in full
// floats and differenced here (uMode 0). A half float is too coarse to
// difference (the light would band), so without them the slope the field
// wrote at full precision is used, spread over its neighbours (uMode 1).
export const LIQN_FS = HEAD + `out vec4 o; uniform sampler2D u_l0; uniform sampler2D u_l2; uniform vec2 u_tx; uniform int uMode;
void main() {
  if (uMode == 0) {
    float a = texture(u_l0, v_uv + u_tx * vec2(-1, 1)).x, b = texture(u_l0, v_uv + u_tx * vec2(0, 1)).x, c = texture(u_l0, v_uv + u_tx * vec2(1, 1)).x;
    float d = texture(u_l0, v_uv + u_tx * vec2(-1, 0)).x, f = texture(u_l0, v_uv + u_tx * vec2(1, 0)).x;
    float g = texture(u_l0, v_uv + u_tx * vec2(-1, -1)).x, h = texture(u_l0, v_uv + u_tx * vec2(0, -1)).x, i = texture(u_l0, v_uv + u_tx * vec2(1, -1)).x;
    o = vec4((a + 2.0 * d + g - c - 2.0 * f - i) * 0.25, (g + 2.0 * h + i - a - 2.0 * b - c) * 0.25, 0.0, 1.0);
  } else {
    vec2 sl = texture(u_l2, v_uv).xy * 0.25;
    sl += (texture(u_l2, v_uv + u_tx * vec2(1.5, 0.5)).xy + texture(u_l2, v_uv + u_tx * vec2(-0.5, 1.5)).xy + texture(u_l2, v_uv + u_tx * vec2(-1.5, -0.5)).xy + texture(u_l2, v_uv + u_tx * vec2(0.5, -1.5)).xy) * 0.1875;
    o = vec4(-2.0 * sl, 0.0, 1.0);
  }
}`;

export const SIM_VS = `#version 300 es
in vec2 a_pos;
out vec2 vUv, vL, vR, vT, vB;
uniform vec2 texel;
void main() { vUv = a_pos * 0.5 + 0.5; vL = vUv - vec2(texel.x, 0.0); vR = vUv + vec2(texel.x, 0.0); vT = vUv + vec2(0.0, texel.y); vB = vUv - vec2(0.0, texel.y); gl_Position = vec4(a_pos, 0.0, 1.0); }`;
const SH = `#version 300 es
precision highp float;
precision highp sampler2D;
in vec2 vUv, vL, vR, vT, vB;
out vec4 o;
`;
export const SIM = {
  splat: SH + `uniform sampler2D uTarget; uniform float aspect; uniform vec4 color; uniform vec2 point; uniform float radius;
void main() { vec2 p = vUv - point; p.x *= aspect; o = texture(uTarget, vUv) + color * exp(-dot(p, p) / radius); }`,
  advect: SH + `uniform sampler2D uVelocity; uniform sampler2D uSource; uniform float dt; uniform vec4 decay; uniform float gain;
void main() {
  vec2 v = texture(uVelocity, vUv).xy;
  o = texture(uSource, vUv - dt * v) / (1.0 + decay * dt);
  // the dye also remembers where the flow has carried it
  o.zw += dt * v * gain;
}`,
  forces: SH + NOISE + `uniform sampler2D uVelocity; uniform sampler2D uDye; uniform float dt; uniform float buoy; uniform float push; uniform float uT; uniform float aspect;
void main() {
  vec2 v = texture(uVelocity, vUv).xy;
  v.y += dt * buoy * texture(uDye, vUv).y;
  // scrolling pushes the whole field, unevenly, so it rolls rather than slides
  vec2 p = vec2(vUv.x * aspect, vUv.y);
  v += dt * push * vec2(0.6 * (vnoise(p * 2.3 + 4.0) - 0.5), 0.5 + 0.9 * vnoise(p * 2.0 + uT * 0.05));
  o = vec4(v, 0.0, 1.0);
}`,
  curl: SH + `uniform sampler2D uVelocity;
void main() { o = vec4(0.5 * (texture(uVelocity, vR).y - texture(uVelocity, vL).y - texture(uVelocity, vT).x + texture(uVelocity, vB).x), 0.0, 0.0, 1.0); }`,
  vort: SH + `uniform sampler2D uVelocity; uniform sampler2D uCurl; uniform float curl; uniform float dt;
void main() {
  float L = texture(uCurl, vL).x, R = texture(uCurl, vR).x, T = texture(uCurl, vT).x, B = texture(uCurl, vB).x, C = texture(uCurl, vUv).x;
  vec2 f = 0.5 * vec2(abs(T) - abs(B), abs(R) - abs(L));
  f /= length(f) + 0.0001; f *= curl * C; f.y *= -1.0;
  o = vec4(clamp(texture(uVelocity, vUv).xy + f * dt, -10.0, 10.0), 0.0, 1.0);
}`,
  div: SH + `uniform sampler2D uVelocity;
void main() {
  float L = texture(uVelocity, vL).x, R = texture(uVelocity, vR).x, T = texture(uVelocity, vT).y, B = texture(uVelocity, vB).y;
  vec2 C = texture(uVelocity, vUv).xy;
  if (vL.x < 0.0) L = -C.x; if (vR.x > 1.0) R = -C.x; if (vT.y > 1.0) T = -C.y; if (vB.y < 0.0) B = -C.y;
  o = vec4(0.5 * (R - L + T - B), 0.0, 0.0, 1.0);
}`,
  clear: SH + `uniform sampler2D uTexture; uniform float value; void main() { o = value * texture(uTexture, vUv); }`,
  press: SH + `uniform sampler2D uPressure; uniform sampler2D uDivergence;
void main() { o = vec4((texture(uPressure, vL).x + texture(uPressure, vR).x + texture(uPressure, vB).x + texture(uPressure, vT).x - texture(uDivergence, vUv).x) * 0.25, 0.0, 0.0, 1.0); }`,
  grad: SH + `uniform sampler2D uPressure; uniform sampler2D uVelocity;
void main() { o = vec4(texture(uVelocity, vUv).xy - vec2(texture(uPressure, vR).x - texture(uPressure, vL).x, texture(uPressure, vT).x - texture(uPressure, vB).x), 0.0, 1.0); }`,
};

export const BRIGHT = HEAD + `out vec4 o; uniform sampler2D uTex; uniform float thresh;
void main() { vec3 c = min(texture(uTex, v_uv).rgb, vec3(6.0)); float l = max(c.r, max(c.g, c.b)); o = vec4(c * smoothstep(thresh, thresh + 0.6, l), 1.0); }`;
export const BLUR = HEAD + `out vec4 o; uniform sampler2D uTex; uniform vec2 dir;
void main() {
  vec4 c = texture(uTex, v_uv) * 0.227027;
  c += (texture(uTex, v_uv + dir * 1.3846) + texture(uTex, v_uv - dir * 1.3846)) * 0.3162162;
  c += (texture(uTex, v_uv + dir * 3.2308) + texture(uTex, v_uv - dir * 3.2308)) * 0.0702703;
  o = c;
}`;
// Tone, and the promise to the text: inside any block of copy the picture is
// held under (or, in daylight, over) the luminance that keeps body text at 4.5:1.
export const FINAL = HEAD + `out vec4 o; uniform sampler2D uScene; uniform sampler2D uB1; uniform sampler2D uB2; uniform sampler2D uB3;
uniform int uLight; uniform vec2 uPx; uniform float uSeed; uniform vec4 uCalm[8]; uniform float uFeather[8]; uniform float uBloom;
// grain with no weave in it: hashed on whole pixels
float hash(vec2 p) { vec3 p3 = fract(vec3(floor(p).xyx) * 0.1031); p3 += dot(p3, p3.yzx + 33.33); return fract((p3.x + p3.y) * p3.z); }
vec3 aces(vec3 x) { return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0); }
void main() {
  vec4 sc = texture(uScene, v_uv);
  vec3 c = sc.rgb;
  float sun = sc.a;
  vec2 p = v_uv * uPx;
  // Three kinds of place are held calm. The first is the header's bar. A
  // negative feather marks large words (the closing headline), which need
  // less than body copy does. The rest are blocks of copy.
  float calm = 0.0, bar = 0.0, big = 0.0;
  for (int i = 0; i < 8; i++) {
    vec4 r = uCalm[i];
    vec2 d = max(r.xy - p, p - r.zw);
    float outside = length(max(d, 0.0));
    float k = 1.0 - smoothstep(0.0, abs(uFeather[i]), outside);
    if (i == 0) bar = k; else if (uFeather[i] < 0.0) big = max(big, k); else calm = max(calm, k);
  }
  vec2 vq = v_uv - 0.5;
  float vig = 1.0 - dot(vq, vq) * 0.55;
  const vec3 lumW = vec3(0.2126, 0.7152, 0.0722);
  if (uLight == 0) {
    c += (texture(uB1, v_uv).rgb * 0.5 + texture(uB2, v_uv).rgb * 0.55 + texture(uB3, v_uv).rgb * 0.7) * uBloom;
    c = aces(c * 1.08 * vig);
    // a colourist's split: the shadows lean to blue
    float lm = dot(c, lumW);
    c *= mix(vec3(0.9, 0.98, 1.14), vec3(1.0), smoothstep(0.0, 0.3, lm));
    vec3 lin = pow(c, vec3(2.2));
    float L = dot(lin, lumW);
    // Behind copy, sunlit water is stopped down until it is the water in
    // shade, and the whole is rolled off under the cap that keeps body text
    // at 4.5:1. Water in shade is all but untouched, so it is the same
    // water behind the words as beside them: a veil over it reads as murk.
    calm = max(calm, bar);
    float cap = 0.032;
    float Ld = L * mix(1.0, 0.09, sun * calm);
    float Lc = cap * (1.0 - exp(-Ld / cap));
    float held = mix(L, Lc, calm);
    // large words need only 3:1: the water round them is shaded, not put out
    float capB = 0.16;
    float Lb = capB * (1.0 - exp(-held * mix(1.0, 0.45, sun) / capB));
    held = mix(held, Lb, big * (1.0 - calm));
    lin *= held / max(L, 1e-5);
    // a held highlight is a grey, and grey on navy reads as brown: keep it the water's blue
    lin = mix(lin, vec3(0.15, 1.0, 3.3) * dot(lin, lumW), smoothstep(0.0, 0.3, max(calm, big)) * smoothstep(0.1, 0.6, L) * (1.0 - held / max(L, 1e-5)));
    c = pow(max(lin, 0.0), vec3(1.0 / 2.2));
    c += (hash(p + uSeed) - 0.5) * 0.013;
  } else {
    // a hazy photograph, not a model: the picture seen through a diffusion filter
    c = mix(clamp(c, 0.0, 1.0), texture(uB1, v_uv).rgb, 0.24);
    c = clamp(c, 0.0, 1.0);
    // the header's bar stands on lightened water
    c = mix(c, vec3(0.949, 0.933, 0.902), bar * 0.4);
    calm = max(calm, bar);
    // Behind copy the water is lifted until body text keeps 4.5:1 on it,
    // by scaling, which keeps its colour: lifted toward white it is a
    // frosted haze. The page's paper is already over the line and is left alone.
    vec3 lin = pow(c, vec3(2.2));
    float L = dot(lin, lumW);
    float Lc = max(L, 0.74 + (L - 0.4) * 0.26);
    float up = Lc / max(L, 1e-5);
    // mostly by scaling, a little toward white, or the lifted water is a pool-side blue too loud for words
    vec3 lifted = mix(min(lin * up, vec3(1.0)), 1.0 - (1.0 - lin) * (1.0 - Lc) / max(1.0 - L, 1e-5), 0.4);
    lin = mix(lin, lifted, calm);
    c = pow(max(lin, 0.0), vec3(1.0 / 2.2));
    c += (hash(p + uSeed) - 0.5) * 0.014;
  }
  o = vec4(c, 1.0);
}`;
