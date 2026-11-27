/*
 * The View display panel: a 3D view's graphic display options, as the file stores them
 * (rvt-open's `sun_settings`, the model's `sun`), applied to the 3D view the way Revit shows
 * that view: visual style, lighting, shadows, ambient shadows, sketchy lines, depth cueing,
 * background and realistic exposure. Each value says where it came from (the view, Revit's
 * default where it was not read, or the user), and each can be overridden; "Use the view's
 * settings" puts them back. Cast shadows are the Daylight panel's sun and shadows, so the two
 * panels agree.
 *
 * What the renderer cannot do as Revit does is said in the panel's notes and in
 * docs/viewer-view-display.md.
 */
import * as THREE from 'three';
import { EffectComposer } from 'three/addons/postprocessing/EffectComposer.js';
import { RenderPass } from 'three/addons/postprocessing/RenderPass.js';
import { GTAOPass } from 'three/addons/postprocessing/GTAOPass.js';
import { OutputPass } from 'three/addons/postprocessing/OutputPass.js';
import type { FileSun } from './daylight';

/** A view's display options as the model carries them (`sun_settings::ViewDisplay`, camelCase). */
export interface FileViewDisplay {
  id: number;
  name: string;
  class?: string;
  is3d: boolean;
  isTemplate: boolean;
  sunSettingId?: number;
  displayStyle?: string;
  castShadows: boolean;
  ambientShadows: boolean;
  shadowIntensity?: number;
  sunlightIntensity?: number;
  ambientIntensity?: number;
  sketchyLines?: { enabled: boolean; jitter: number; extension: number };
  depthCueing?: { enabled: boolean; start: number; end: number; fadeTo: number };
  background?: { kind: string; typeValue: number; skyColor?: string; horizonColor?: string; groundColor?: string; imagePath?: string };
  exposure?: { kind: string; typeValue: number; value?: number };
  lightingSchemeId?: number;
}

interface FileSunSetting {
  id: number;
  name: string;
  sunAndShadowType?: string;
  timeInterval?: string;
  start?: string;
  end?: string;
  azimuth?: number;
  altitude?: number;
  relativeToView: boolean;
}

/** The model's `sun` (`sun_settings::SunAndDisplay`). */
export interface FileSunAndDisplay {
  settings: FileSunSetting[];
  presets: number[];
  views: FileViewDisplay[];
  defaultView?: number;
}

export const STYLES = ['Wireframe', 'HLR', 'Shading', 'ShadingWithEdges', 'FlatColors', 'Textures', 'Realistic', 'RealisticWithEdges'] as const;
export type Style = (typeof STYLES)[number];
const STYLE_LABEL: Record<Style, string> = {
  Wireframe: 'Wireframe',
  HLR: 'Hidden Line',
  Shading: 'Shaded',
  ShadingWithEdges: 'Shaded with edges',
  FlatColors: 'Consistent Colors',
  Textures: 'Textures',
  Realistic: 'Realistic',
  RealisticWithEdges: 'Realistic with edges',
};
const BACKGROUNDS = ['None', 'Sky', 'Gradient', 'Image'] as const;
type BackgroundKind = (typeof BACKGROUNDS)[number];

export interface Settings {
  style: Style;
  castShadows: boolean;
  ambientShadows: boolean;
  /** 0 to 100, Revit's Lighting sliders. */
  sun: number;
  ambient: number;
  shadows: number;
  sketchy: boolean;
  jitter: number;
  extension: number;
  /** Whether depth cueing is drawn here (Revit draws it in elevations and sections only). */
  depthCueing: boolean;
  fadeStart: number;
  fadeEnd: number;
  fadeLimit: number;
  background: BackgroundKind;
  skyColor: string;
  horizonColor: string;
  groundColor: string;
  exposure: 'Automatic' | 'Manual';
  exposureValue: number;
}
type Key = keyof Settings;
export type Source = 'view' | 'default' | 'user';

/**
 * Revit's values where the file's were not read. The help gives Automatic exposure for new views
 * and Jitter and Extension 0 when sketchy lines are turned on; it gives no default for the
 * lighting sliders or the background colours, so those are the values most stored in the measured
 * files' 3D views (Sun 30, Ambient Light 0, Shadows 50; sky, horizon and ground colours). The
 * style has no Revit default (a view template sets it): the viewer's own shaded look with edges.
 */
export const DEFAULTS: Settings = {
  style: 'ShadingWithEdges',
  castShadows: false,
  ambientShadows: false,
  sun: 30,
  ambient: 0,
  shadows: 50,
  sketchy: false,
  jitter: 0,
  extension: 0,
  depthCueing: false,
  fadeStart: 0,
  fadeEnd: 100,
  fadeLimit: 0,
  background: 'None',
  skyColor: '#bdd1d0',
  horizonColor: '#f2f2f2',
  groundColor: '#a5a5a5',
  exposure: 'Automatic',
  exposureValue: 14,
};

export interface FromView {
  values: Settings;
  source: Record<Key, Source>;
  /** Per setting, why the value is what it is where that is not plain (shown in the panel). */
  notes: Partial<Record<Key, string>>;
}

