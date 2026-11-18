// The liquid, for the journey. Two things make it:
//  - a field of slow folds (noise warped through itself), which is the silk
//    of Act 2 at rest and, heated, the plume the camera rises through;
//  - a stable-fluids simulation that the cursor and the scroll stir. Its dye
//    is light poured into the field, and the flow it remembers bends the
//    folds, so the silk moves like something with weight.
// The field is written at half resolution (height, light, temperature,
// cover) and lit as a surface where the frame is composed (journey-city.ts).

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

// o0: height, light, temperature, cover.  o1: tone (indigo to umber), vein,
// hairline.  o2: the height's slope, taken here at full precision.
export const LIQ_FS = HEAD + `layout(location = 0) out vec4 o0; layout(location = 1) out vec4 o1; layout(location = 2) out vec4 o2;
uniform sampler2D u_dye; uniform float u_t; uniform float u_scroll; uniform float u_aspect; uniform float u_cov; uniform float u_hot;
uniform vec2 u_focus; uniform vec2 u_hotBand; uniform vec4 u_bands[4]; uniform vec2 u_dTexel;` + NOISE + `
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
  float tt = u_t * 0.028;
  vec2 q = vec2(fbm3(P + vec2(0.0, tt)), fbm3(P + vec2(5.2, 1.3) - vec2(tt, 0.0)));
  vec2 r = vec2(fbm3(P + 3.0 * q + vec2(1.7, 9.2) + tt * 0.7), fbm3(P + 3.0 * q + vec2(8.3, 2.8) - tt * 0.6));
  float f = fbm3(P + 3.2 * r);
  // drapery: long folds that lean across the frame, bent by the warp, with a
  // finer set riding on them. This is the silk.
  vec2 q0 = vec2(vnoise(P * 0.9 + vec2(3.1, tt)), vnoise(P * 0.9 + vec2(9.7, 4.2) - vec2(tt, 0.0)));
  vec2 q1 = vec2(vnoise(P * 2.1 + 2.0 * q0 + 1.3), vnoise(P * 2.1 + 2.0 * q0 + 6.8));
  float u = P.x * 0.62 - P.y * 0.78 + 1.5 * (q0.x - 0.5) + 0.42 * (q1.x - 0.5);
  float ph1 = u * 12.5 + 2.2 * (q0.y - 0.5), ph2 = u * 30.0 + 4.0 * (q1.y - 0.5);
  float fold = 0.5 + 0.5 * sin(ph1), fold2 = 0.5 + 0.5 * sin(ph2);
  float silkH = 0.085 * fold * fold * (3.0 - 2.0 * fold) + 0.012 * fold2 * smoothstep(0.35, 0.65, q1.y) + 0.05 * q0.y;
  float tone = smoothstep(0.5, 0.64, 0.62 * q0.y + 0.4 * q1.x);
  // thin veins of ember in the creases of some folds, broken along their length
  float dph = abs(mod(ph1 + 1.5 * 3.14159265, 6.2831853) - 3.14159265);
  float crease = exp(-dph * dph / (2.0 * 0.036 * 0.036));
  float along = fbm3(vec2(u * 0.7, (P.x * 0.78 + P.y * 0.62) * 2.2) + 17.0);
  float vein = crease * smoothstep(0.5, 0.6, along) * smoothstep(0.42, 0.62, q1.y);

  // finer hairlines in the creases of the smaller folds, as marble has
  float dp2 = abs(mod(ph2 + 1.5 * 3.14159265, 6.2831853) - 3.14159265);
  float hair = exp(-dp2 * dp2 / (2.0 * 0.07 * 0.07)) * smoothstep(0.4, 0.7, q1.y) * smoothstep(0.45, 0.6, fbm3(vec2(u * 1.3, (P.x * 0.78 + P.y * 0.62) * 3.0) + 41.0));

  // the plume: the same liquid, hot, where the camera passes through it
  float zone = exp(-pow((v_uv.y - u_hotBand.x) / u_hotBand.y, 2.0)) * u_hot;
  float fine = fbm(P * 2.1 + 4.0 * r + 3.0 * q);
  float fil = fbm(P * 4.6 + 7.0 * r + 2.0 * q + vec2(0.0, tt * 3.0));
  float ridge = pow(1.0 - abs(2.0 * fbm(P * 2.9 + 5.0 * r + vec2(tt * 2.0, 0.0)) - 1.0), 3.0);
  float body = smoothstep(0.43, 0.6, 0.55 * f + 0.45 * r.x + 0.08 * (fold - 0.5)) * (0.08 + 0.5 * fold + 0.8 * fine + 0.9 * ridge) * (0.4 + 1.1 * smoothstep(0.3, 0.7, fil));
  float tH = 0.92 * smoothstep(0.32, 0.8, 0.46 * r.y + 0.3 * fine + 0.2 * fold + 0.14 * fil + 0.12 * ridge);
  float E = zone * body * 1.2;
  float heat = E * tH;

  // how far the liquid has closed over the square: it swells out from the fountain
  float dF = length(sp - vec2(u_focus.x * u_aspect, u_focus.y));
  float cov = smoothstep(0.0, 0.26, u_cov * 2.35 - 0.3 - dF * (0.62 + 0.85 * f));
  float rim = cov * (1.0 - cov) * 4.0 * (0.4 + fine);
  E += rim * 1.0; heat += rim * 0.75;

  // a ribbon of molten light across a gap between two sections
  for (int i = 0; i < 4; i++) {
    vec4 b = u_bands[i];
    if (b.w < 0.5) continue;
    float x = b.w > 1.5 ? u_aspect - p.x : p.x;
    float ph = b.w * 2.7;
    float yc = b.x + b.y * (0.6 * sin(x * 2.0 + ph) + 0.2 * sin(x * 4.6 + 1.3 + u_t * 0.22));
    float sl = b.y * (1.2 * cos(x * 2.0 + ph) + 0.92 * cos(x * 4.6 + 1.3 + u_t * 0.22));
    float dist = (p.y - yc) / sqrt(1.0 + sl * sl);
    float head = (u_aspect + 0.7) * smoothstep(0.02, 0.52, b.z) - 0.15, tail = (u_aspect + 0.9) * smoothstep(0.5, 0.98, b.z) - 0.45;
    float span = smoothstep(tail, tail + 0.55, x) * (1.0 - smoothstep(head - 0.4, head, x));
    // it turns over as it goes, wide where it lies flat and thin on edge
    float tw = 0.5 + 0.5 * sin(x * 2.5 + u_t * 0.28 + ph * 2.0);
    float wd = (0.007 + 0.034 * tw * tw) * (0.35 + 0.65 * span);
    float e = dist / wd;
    float stri = 0.55 + 0.6 * fbm3(vec2(x * 1.6 - u_t * 0.35, e * 1.3 + 7.0));
    float rb = exp(-e * e) * span * stri;
    // a second, finer strand wound round it
    float d2 = dist - wd * 2.1 * sin(x * 3.4 + ph + u_t * 0.2);
    float rb2 = exp(-pow(d2 / 0.0045, 2.0)) * span * 0.7;
    E += rb * 1.5 + rb2 * 0.9; heat += rb * 1.5 * (0.4 + 0.5 * tw) + rb2 * 0.6;
  }

  // light poured in by the cursor and the plume's own flow
  E += max(dy.x, 0.0); heat += max(dy.y, 0.0);
  float temp = clamp(heat / (E + 0.06), 0.0, 1.0);
  float height = silkH * (1.0 - 0.6 * min(E, 1.0)) + 0.07 * E;
  o0 = vec4(height, E, temp, cov);
  o1 = vec4(tone, vein * (1.0 - min(zone, 1.0)), hair * (1.0 - min(zone, 1.0)), 1.0);
  o2 = vec4(dFdx(height), dFdy(height), 0.0, 1.0);
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
float hash(vec2 p) { p = fract(p * vec2(123.34, 456.21)); p += dot(p, p + 45.32); return fract(p.x * p.y); }
vec3 aces(vec3 x) { return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0); }
void main() {
  vec3 c = texture(uScene, v_uv).rgb;
  vec2 p = v_uv * uPx;
  float calm = 0.0;
  for (int i = 0; i < 8; i++) {
    vec4 r = uCalm[i];
    vec2 d = max(r.xy - p, p - r.zw);
    float outside = length(max(d, 0.0));
    calm = max(calm, 1.0 - smoothstep(0.0, uFeather[i], outside));
  }
  vec2 vq = v_uv - 0.5;
  float vig = 1.0 - dot(vq, vq) * 0.55;
  if (uLight == 0) {
    c += (texture(uB1, v_uv).rgb * 0.5 + texture(uB2, v_uv).rgb * 0.55 + texture(uB3, v_uv).rgb * 0.7) * uBloom;
    c = aces(c * 1.08 * vig);
    // a colourist's split: the shadows lean to blue, the lights stay warm
    float lm = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c *= mix(vec3(0.9, 0.98, 1.14), vec3(1.0), smoothstep(0.0, 0.3, lm));
    vec3 lin = pow(c, vec3(2.2));
    float L = dot(lin, vec3(0.2126, 0.7152, 0.0722));
    // behind copy the picture is stopped down, like a graduated filter, and then held under the cap
    float Ld = L * mix(1.0, 0.3, calm);
    float cap = 0.027;
    float Lc = cap * (1.0 - exp(-Ld / cap));
    lin *= mix(1.0, Lc / max(L, 1e-5), calm);
    c = pow(lin, vec3(1.0 / 2.2));
    c += (hash(p + uSeed) - 0.5) * 0.022;
  } else {
    c = clamp(c, 0.0, 1.0);
    // the copy sits on clean paper: the picture is washed out toward it
    vec3 paper = vec3(0.949, 0.933, 0.902);
    c = mix(c, paper, calm * 0.9);
    vec3 lin = pow(c, vec3(2.2));
    float L = dot(lin, vec3(0.2126, 0.7152, 0.0722));
    float D = 1.0 - L, Dmax = 0.32;
    float Dc = Dmax * (1.0 - exp(-D / Dmax));
    lin = 1.0 - (1.0 - lin) * mix(1.0, Dc / max(D, 1e-5), calm);
    c = pow(lin, vec3(1.0 / 2.2));
    c += (hash(p + uSeed) - 0.5) * 0.014;
  }
  o = vec4(c, 1.0);
}`;
