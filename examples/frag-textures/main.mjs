// Demo: a .frag with or without its texture sidecar, or a .glb, from one camera.
// ?frag=/m.frag[&textures=0] | ?glb=/m.glb, &az=<deg>&el=<deg>&zoom=<factor>
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
  const fragments = new FRAGS.FragmentsModels(workerUrl);
  if (q.get("textures") !== "0") {
    const url = new URL(`${fragUrl}.textures.json`, location.href);
    state.counts = applyFragTextures(fragments, await (await fetch(url)).json(), url);
  }
  const buffer = await (await fetch(fragUrl)).arrayBuffer();
  const model = await fragments.load(buffer, { modelId: "m", camera });
  scene.add(model.object);
  frame(model.box);
  for (let i = 0; i < 40; i++) {
    await fragments.update(true);
    renderer.render(scene, camera);
    await new Promise((r) => setTimeout(r, 50));
  }
}
await new Promise((r) => setTimeout(r, 1500)); // images
renderer.render(scene, camera);
state.ready = true;