const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));

/** The API's DisplayStyle name as one of the styles drawn here. */
export function styleOf(name: string | undefined): { style: Style; note?: string } | null {
  if (!name || name === 'Undefined') return null;
  if ((STYLES as readonly string[]).includes(name)) return { style: name as Style };
  if (name === 'Rendering') return { style: 'Realistic', note: 'The view is a Rendering view: drawn as Realistic.' };
  return null;
}

/**
 * A view's background kind. The stored type is a set of bits whose low byte is the kind in the
 * dialog's order (None, Sky, Gradient, Image): the measured files store 256, 257 and 512, and the
 * views storing 257 are the ones Revit draws with a sky (deduction, no answer key holds it).
 */
export function backgroundOf(typeValue: number): BackgroundKind | null {
  return BACKGROUNDS[typeValue & 0xff] ?? null;
}

/** The settings a view gives, with Revit's defaults for what was not read. */
export function fromView(v: FileViewDisplay | null): FromView {
  const values = { ...DEFAULTS };
  const source = Object.fromEntries(Object.keys(DEFAULTS).map((k) => [k, 'default'])) as Record<Key, Source>;
  const notes: Partial<Record<Key, string>> = {};
  if (!v) return { values, source, notes };
  const take = <K extends Key>(k: K, value: Settings[K] | undefined): void => {
    if (value === undefined) return;
    values[k] = value;
    source[k] = 'view';
  };
  const st = styleOf(v.displayStyle);
  if (st) {
    take('style', st.style);
    if (st.note) notes.style = st.note;
  }
  take('castShadows', v.castShadows);
  take('ambientShadows', v.ambientShadows);
  take('sun', v.sunlightIntensity === undefined ? undefined : clamp(v.sunlightIntensity, 0, 100));
  take('ambient', v.ambientIntensity === undefined ? undefined : clamp(v.ambientIntensity, 0, 100));
  take('shadows', v.shadowIntensity === undefined ? undefined : clamp(v.shadowIntensity, 0, 100));
  const sk = v.sketchyLines;
  if (sk) {
    take('sketchy', sk.enabled);
    take('jitter', clamp(sk.jitter, 0, 10));
    take('extension', clamp(sk.extension, 0, 10));
  }
  const dc = v.depthCueing;
  if (dc) {
    // Revit draws depth cueing in elevations and sections only, not in a 3D view.
    values.depthCueing = false;
    source.depthCueing = 'view';
    notes.depthCueing = dc.enabled
      ? 'The view has Show Depth on; Revit draws depth cueing in elevations and sections only, so it is off in this 3D view. Tick it to draw it anyway.'
      : 'Revit draws depth cueing in elevations and sections only.';
    take('fadeStart', clamp(dc.start, 0, 100));
    take('fadeEnd', clamp(dc.end, 0, 100));
    take('fadeLimit', clamp(dc.fadeTo, 0, 100));
  }
  const bg = v.background;
  if (bg) {
    const kind = backgroundOf(bg.typeValue);
    if (kind === 'Image') {
      values.background = 'None';
      source.background = 'view';
      notes.background = `The view's background is an image${bg.imagePath ? ` (${bg.imagePath})` : ''}; the file keeps only its path, not the image, so none is drawn.`;
    } else if (kind) {
      take('background', kind);
    } else {
      notes.background = `The stored background type ${bg.typeValue} is not one the viewer knows: none is drawn.`;
    }
    take('skyColor', bg.skyColor);
    take('horizonColor', bg.horizonColor);
    take('groundColor', bg.groundColor);
  }
  const ex = v.exposure;
  if (ex) {
    if (ex.kind === 'Automatic' || ex.kind === 'Manual') take('exposure', ex.kind);
    if (ex.value !== undefined) take('exposureValue', clamp(ex.value, 0, 21));
  }
  return { values, source, notes };
}

/** The sun a view gives the Daylight panel: its own setting, else the project's first preset. */
export function sunOfView(data: FileSunAndDisplay | null, v: FileViewDisplay | null): FileSun | null {
  if (!data) return null;
  const own = v?.sunSettingId !== undefined ? data.settings.find((s) => s.id === v.sunSettingId) : undefined;
  const s = own ?? data.settings.find((x) => x.id === data.presets[0]);
  if (!s) return null;
  const out: FileSun = {
    source: own ? 'view' : 'project',
    settingName: s.name,
    settingId: s.id,
    sunAndShadowType: s.sunAndShadowType,
    timeInterval: s.timeInterval,
    start: s.start,
    end: s.end,
    azimuth: s.azimuth,
    altitude: s.altitude,
    relativeToView: s.relativeToView,
  };
  if (v) {
    Object.assign(out, {
      viewName: v.name,
      viewId: v.id,
      castShadows: v.castShadows,
      ambientShadows: v.ambientShadows,
      sunlightIntensity: v.sunlightIntensity,
      shadowIntensity: v.shadowIntensity,
      displayStyle: v.displayStyle,
    });
  }
  return out;
}

