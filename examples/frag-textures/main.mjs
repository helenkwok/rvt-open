// Demo: a .frag with or without its texture sidecar, or a .glb, from one camera.
// ?frag=/m.frag[&textures=0][&dispose=1] | ?glb=/m.glb, &az=<deg>&el=<deg>&zoom=<factor>
import * as THREE from "three";
import * as FRAGS from "@thatopen/fragments";
import workerUrl from "@thatopen/fragments/worker?url";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { applyFragTextures } from "./frag-textures.mjs";

const q = new URLSearchParams(location.search);
const renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
renderer.setSize(innerWidth, innerHeight);
renderer.setPixelRatio(1);
document.body.appendChild(renderer.domElement);
const scene = new THREE.Scene();
scene.background = new THREE.Color(0xffffff);
scene.add(new THREE.HemisphereLight(0xffffff, 0x888888, 2.2));
const sun = new THREE.DirectionalLight(0xffffff, 1.6);
sun.position.set(3, 5, 4);
scene.add(sun);
const camera = new THREE.PerspectiveCamera(35, innerWidth / innerHeight, 0.01, 10000);
const state = (window.__state = { ready: false });

function frame(box) {
  const c = box.getCenter(new THREE.Vector3());
  const r = box.getSize(new THREE.Vector3()).length() / 2 / Number(q.get("zoom") ?? 1);
  const az = THREE.MathUtils.degToRad(Number(q.get("az") ?? 35));
  const el = THREE.MathUtils.degToRad(Number(q.get("el") ?? 25));
  const d = r / Math.sin(THREE.MathUtils.degToRad(camera.fov / 2));
  camera.position.set(c.x + d * Math.cos(el) * Math.sin(az), c.y + d * Math.sin(el), c.z + d * Math.cos(el) * Math.cos(az));
  camera.lookAt(c);
  camera.updateMatrixWorld();
  state.box = [box.min.toArray(), box.max.toArray()];
}

if (q.get("glb")) {
  const gltf = await new GLTFLoader().loadAsync(q.get("glb"));
  scene.add(gltf.scene);
  frame(new THREE.Box3().setFromObject(gltf.scene));
  renderer.render(scene, camera);
} else {
  const fragUrl = q.get("frag");
  const buffer = await (await fetch(fragUrl)).arrayBuffer();
  const fragments = new FRAGS.FragmentsModels(workerUrl);
  let applied;
  if (q.get("textures") !== "0") {
    const url = new URL(`${fragUrl}.textures.json`, location.href);
    const res = await fetch(url);
    // a model without textures has no sidecar (a dev server may answer a
    // missing file with its index page, so check the type too).
    if (res.ok && (res.headers.get("content-type") ?? "").includes("json")) {
      const sidecar = await res.json();
      state.samplesWithUv = sidecar.samples.filter((s) => s.uv).length;
      applied = await applyFragTextures(fragments, "m", sidecar, url, buffer);
      state.counts = applied.counts;
      state.sidecar = "applied";
    } else {
      state.sidecar = "none";
    }
  }
  const model = await fragments.load(buffer, { modelId: "m", camera });
  scene.add(model.object);
  frame(model.box);
  for (let i = 0; i < 40; i++) {
    await fragments.update(true);
    renderer.render(scene, camera);
    await new Promise((r) => setTimeout(r, 50));
  }
  if (applied) {
    state.images = await applied.images(); // rejects when an image fails
    const textured = () => {
      let n = 0;
      model.object.traverse((o) => {
        const m = Array.isArray(o.material) ? o.material[0] : o.material;
        if (m?.userData?.fragTextured) n++;
      });
      return n;
    };
    state.texturedTiles = textured();
    // ?dispose=1: take the textures off again (checks that every tile gets its material back)
    if (q.get("dispose") === "1") {
      applied.dispose();
      state.texturedTilesAfterDispose = textured();
    }
  }
}
renderer.render(scene, camera);
state.ready = true;
