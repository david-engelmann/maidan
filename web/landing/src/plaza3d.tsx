import { Canvas, useFrame, useThree, type ThreeEvent } from "@react-three/fiber";
import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { FOUNTAIN, HOME, KIOSKS, MONUMENT, addTask, leaseProgress, step, world } from "./sim";

type View = { az: number; zoom: number; scroll: number };

function Rig({ view }: { view: { current: View } }) {
  const { camera, size } = useThree();
  const cur = useRef({ az: 0.5, r: 30, el: 0.95 });
  useEffect(() => {
    const cam = camera as THREE.PerspectiveCamera;
    const portrait = size.height > size.width;
    // Push the plaza out from under the copy: down on phones, right on laptops.
    if (portrait) cam.setViewOffset(size.width, size.height, 0, -size.height * 0.1, size.width, size.height);
    else cam.setViewOffset(size.width, size.height, -size.width * 0.13, -size.height * 0.02, size.width, size.height);
    cam.updateProjectionMatrix();
  }, [camera, size]);
  useFrame((_, dt) => {
    const v = view.current;
    const portrait = size.height > size.width;
    const base = portrait ? 56 : 44;
    const r = (base - v.scroll * base * 0.5) / v.zoom;
    const el = 0.82 - v.scroll * 0.36;
    const k = 1 - Math.exp(-dt * 4);
    cur.current.az += (v.az - cur.current.az) * k;
    cur.current.r += (r - cur.current.r) * k;
    cur.current.el += (el - cur.current.el) * k;
    const { az, r: rr, el: ee } = cur.current;
    camera.position.set(Math.sin(az) * Math.cos(ee) * rr, Math.sin(ee) * rr, Math.cos(az) * Math.cos(ee) * rr);
    camera.lookAt(0, 0, portrait ? -0.5 : 0);
  });
  return null;
}

const stoneGeo = new THREE.DodecahedronGeometry(0.28, 0);
function Monument() {
  const ref = useRef<THREE.InstancedMesh>(null);
  const m = useMemo(() => new THREE.Matrix4(), []);
  useFrame(() => {
    const inst = ref.current;
    if (!inst) return;
    for (let i = 0; i < 40; i++) {
      const ring = Math.floor(i / 10), a = i * 2.39996, rad = 1.0 + ring * 0.05 + (i % 3) * 0.12;
      const s = i < world.stones ? 1 : 0;
      m.compose(new THREE.Vector3(Math.sin(a) * rad, 0.2 + ring * 0.38, Math.cos(a) * rad), new THREE.Quaternion().setFromEuler(new THREE.Euler(i, i * 2, 0)), new THREE.Vector3(s, s, s));
      inst.setMatrixAt(i, m);
    }
    inst.instanceMatrix.needsUpdate = true;
  });
  return (
    <group position={[MONUMENT.x, 0, MONUMENT.z]}>
      <mesh castShadow position={[0, 0.25, 0]}><cylinderGeometry args={[1.5, 1.7, 0.5, 8]} /><meshStandardMaterial color="#d9d2c4" /></mesh>
      <mesh castShadow position={[0, 2.1, 0]}><boxGeometry args={[0.7, 3.4, 0.7]} /><meshStandardMaterial color="#ece6da" /></mesh>
      <mesh castShadow position={[0, 4.0, 0]}><coneGeometry args={[0.5, 0.6, 4]} /><meshStandardMaterial color="#f2b84b" metalness={0.4} roughness={0.4} /></mesh>
      <instancedMesh ref={ref} args={[stoneGeo, undefined, 40]} castShadow position={[0, 0.5, 0]}><meshStandardMaterial color="#8d8574" roughness={0.9} /></instancedMesh>
    </group>
  );
}

function Fountain() {
  const water = useRef<THREE.MeshStandardMaterial>(null);
  useFrame(({ clock }) => {
    const busy = world.agents.some((a) => a.phase === "review");
    if (water.current) water.current.emissiveIntensity = busy ? 0.55 + Math.sin(clock.elapsedTime * 6) * 0.25 : 0.12;
  });
  return (
    <group position={[FOUNTAIN.x, 0, FOUNTAIN.z]}>
      <mesh receiveShadow castShadow position={[0, 0.2, 0]}><cylinderGeometry args={[1.6, 1.75, 0.4, 32]} /><meshStandardMaterial color="#e3dccd" /></mesh>
      <mesh position={[0, 0.41, 0]} rotation={[-Math.PI / 2, 0, 0]}><circleGeometry args={[1.38, 32]} /><meshStandardMaterial ref={water} color="#5fb3dd" emissive="#5fb3dd" emissiveIntensity={0.12} /></mesh>
      <mesh castShadow position={[0, 0.9, 0]}><cylinderGeometry args={[0.18, 0.25, 1, 12]} /><meshStandardMaterial color="#e3dccd" /></mesh>
      <mesh position={[0, 1.45, 0]}><sphereGeometry args={[0.28, 16, 12]} /><meshStandardMaterial color="#7cc4e8" emissive="#7cc4e8" emissiveIntensity={0.3} /></mesh>
    </group>
  );
}