/** The 3D views a user can pick: not templates, not Revit's internal ones (named `???`), by name. */
export function pickableViews(data: FileSunAndDisplay | null): FileViewDisplay[] {
  return (data?.views ?? []).filter((v) => v.is3d && !v.isTemplate && v.name !== '???').sort((a, b) => a.name.localeCompare(b.name));
}

/** Where `smoothstep(0, 1, t)` reaches `y`, for 0 < y <= 1. */
export function smoothstepInverse(y: number): number {
  let lo = 0;
  let hi = 1;
  for (let i = 0; i < 30; i++) {
    const t = (lo + hi) / 2;
    if (t * t * (3 - 2 * t) < y) lo = t;
    else hi = t;
  }
  return (lo + hi) / 2;
}

/** A light scale: Revit's slider value 0-100 over the value the viewer's lights were tuned to. */
export const sunScale = (sun: number): number => 0.5 + sun / 60;
export const ambientScale = (ambient: number): number => 1 + ambient / 100;
/** Realistic exposure: automatic is 1; a manual value is in stops about Revit's default 14, two values a stop. */
export const exposureOf = (mode: 'Automatic' | 'Manual', value: number): number => (mode === 'Automatic' ? 1 : 2 ** ((14 - value) / 2));

/** Deterministic noise in [-1, 1) for sketchy lines, so a line jitters the same way every frame. */
function noise(i: number): number {
  let h = Math.imul(i ^ 0x9e3779b9, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  return (h >>> 0) / 2 ** 31 - 1;
}

/**
 * A sketchy copy of a LINES geometry: each segment extended past its ends by `extension` and,
 * with `jitter`, drawn twice as wavering strokes. Both are in tenths of a scale tied to the
 * segment (short segments jitter and extend less, as Revit's do) and capped by the model's size
 * `d`.
 */
export function sketchyGeometry(g: THREE.BufferGeometry, jitter: number, extension: number, d: number): THREE.BufferGeometry {
  const pos = g.getAttribute('position') as THREE.BufferAttribute;
  const idx = g.getIndex();
  const refs = idx ? idx.count : pos.count;
  const strokes = jitter > 0 ? 2 : 1;
  const pieces = jitter > 0 ? 3 : 1;
  const out = new Float32Array((refs / 2) * strokes * pieces * 6);
  const a = new THREE.Vector3();
  const b = new THREE.Vector3();
  const dir = new THREE.Vector3();
  const u = new THREE.Vector3();
  const w = new THREE.Vector3();
  const p = new THREE.Vector3();
  const q = new THREE.Vector3();
  const off = new THREE.Vector3();
  const up = new THREE.Vector3(0, 1, 0);
  const side = new THREE.Vector3(1, 0, 0);
  let o = 0;
  const at = (t: number, s: number, k: number, amp: number, start: THREE.Vector3, end: THREE.Vector3, into: THREE.Vector3): void => {
    into.lerpVectors(start, end, t);
    const angle = noise(s * 131 + k * 17 + 3) * Math.PI;
    const r = noise(s * 977 + k * 29 + 11) * amp * (t === 0 || t === 1 ? 0.35 : 1);
    off.copy(u).multiplyScalar(Math.cos(angle)).addScaledVector(w, Math.sin(angle)).multiplyScalar(r);
    into.add(off);
  };
  const start = new THREE.Vector3();
  const end = new THREE.Vector3();
  for (let s = 0; s + 1 < refs; s += 2) {
    a.fromBufferAttribute(pos, idx ? idx.getX(s) : s);
    b.fromBufferAttribute(pos, idx ? idx.getX(s + 1) : s + 1);
    dir.subVectors(b, a);
    const len = dir.length();
    if (len === 0) continue;
    dir.divideScalar(len);
    u.crossVectors(dir, Math.abs(dir.y) < 0.9 ? up : side).normalize();
    w.crossVectors(dir, u);
    const ext = (extension / 10) * Math.min(0.25 * len, 0.02 * d);
    const amp = (jitter / 10) * Math.min(0.04 * len, 0.004 * d);
    for (let k = 0; k < strokes; k++) {
      const seed = (s / 2) * 2 + k;
      start.copy(a).addScaledVector(dir, -ext * (0.6 + 0.4 * Math.abs(noise(seed * 7 + 1))));
      end.copy(b).addScaledVector(dir, ext * (0.6 + 0.4 * Math.abs(noise(seed * 7 + 2))));
      at(0, seed, 0, amp, start, end, p);
      for (let piece = 1; piece <= pieces; piece++) {
        at(piece / pieces, seed, piece, amp, start, end, q);
        out[o++] = p.x;
        out[o++] = p.y;
        out[o++] = p.z;
        out[o++] = q.x;
        out[o++] = q.y;
        out[o++] = q.z;
        p.copy(q);
      }
    }
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(out.slice(0, o), 3));
  return geo;
}

export interface ViewDisplayHost {
  scene: THREE.Scene;
  renderer: THREE.WebGLRenderer;
  camera: THREE.PerspectiveCamera;
  hemi: THREE.HemisphereLight;
  sun: THREE.DirectionalLight;
  /** The theme's canvas colour: the background where the view has none. */
  canvasColour(): THREE.Color;
  /** The material a selected element is drawn in: never replaced here. */
  isHighlight(m: THREE.Material | THREE.Material[]): boolean;
  /** Daylight panel: shadows on or off, and the view's sun for its date and time. */
  setShadows(on: boolean): void;
  useSun(sun: FileSun | null): void;
  requestRender(): void;
}

type Mat = THREE.Material | THREE.Material[];

/** Whether a style draws feature edges (Revit's Show Edges in that style). */
const EDGES: Record<Style, boolean> = {
  Wireframe: true,
  HLR: true,
  Shading: false,
  ShadingWithEdges: true,
  FlatColors: true,
  Textures: false,
  Realistic: false,
  RealisticWithEdges: true,
};
const LIT = (s: Style): boolean => s === 'Shading' || s === 'ShadingWithEdges' || s === 'Realistic' || s === 'RealisticWithEdges';
const REALISTIC = (s: Style): boolean => s === 'Realistic' || s === 'RealisticWithEdges';

function el<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Record<string, string> = {}, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  if (text !== undefined) e.textContent = text;
  return e;
}

