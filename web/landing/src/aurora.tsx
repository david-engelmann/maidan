import { useEffect, useRef } from "react";

const VERT = `
attribute vec2 a;
void main(){gl_Position=vec4(a,0.0,1.0);}
`;

const FRAG = `
precision highp float;
uniform vec2 u_res;
uniform float u_time;
uniform vec2 u_pointer;
float hash(vec2 p){
  p=fract(p*vec2(123.34,345.45));
  p+=dot(p,p+34.345);
  return fract(p.x*p.y);
}
float noise(vec2 p){
  vec2 i=floor(p);
  vec2 f=fract(p);
  f=f*f*(3.0-2.0*f);
  float a=hash(i);
  float b=hash(i+vec2(1.0,0.0));
  float c=hash(i+vec2(0.0,1.0));
  float d=hash(i+vec2(1.0,1.0));
  return mix(mix(a,b,f.x),mix(c,d,f.x),f.y);
}
float fbm(vec2 p){
  float v=0.0;
  float a=0.5;
  mat2 m=mat2(0.8,0.6,-0.6,0.8);
  for(int i=0;i<5;i++){
    v+=a*noise(p);
    p=m*p*2.02;
    a*=0.5;
  }
  return v;
}
void main(){
  vec2 uv=gl_FragCoord.xy/u_res.xy;
  float aspect=u_res.x/max(u_res.y,1.0);
  vec2 p=(uv-0.5)*vec2(aspect,1.0);
  float t=u_time*0.045;
  vec2 ptr=(u_pointer-0.5)*vec2(aspect,1.0);
  vec2 q=p*1.15;
  q+=(vec2(fbm(q+t),fbm(q-t+3.7))-0.5)*0.85;
  q+=ptr*0.18;
  float n1=fbm(q*1.15+vec2(t,0.0));
  float n2=fbm(q*1.35+vec2(4.2,-t*0.8));
  float n3=fbm(q*0.9+vec2(-t*0.6,2.4));
  vec3 col=vec3(0.012,0.014,0.02);
  col+=vec3(0.45,0.98,0.82)*smoothstep(0.35,0.78,n1)*1.15;
  col+=vec3(0.25,0.72,1.0)*smoothstep(0.4,0.82,n2)*0.95;
  col+=vec3(0.62,0.38,1.0)*smoothstep(0.42,0.86,n3)*1.0;
  float glow=exp(-dot(p-ptr*0.55,p-ptr*0.55)*1.4);
  col+=vec3(0.35,0.85,0.95)*glow*0.28;
  float vign=smoothstep(1.25,0.25,length(p));
  col*=mix(0.45,1.0,vign);
  col*=mix(1.0,0.62,smoothstep(0.55,1.0,uv.y));
  float wide=smoothstep(1.15,1.45,aspect);
  col*=mix(1.0,0.72,(1.0-smoothstep(0.05,0.42,uv.x))*wide);
  col+=(hash(gl_FragCoord.xy)-0.5)*0.02;
  gl_FragColor=vec4(col,1.0);
}
`;

export default function Aurora() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const gl = canvas.getContext("webgl", {
      alpha: false,
      antialias: false,
      depth: false,
      stencil: false,
      powerPreference: "low-power",
    });
    if (!gl) return;

    const compile = (type: number, source: string) => {
      const shader = gl.createShader(type);
      if (!shader) return null;
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        gl.deleteShader(shader);
        return null;
      }
      return shader;
    };
    const vs = compile(gl.VERTEX_SHADER, VERT);
    const fs = compile(gl.FRAGMENT_SHADER, FRAG);
    if (!vs || !fs) return;
    const program = gl.createProgram();
    if (!program) return;
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.bindAttribLocation(program, 0, "a");
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) return;
    gl.useProgram(program);

    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
    gl.enableVertexAttribArray(0);
    gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);

    const uRes = gl.getUniformLocation(program, "u_res");
    const uTime = gl.getUniformLocation(program, "u_time");
    const uPointer = gl.getUniformLocation(program, "u_pointer");
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const pointer = { x: 0.72, y: 0.38, tx: 0.72, ty: 0.38 };
    let running = false;
    let raf = 0;
    let past = false;
    let hidden = document.hidden;
    const started = performance.now();

    const resize = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 1.35) * 0.72;
      const w = Math.max(2, Math.floor(window.innerWidth * dpr));
      const h = Math.max(2, Math.floor(window.innerHeight * dpr));
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
        gl.viewport(0, 0, w, h);
      }
    };

    const draw = (now: number) => {
      resize();
      pointer.x += (pointer.tx - pointer.x) * 0.06;
      pointer.y += (pointer.ty - pointer.y) * 0.06;
      gl.uniform2f(uRes, canvas.width, canvas.height);
      gl.uniform1f(uTime, reduce ? 0 : (now - started) / 1000);
      gl.uniform2f(uPointer, pointer.x, 1 - pointer.y);
      gl.drawArrays(gl.TRIANGLES, 0, 3);
    };

    const stop = () => {
      running = false;
      cancelAnimationFrame(raf);
    };
    const start = () => {
      if (running) return;
      running = true;
      const loop = (now: number) => {
        if (!running) return;
        draw(now);
        raf = requestAnimationFrame(loop);
      };
      raf = requestAnimationFrame(loop);
    };
    const sync = () => {
      const live = !reduce && !hidden && !past;
      canvas.classList.toggle("is-paused", !live);
      if (live) start();
      else stop();
    };

    const onPointer = (event: PointerEvent) => {
      pointer.tx = event.clientX / Math.max(window.innerWidth, 1);
      pointer.ty = event.clientY / Math.max(window.innerHeight, 1);
    };
    const onScroll = () => {
      past = window.scrollY > window.innerHeight * 1.15;
      sync();
    };
    const onVis = () => {
      hidden = document.hidden;
      sync();
    };

    resize();
    draw(started);
    canvas.classList.add("is-on");
    sync();
    window.addEventListener("pointermove", onPointer, { passive: true });
    window.addEventListener("scroll", onScroll, { passive: true });
    document.addEventListener("visibilitychange", onVis);
    return () => {
      stop();
      window.removeEventListener("pointermove", onPointer);
      window.removeEventListener("scroll", onScroll);
      document.removeEventListener("visibilitychange", onVis);
      const lose = gl.getExtension("WEBGL_lose_context");
      if (lose) lose.loseContext();
    };
  }, []);

  return <canvas ref={ref} className="aurora" aria-hidden="true" />;
}