function Kiosks() {
  const refs = useRef<(THREE.Mesh | null)[]>([]);
  useFrame(() => {
    KIOSKS.forEach((k, i) => { for (let j = 0; j < 4; j++) { const m = refs.current[i * 4 + j]; if (m) m.visible = j < k.queue.length; } });
  });
  return (
    <>
      {KIOSKS.map((k, i) => (
        <group key={k.label} position={[k.x, 0, k.z]}>
          <mesh castShadow receiveShadow position={[0, 0.7, 0]}><boxGeometry args={[1.8, 1.4, 1.2]} /><meshStandardMaterial color="#f4efe5" /></mesh>
          <mesh castShadow position={[0, 1.75, 0.15]} rotation={[0.12, 0, 0]}><boxGeometry args={[2.3, 0.14, 1.8]} /><meshStandardMaterial color={k.color} /></mesh>
          {[-0.95, 0.95].map((x) => <mesh key={x} castShadow position={[x, 0.85, 0.75]}><cylinderGeometry args={[0.05, 0.05, 1.7, 6]} /><meshStandardMaterial color="#7a7468" /></mesh>)}
          {[0, 1, 2, 3].map((j) => (
            <mesh key={j} ref={(m) => { refs.current[i * 4 + j] = m; }} castShadow position={[-0.6 + j * 0.4, 1.58, 0.45]}><boxGeometry args={[0.3, 0.3, 0.3]} /><meshStandardMaterial color="#fffaf0" emissive={k.color} emissiveIntensity={0.25} /></mesh>
          ))}
        </group>
      ))}
    </>
  );
}

const ringGeo = new THREE.TorusGeometry(0.42, 0.06, 8, 40);
function Agents() {
  const groups = useRef<(THREE.Group | null)[]>([]);
  const rings = useRef<(THREE.Mesh | null)[]>([]);
  const boxes = useRef<(THREE.Mesh | null)[]>([]);
  useFrame(({ clock }) => {
    world.agents.forEach((a, i) => {
      const g = groups.current[i], r = rings.current[i], b = boxes.current[i];
      if (!g || !r || !b) return;
      const walking = a.phase === "toKiosk" || a.phase === "toFountain" || a.phase === "toMonument" || a.phase === "home";
      g.position.set(a.x, walking ? Math.abs(Math.sin(clock.elapsedTime * 10 + i)) * 0.12 : 0, a.z);
      g.rotation.y += (a.heading - g.rotation.y) * 0.2;
      const p = leaseProgress(a);
      r.visible = p > 0;
      r.scale.setScalar(0.3 + p * 0.7);
      r.rotation.z = clock.elapsedTime * 1.5;
      (r.material as THREE.MeshStandardMaterial).color.set(a.phase === "review" ? "#f2b84b" : "#3f7fd9");
      b.visible = a.task !== "" && a.phase !== "toKiosk" && a.phase !== "idle";
    });
  });
  return (
    <>
      {world.agents.map((a, i) => (
        <group key={a.id} ref={(g) => { groups.current[i] = g; }}>
          <mesh castShadow position={[0, 0.55, 0]}><capsuleGeometry args={[0.28, 0.5, 4, 12]} /><meshStandardMaterial color={a.color} /></mesh>
          <mesh castShadow position={[0, 1.2, 0]}><sphereGeometry args={[0.22, 16, 12]} /><meshStandardMaterial color={a.color} /></mesh>
          <mesh ref={(m) => { rings.current[i] = m; }} geometry={ringGeo} position={[0, 1.85, 0]} rotation={[Math.PI / 2, 0, 0]}><meshStandardMaterial color="#3f7fd9" emissive="#3f7fd9" emissiveIntensity={0.6} /></mesh>
          <mesh ref={(m) => { boxes.current[i] = m; }} castShadow position={[0, 0.75, 0.38]}><boxGeometry args={[0.32, 0.32, 0.32]} /><meshStandardMaterial color="#fffaf0" /></mesh>
        </group>
      ))}
    </>
  );
}