export function initViewDisplay(host: ViewDisplayHost, root: HTMLElement, enabled = true) {
  let data: FileSunAndDisplay | null = null;
  let views: FileViewDisplay[] = [];
  let view: FileViewDisplay | null = null;
  let fileState = fromView(null);
  let values: Settings = { ...DEFAULTS };
  let source: Record<Key, Source> = { ...fileState.source };
  let model: THREE.Object3D | null = null;
  let sphere = new THREE.Sphere();
  let diag = 1;
  /** Each mesh's own material from the glTF, and each line's own geometry. */
  const original = new WeakMap<THREE.Mesh, Mat>();
  const lineGeometry = new WeakMap<THREE.LineSegments, THREE.BufferGeometry>();
  /** Per original material, its copy for each style that needs one. */
  const styled = new Map<THREE.Material, Partial<Record<Style, THREE.Material>>>();
  const invisible = new THREE.MeshBasicMaterial({ visible: false });
  let sketchyKey = '';
  const sketchy = new Map<THREE.BufferGeometry, THREE.BufferGeometry>();
  let shadowsOn = false;
  let composer: EffectComposer | null = null;
  let gtao: GTAOPass | null = null;
  let fog: THREE.Fog | null = null;
  let bgTexture: THREE.CanvasTexture | null = null;
  /** The last frame drawn: its milliseconds and whether ambient shadows were in it. */
  const lastFrame = { ms: 0, ao: false };

  // ---------- materials ----------
  const averages = new WeakMap<THREE.Texture, THREE.Color>();
  /** A texture's average colour: what Shaded and Consistent Colors show of a textured material. */
  function average(t: THREE.Texture): THREE.Color | null {
    const hit = averages.get(t);
    if (hit) return hit;
    const img = t.image as CanvasImageSource | undefined;
    if (!img) return null;
    try {
      const c = document.createElement('canvas');
      c.width = c.height = 1;
      const g = c.getContext('2d', { willReadFrequently: true });
      if (!g) return null;
      g.drawImage(img, 0, 0, 1, 1);
      const [r, gg, b] = g.getImageData(0, 0, 1, 1).data;
      const colour = new THREE.Color().setRGB(r / 255, gg / 255, b / 255, THREE.SRGBColorSpace);
      averages.set(t, colour);
      return colour;
    } catch {
      return null;
    }
  }

  function copyCommon(from: THREE.Material, to: THREE.Material): THREE.Material {
    to.transparent = from.transparent;
    to.opacity = from.opacity;
    to.side = from.side;
    to.depthWrite = from.depthWrite;
    to.polygonOffset = true;
    to.polygonOffsetFactor = 1;
    to.polygonOffsetUnits = 1;
    return to;
  }

  function styledMaterial(m: THREE.Material, style: Style): THREE.Material {
    if (style === 'Realistic' || style === 'RealisticWithEdges') return m;
    if (style === 'Wireframe') return invisible;
    let set = styled.get(m);
    if (!set) styled.set(m, (set = {}));
    const key: Style = style === 'ShadingWithEdges' ? 'Shading' : style;
    const hit = set[key];
    if (hit) return hit;
    const std = m as THREE.MeshStandardMaterial;
    const flat = (): THREE.Color => (std.map ? (average(std.map) ?? std.color?.clone() ?? new THREE.Color(1, 1, 1)) : (std.color?.clone() ?? new THREE.Color(1, 1, 1)));
    let made: THREE.Material;
    if (key === 'Shading') {
      // Shaded shows the material's colour, lit, without its appearance's image
      if (!std.map) return m;
      const c = std.clone();
      c.map = null;
      c.color = flat();
      made = c;
    } else if (key === 'HLR') {
      made = copyCommon(m, new THREE.MeshBasicMaterial({ color: 0xffffff }));
    } else if (key === 'FlatColors') {
      made = copyCommon(m, new THREE.MeshBasicMaterial({ color: flat() }));
    } else {
      // Textures: the appearance's image, unlit
      made = copyCommon(m, new THREE.MeshBasicMaterial({ color: std.color?.clone() ?? new THREE.Color(1, 1, 1), map: std.map ?? null }));
    }
    set[key] = made;
    return made;
  }

  /** The Daylight panel's cut (clipping planes) carried over to the materials a style swaps in. */
  function withCut(m: Mat, planes: THREE.Plane[] | null): Mat {
    for (const x of Array.isArray(m) ? m : [m]) {
      if (x.clippingPlanes === planes || (!x.clippingPlanes?.length && !planes?.length)) continue;
      x.clippingPlanes = planes;
      x.needsUpdate = true;
    }
    return m;
  }
  const cutOf = (m: Mat): THREE.Plane[] | null => (Array.isArray(m) ? m[0] : m)?.clippingPlanes ?? null;

  function materialOf(mesh: THREE.Mesh, style: Style): Mat | undefined {
    const own = original.get(mesh);
    if (!own) return undefined;
    return Array.isArray(own) ? own.map((m) => styledMaterial(m, style)) : styledMaterial(own, style);
  }

  // ---------- applying ----------
  function applyStyle(): void {
    if (!model) return;
    const style = values.style;
    const edges = EDGES[style];
    const key = values.sketchy && (values.jitter > 0 || values.extension > 0) ? `${values.jitter}/${values.extension}` : '';
    if (key !== sketchyKey) {
      for (const g of sketchy.values()) g.dispose();
      sketchy.clear();
      sketchyKey = key;
    }
    // the cut the Daylight panel set on the materials drawn now
    let planes: THREE.Plane[] | null = null;
    model.traverse((o) => {
      if (planes === null && (o as THREE.Mesh).isMesh) planes = cutOf((o as THREE.Mesh).material);
    });
    model.traverse((o) => {
      const line = o as THREE.LineSegments;
      if (line.isLineSegments) {
        const own = lineGeometry.get(line) ?? line.geometry;
        lineGeometry.set(line, own);
        for (const m of Array.isArray(line.material) ? line.material : [line.material]) m.visible = edges;
        if (key && edges) {
          let g = sketchy.get(own);
          if (!g) sketchy.set(own, (g = sketchyGeometry(own, values.jitter, values.extension, diag)));
          line.geometry = g;
        } else line.geometry = own;
        return;
      }
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh) return;
      if (!original.has(mesh)) {
        if (host.isHighlight(mesh.material)) return;
        original.set(mesh, mesh.material);
      }
      if (host.isHighlight(mesh.material)) return;
      mesh.material = withCut(materialOf(mesh, style)!, planes);
    });
  }

  function paintBackground(): void {
    bgTexture?.dispose();
    bgTexture = null;
    const kind = values.background;
    if (kind === 'None' || kind === 'Image') {
      host.scene.background = host.canvasColour();
      return;
    }
    const c = document.createElement('canvas');
    c.width = 2;
    c.height = 256;
    const g = c.getContext('2d')!;
    const grad = g.createLinearGradient(0, 0, 0, 256);
    grad.addColorStop(0, values.skyColor);
    if (kind === 'Sky') grad.addColorStop(1, values.horizonColor);
    else {
      grad.addColorStop(0.5, values.horizonColor);
      grad.addColorStop(1, values.groundColor);
    }
    g.fillStyle = grad;
    g.fillRect(0, 0, 2, 256);
    bgTexture = new THREE.CanvasTexture(c);
    bgTexture.colorSpace = THREE.SRGBColorSpace;
    host.scene.background = bgTexture;
  }

  function fogColour(): THREE.Color {
    if (values.background === 'Sky' || values.background === 'Gradient') return new THREE.Color(values.horizonColor);
    return host.canvasColour();
  }

  function applyAll(fromUser = false): void {
    if (!enabled) return;
    applyStyle();
    // lighting: the sliders scale the lights the viewer was tuned to; unlit styles ignore them
    host.hemi.intensity = 2.0 * ambientScale(values.ambient);
    if (!shadowsOn) host.sun.intensity = 1.6 * sunScale(values.sun);
    host.sun.shadow.intensity = values.shadows / 100;
    // exposure: Realistic only
    const r = host.renderer;
    if (REALISTIC(values.style)) {
      r.toneMapping = THREE.ACESFilmicToneMapping;
      r.toneMappingExposure = exposureOf(values.exposure, values.exposureValue);
    } else {
      r.toneMapping = THREE.NoToneMapping;
      r.toneMappingExposure = 1;
    }
    paintBackground();
    if (values.depthCueing && values.fadeLimit < 100) {
      fog ??= new THREE.Fog(0xffffff, 1, 2);
      fog.color = fogColour();
      host.scene.fog = fog;
    } else host.scene.fog = null;
    // an explicit off goes to the Daylight panel even when already off, to drop a sun request still on its way
    if (values.castShadows !== shadowsOn || fromUser) {
      shadowsOn = values.castShadows;
      host.setShadows(values.castShadows);
    }
    show();
    describeScene();
    host.requestRender();
  }

  /** What the 3D scene now draws, for the tests: materials by kind, edges, lights, exposure, fog, background. */
  function describeScene(): void {
    const materials: Record<string, number> = {};
    let edges = 0;
    let edgesShown = false;
    let segments = 0;
    model?.traverse((o) => {
      const line = o as THREE.LineSegments;
      if (line.isLineSegments) {
        edges++;
        edgesShown ||= (Array.isArray(line.material) ? line.material : [line.material]).some((m) => m.visible);
        const g = line.geometry;
        segments += (g.getIndex()?.count ?? g.getAttribute('position').count) / 2;
        return;
      }
      const mesh = o as THREE.Mesh;
      if (!mesh.isMesh) return;
      for (const m of Array.isArray(mesh.material) ? mesh.material : [mesh.material]) {
        const k = m.visible ? m.type : 'hidden';
        materials[k] = (materials[k] ?? 0) + 1;
      }
    });
    const bg = host.scene.background;
    root.dataset.scene = JSON.stringify({
      materials,
      edges,
      edgesShown,
      segments,
      hemi: Math.round(host.hemi.intensity * 1000) / 1000,
      sun: Math.round(host.sun.intensity * 1000) / 1000,
      shadowIntensity: host.sun.shadow.intensity,
      toneMapping: host.renderer.toneMapping,
      exposure: Math.round(host.renderer.toneMappingExposure * 1000) / 1000,
      fog: host.scene.fog !== null,
      background: bg instanceof THREE.Color ? `#${bg.getHexString()}` : bg ? 'gradient' : null,
    });
  }

  // ---------- drawing ----------
  function ambientShadowsDrawn(): boolean {
    return enabled && values.ambientShadows && values.style !== 'Wireframe' && model !== null;
  }

  /** Draws a frame: ambient shadows only while the camera is still, the cost they add. */
  function render(moving: boolean): void {
    const t = performance.now();
    if (fog && host.scene.fog === fog) {
      const d = host.camera.position.distanceTo(sphere.center);
      const n0 = Math.max(0, d - sphere.radius);
      const f0 = d + sphere.radius;
      const near = n0 + (values.fadeStart / 100) * (f0 - n0);
      const end = Math.max(near + 1e-3, n0 + (values.fadeEnd / 100) * (f0 - n0));
      fog.near = near;
      // smoothstep fog reaches the fade limit at the fade end, not full fade
      fog.far = near + (end - near) / smoothstepInverse(1 - values.fadeLimit / 100);
    }
    const ao = ambientShadowsDrawn() && !moving;
    if (ao) {
      const size = host.renderer.getSize(new THREE.Vector2());
      if (!composer) {
        composer = new EffectComposer(host.renderer);
        composer.addPass(new RenderPass(host.scene, host.camera));
        gtao = new GTAOPass(host.scene, host.camera, size.x, size.y);
        composer.addPass(gtao);
        composer.addPass(new OutputPass());
      }
      composer.setPixelRatio(host.renderer.getPixelRatio());
      composer.setSize(size.x, size.y);
      gtao!.updateGtaoMaterial({ radius: Math.max(0.25, Math.min(1, diag * 0.01)) });
      composer.render();
    } else host.renderer.render(host.scene, host.camera);
    lastFrame.ms = performance.now() - t;
    lastFrame.ao = ao;
    root.dataset.frame = JSON.stringify({ ms: Math.round(lastFrame.ms * 100) / 100, ao });
  }

  // ---------- the panel ----------
  root.replaceChildren();
  root.dataset.state = enabled ? 'idle' : 'off';
  const intro = el('p', { class: 'option-hint' }, 'The chosen 3D view’s Graphic Display Options from the file, drawn as Revit shows that view. Change any of them here; the file is not changed.');
  const viewSel = el('select', { id: 'vd-view', 'aria-label': '3D view' });
  const viewRow = el('label', { class: 'daylight-row' });
  viewRow.append(el('span', {}, '3D view'), viewSel, el('span'));
  const viewNote = el('p', { class: 'option-hint', id: 'vd-view-note' });
  const rows = el('div', { class: 'vd-rows' });
  const tags = {} as Record<Key, HTMLElement>;
  const inputs = {} as Partial<Record<Key, HTMLInputElement | HTMLSelectElement>>;
  const outs = {} as Partial<Record<Key, HTMLOutputElement>>;

  function addRow(parent: HTMLElement, key: Key, label: string, input: HTMLInputElement | HTMLSelectElement, unit = ''): void {
    const row = el('label', { class: 'daylight-row' });
    const out = el('output', { class: 'daylight-unit' }, unit);
    row.append(el('span', {}, label), input, out);
    const tag = el('span', { class: 'daylight-source', id: `vd-${key}-source` });
    row.append(tag);
    parent.append(row);
    tags[key] = tag;
    inputs[key] = input;
    if (input instanceof HTMLInputElement && input.type === 'range') outs[key] = out;
    input.addEventListener(input instanceof HTMLInputElement && input.type === 'range' ? 'input' : 'change', () => {
      const v = input instanceof HTMLInputElement ? (input.type === 'checkbox' ? input.checked : input.type === 'range' ? Number(input.value) : input.value) : input.value;
      (values as unknown as Record<string, unknown>)[key] = v;
      source[key] = 'user';
      applyAll(true);
    });
  }
  const range = (key: Key, max: number, step = 1): HTMLInputElement => el('input', { type: 'range', min: '0', max: String(max), step: String(step), id: `vd-${key}` });
  const check = (key: Key): HTMLInputElement => el('input', { type: 'checkbox', id: `vd-${key}` });
  const select = (key: Key, options: readonly string[], labels?: Record<string, string>): HTMLSelectElement => {
    const s = el('select', { id: `vd-${key}` });
    for (const o of options) s.append(el('option', { value: o }, labels?.[o] ?? o));
    return s;
  };
  const colour = (key: Key): HTMLInputElement => el('input', { type: 'color', id: `vd-${key}` });

  const group = (title: string): HTMLFieldSetElement => {
    const f = el('fieldset', { class: 'daylight-site' });
    f.append(el('legend', {}, title));
    rows.append(f);
    return f;
  };
  const model_ = group('Model display');
  addRow(model_, 'style', 'Style', select('style', STYLES, STYLE_LABEL));
  const shadowsG = group('Shadows');
  addRow(shadowsG, 'castShadows', 'Cast shadows', check('castShadows'));
  addRow(shadowsG, 'ambientShadows', 'Ambient shadows', check('ambientShadows'));
  const lightG = group('Lighting');
  const schemeRow = el('div', { class: 'daylight-row' });
  schemeRow.append(el('span', {}, 'Scheme'), el('span', { id: 'vd-scheme' }, 'not read'), el('span'));
  const schemeTag = el('span', { class: 'daylight-source', 'data-source': 'default', id: 'vd-scheme-source' });
  schemeRow.append(schemeTag);
  lightG.append(schemeRow);
  addRow(lightG, 'sun', 'Sun', range('sun', 100));
  addRow(lightG, 'ambient', 'Ambient light', range('ambient', 100));
  addRow(lightG, 'shadows', 'Shadows', range('shadows', 100));
  const sketchG = group('Sketchy lines');
  addRow(sketchG, 'sketchy', 'Enable', check('sketchy'));
  addRow(sketchG, 'jitter', 'Jitter', range('jitter', 10));
  addRow(sketchG, 'extension', 'Extension', range('extension', 10));
  const depthG = group('Depth cueing');
  addRow(depthG, 'depthCueing', 'Show depth', check('depthCueing'));
  addRow(depthG, 'fadeStart', 'Fade start', range('fadeStart', 100), '%');
  addRow(depthG, 'fadeEnd', 'Fade end', range('fadeEnd', 100), '%');
  addRow(depthG, 'fadeLimit', 'Fade limit', range('fadeLimit', 100), '%');
  const bgG = group('Background');
  addRow(bgG, 'background', 'Background', select('background', BACKGROUNDS.filter((b) => b !== 'Image')));
  addRow(bgG, 'skyColor', 'Sky', colour('skyColor'));
  addRow(bgG, 'horizonColor', 'Horizon', colour('horizonColor'));
  addRow(bgG, 'groundColor', 'Ground', colour('groundColor'));
  const exG = group('Realistic exposure');
  addRow(exG, 'exposure', 'Exposure', select('exposure', ['Automatic', 'Manual']));
  addRow(exG, 'exposureValue', 'Value', range('exposureValue', 21, 0.5));
  const reset = el('button', { type: 'button', class: 'side-btn', id: 'vd-reset' }, 'Use the view’s settings');
  const notesEl = el('ul', { class: 'option-hint vd-notes', id: 'vd-notes' });
  root.append(intro, viewRow, viewNote, rows, reset, notesEl);

  reset.addEventListener('click', () => {
    values = { ...fileState.values };
    source = { ...fileState.source };
    host.useSun(sunOfView(data, view));
    applyAll(true);
  });
  viewSel.addEventListener('change', () => {
    chooseView(views.find((v) => String(v.id) === viewSel.value) ?? null);
  });

  function sourceText(k: Key): string {
    const s = source[k];
    if (s === 'user') return 'set by you';
    if (s === 'view') return view ? `from the view “${view.name}”` : 'from the file';
    return 'not read: Revit’s default';
  }

  function show(): void {
    for (const k of Object.keys(DEFAULTS) as Key[]) {
      const input = inputs[k];
      if (input) {
        const v = values[k];
        if (input instanceof HTMLInputElement && input.type === 'checkbox') input.checked = v as boolean;
        else input.value = String(v);
      }
      const out = outs[k];
      if (out) out.textContent = `${values[k]}${k.startsWith('fade') ? ' %' : ''}`;
      tags[k].textContent = sourceText(k);
      tags[k].dataset.source = source[k] === 'default' ? 'assumed' : source[k];
    }
    schemeTag.textContent = view?.lightingSchemeId !== undefined
      ? 'not read: the file names the scheme by an element the reader does not decode yet; exterior, sun only is drawn'
      : 'not read: exterior, sun only is drawn';
    const notes: string[] = [];
    for (const n of Object.values(fileState.notes)) if (n) notes.push(n);
    if (!LIT(values.style)) notes.push(`${STYLE_LABEL[values.style]} is drawn unlit: the Sun and Ambient Light sliders, and shadows on the model, show in the lit styles only.`);
    if (!REALISTIC(values.style)) notes.push('Realistic exposure applies in the Realistic styles only.');
    if (values.style === 'Textures' || REALISTIC(values.style)) notes.push('Textures are the material appearances the export carries; artificial lights are not drawn.');
    if (values.background === 'Sky') notes.push('The sky is drawn as a fixed gradient from the sky to the horizon colour; Revit’s sky follows the camera’s tilt.');
    if (values.ambientShadows) notes.push('Ambient shadows are drawn once the camera stops, and left out while it moves.');
    notesEl.replaceChildren(...notes.map((n) => el('li', {}, n)));
    root.dataset.applied = JSON.stringify({ view: view?.name ?? null, values, source });
  }

  function chooseView(v: FileViewDisplay | null): void {
    view = v;
    fileState = fromView(v);
    values = { ...fileState.values };
    source = { ...fileState.source };
    viewNote.textContent = v
      ? 'The view’s camera is not read: the 3D view keeps its own.'
      : data
        ? 'The file has no 3D view to read the options from (only templates or none): Revit’s defaults are drawn.'
        : 'This file’s view display options were not read: Revit’s defaults are drawn.';
    root.dataset.state = enabled ? 'ready' : 'off';
    host.useSun(sunOfView(data, v));
    applyAll(true);
  }

  show();

  return {
    /** A new model's scene is in place: lists its 3D views and applies the default one. */
    modelChanged(root3d: THREE.Object3D, sun: FileSunAndDisplay | null | undefined): void {
      model = root3d;
      const box = new THREE.Box3().setFromObject(root3d);
      sphere = box.isEmpty() ? new THREE.Sphere(new THREE.Vector3(), 1) : box.getBoundingSphere(new THREE.Sphere());
      diag = Math.max(1, sphere.radius * 2);
      data = sun ?? null;
      views = pickableViews(data);
      viewSel.replaceChildren(...(views.length ? views.map((v) => el('option', { value: String(v.id) }, v.name)) : [el('option', { value: '' }, data ? 'No 3D view in the file' : 'Not read')]));
      viewSel.disabled = views.length === 0;
      const first = views.find((v) => v.id === data?.defaultView) ?? views[0] ?? null;
      if (first) viewSel.value = String(first.id);
      if (!enabled) {
        root.dataset.state = 'off';
        return;
      }
      chooseView(first);
    },
    /** The Daylight panel turned its shadows on or off: the view's cast shadows follow. */
    shadowsChanged(on: boolean): void {
      if (on === shadowsOn) return;
      shadowsOn = on;
      values.castShadows = on;
      source.castShadows = 'user';
      show();
    },
    themeChanged(): void {
      if (!enabled) return;
      paintBackground();
      if (fog) fog.color = fogColour();
    },
    materialFor(mesh: THREE.Mesh): Mat | undefined {
      const m = enabled && model ? materialOf(mesh, values.style) : undefined;
      return m && withCut(m, cutOf(mesh.material));
    },
    /** Whether a still frame after a move differs from a moving one (ambient shadows). */
    degradesWhileMoving: ambientShadowsDrawn,
    render,
    /** The sun's intensity over the viewer's own, from the Sun slider (the Daylight panel's sun uses it). */
    sunScale: (): number => (enabled ? sunScale(values.sun) : 1),
    /** How dark a shadow on the ground is, 0 to 1, from the Shadows slider (0.28 at Revit's 50). */
    shadowOpacity: (): number => (enabled ? 0.56 * (values.shadows / 100) : 0.28),
    reset(): void {
      // the old model gets its own materials and edges back, so whoever disposes it disposes those
      model?.traverse((o) => {
        const line = o as THREE.LineSegments;
        if (line.isLineSegments) {
          line.geometry = lineGeometry.get(line) ?? line.geometry;
          for (const m of Array.isArray(line.material) ? line.material : [line.material]) m.visible = true;
          return;
        }
        const mesh = o as THREE.Mesh;
        const own = mesh.isMesh ? original.get(mesh) : undefined;
        if (own && !host.isHighlight(mesh.material)) mesh.material = own;
      });
      for (const set of styled.values()) for (const m of Object.values(set)) m?.dispose();
      model = null;
      data = null;
      view = null;
      views = [];
      shadowsOn = false;
      styled.clear();
      for (const g of sketchy.values()) g.dispose();
      sketchy.clear();
      sketchyKey = '';
      fileState = fromView(null);
      values = { ...DEFAULTS };
      source = { ...fileState.source };
      host.scene.fog = null;
      host.renderer.toneMapping = THREE.NoToneMapping;
      host.renderer.toneMappingExposure = 1;
      host.hemi.intensity = 2.0;
      host.sun.intensity = 1.6;
      host.sun.shadow.intensity = 1;
      bgTexture?.dispose();
      bgTexture = null;
      host.scene.background = host.canvasColour();
      viewSel.replaceChildren();
      root.dataset.state = enabled ? 'idle' : 'off';
      show();
    },
  };
}