function Trees() {
  const spots = useMemo(() => Array.from({ length: 22 }, (_, i) => { const a = (i / 22) * Math.PI * 2 + 0.2; const r = 12.5 + (i % 3) * 0.9; return [Math.sin(a) * r, Math.cos(a) * r, 0.8 + (i % 4) * 0.25] as const; }), []);
  return <>{spots.map(([x, z, s], i) => (
    <group key={i} position={[x, 0, z]} scale={s}>
      <mesh castShadow position={[0, 0.4, 0]}><cylinderGeometry args={[0.1, 0.14, 0.8, 6]} /><meshStandardMaterial color="#7a5c3e" /></mesh>
      <mesh castShadow position={[0, 1.5, 0]}><icosahedronGeometry args={[0.85, 0]} /><meshStandardMaterial color={i % 2 ? "#6f9e58" : "#88b06a"} flatShading /></mesh>
    </group>
  ))}</>;
}

function Props() {
  const lamps = useMemo(() => Array.from({ length: 8 }, (_, i) => { const a = (i / 8) * Math.PI * 2 + 0.39; return [Math.sin(a) * 9.4, Math.cos(a) * 9.4] as const; }), []);
  const benches = useMemo(() => [[-2.6, -2.3, 0.8], [2.6, -2.3, -0.8], [-2.8, 3.1, 2.4], [2.8, 3.1, -2.4]] as const, []);
  return (
    <>
      {lamps.map(([x, z], i) => (
        <group key={i} position={[x, 0, z]}>
          <mesh castShadow position={[0, 1.2, 0]}><cylinderGeometry args={[0.05, 0.07, 2.4, 6]} /><meshStandardMaterial color="#3b3a36" /></mesh>
          <mesh position={[0, 2.5, 0]}><sphereGeometry args={[0.2, 12, 8]} /><meshStandardMaterial color="#fff3cf" emissive="#ffd98a" emissiveIntensity={0.8} /></mesh>
        </group>
      ))}
      {benches.map(([x, z, r], i) => (
        <mesh key={i} castShadow receiveShadow position={[x, 0.22, z]} rotation={[0, r, 0]}><boxGeometry args={[1.4, 0.16, 0.45]} /><meshStandardMaterial color="#9c6b45" /></mesh>
      ))}
    </>
  );
}

function Ground() {
  const down = useRef<{ x: number; y: number } | null>(null);
  const onDown = (e: ThreeEvent<PointerEvent>) => { down.current = { x: e.clientX, y: e.clientY }; };
  const onUp = (e: ThreeEvent<PointerEvent>) => {
    const d = down.current; down.current = null;
    if (!d || Math.hypot(e.clientX - d.x, e.clientY - d.y) > 8) return;
    addTask(e.point.x, e.point.z);
  };
  return (
    <group>
      <mesh receiveShadow rotation={[-Math.PI / 2, 0, 0]} onPointerDown={onDown} onPointerUp={onUp}><circleGeometry args={[16, 64]} /><meshStandardMaterial color="#ece2cf" /></mesh>
      {[4, 8, 11].map((r) => <mesh key={r} rotation={[-Math.PI / 2, 0, 0]} position={[0, 0.01, 0]}><ringGeometry args={[r, r + 0.08, 96]} /><meshStandardMaterial color="#d6ccb8" /></mesh>)}
      <mesh rotation={[-Math.PI / 2, 0, 0]} position={[HOME.x, 0.012, HOME.z + 0.4]}><circleGeometry args={[2.2, 32]} /><meshStandardMaterial color="#ddd3c0" /></mesh>
      <mesh rotation={[-Math.PI / 2, 0, 0]} position={[0, -0.02, 0]}><circleGeometry args={[60, 32]} /><meshStandardMaterial color="#9dbb7e" /></mesh>
    </group>
  );
}

function Tick() {
  useFrame((_, dt) => step(Math.min(dt, 0.05)));
  return null;
}

export default function Plaza3D({ view, active }: { view: { current: View }; active: boolean }) {
  return (
    <Canvas
      shadows
      dpr={[1, 2]}
      frameloop={active ? "always" : "never"}
      camera={{ fov: 26, near: 1, far: 200, position: [10, 25, 18] }}
      gl={{ antialias: true, powerPreference: "high-performance" }}
      aria-hidden="true"
    >
      <color attach="background" args={["#cfe0d6"]} />
      <fog attach="fog" args={["#e4e6d6", 70, 130]} />
      <hemisphereLight args={["#fff6e8", "#8fa37c", 1.1]} />
      <directionalLight castShadow position={[8, 16, 6]} intensity={2.1} shadow-mapSize={[1024, 1024]} shadow-camera-left={-16} shadow-camera-right={16} shadow-camera-top={16} shadow-camera-bottom={-16} />
      <Rig view={view} />
      <Tick />
      <Ground />
      <Trees />
      <Props />
      <Kiosks />
      <Fountain />
      <Monument />
      <Agents />
    </Canvas>
  );
}
