/*
 * The open Revit model's linked and imported files, beside the scene tree: the rows of the
 * authoring tool's Manage Links dialog (kind, name, status, Attachment or Overlay, saved path and
 * path type, size, placements), read by the parse worker (worker.ts, `linksJson`). A Revit link
 * whose file the user gives (its button, or dropping the file while the model is open) is drawn
 * as glTF by the worker and placed once per placement with the transform the host file stores.
 * Nothing is fetched: a link whose file is not given stays listed as not loaded, with its saved
 * path.
 */
import * as THREE from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';

export interface ModelLinksHost {
  /** The worker that opened the model, or null. */
  worker(): Worker | null;
  /** The file it opened: read again for each request, so no copy is kept. */
  file(): File | null;
  scene: THREE.Scene;
  requestRender(): void;
  setStatus(text: string): void;
  toast(message: string, kind?: 'ok' | 'bad' | 'info'): void;
}

export interface ManageLinksRow {
  id: number;
  kind: string;
  link_name: string;
  status: string;
  reference_type: string;
  saved_path: string;
  path_type: string;
  file_size?: number;
  resolution?: [number, number];
  count: number;
}

export interface LinkInstance {
  id: number;
  class: string;
  type_id: number;
  name?: string;
  /** x axis, y axis, z axis, origin; feet. */
  transform: [number[], number[], number[], number[]];
  view?: number;
}

interface LinksJson {
  manage_links: ManageLinksRow[];
  links: { types: { id: number; kind: string; name: string; parent?: number }[]; instances: LinkInstance[] };
}

type Reply =
  | { type: 'links'; json: string }
  | { type: 'link-glb'; name: string; req: number; glb: Uint8Array }
  | { type: 'links-error'; request: string; req?: number; message: string };

/** The model's frame (feet, +Z up) to glTF's (metres, +Y up), as rvt-open's glTF root writes it. */
const C = new THREE.Matrix4().set(0.3048, 0, 0, 0, 0, 0, 0.3048, 0, 0, -0.3048, 0, 0, 0, 0, 0, 1);
const C_INV = C.clone().invert();

/**
 * Where a link placed with `t` (its axes and origin in the host's feet) puts the linked model's
 * glTF scene, in the host's glTF frame: C · T · C⁻¹.
 */
export function linkMatrix(t: LinkInstance['transform']): THREE.Matrix4 {
  const [x, y, z, o] = t;
  const m = new THREE.Matrix4().set(x[0], y[0], z[0], o[0], x[1], y[1], z[1], o[1], x[2], y[2], z[2], o[2], 0, 0, 0, 1);
  return C.clone().multiply(m).multiply(C_INV);
}

const baseName = (p: string): string => (p.split(/[\\/]/).pop() ?? p).toLowerCase();

export function initModelLinks(
  host: ModelLinksHost,
  listEl: HTMLElement,
): { load(): Promise<void>; reset(): void; tryFile(file: File): boolean; rows(): ManageLinksRow[] } {
  let data: LinksJson | null = null;
  let listening: Worker | null = null;
  let nextReq = 0;
  /** The groups placed for each link type, by type id. */
  const placed = new Map<number, THREE.Group[]>();
  /** Requests waiting for their glTF: request → link type. */
  const waiting = new Map<number, number>();
  const picker = document.createElement('input');
  picker.type = 'file';
  picker.accept = '.rvt';
  picker.hidden = true;
  listEl.after(picker);
  let pickFor: number | null = null;

  const clearPlaced = (): void => {
    for (const groups of placed.values()) {
      for (const g of groups) {
        host.scene.remove(g);
        g.traverse((o) => {
          const mesh = o as THREE.Mesh;
          mesh.geometry?.dispose();
          const mat = mesh.material;
          if (Array.isArray(mat)) mat.forEach((m) => m.dispose());
          else (mat as THREE.Material | undefined)?.dispose();
        });
      }
    }
    placed.clear();
    host.requestRender();
  };

  const statusOf = (row: ManageLinksRow): string => (placed.has(row.id) ? 'Loaded (file given)' : row.status);

  const render = (): void => {
    listEl.innerHTML = '';
    listEl.removeAttribute('aria-busy');
    const rows = data?.manage_links ?? [];
    listEl.dataset.count = String(rows.length);
    if (rows.length === 0) {
      const p = document.createElement('p');
      p.className = 'links-empty';
      p.textContent = data ? 'No linked or imported files' : '';
      listEl.appendChild(p);
      return;
    }
    const table = document.createElement('table');
    table.className = 'links-table';
    const head = table.createTHead().insertRow();
    for (const h of ['Kind', 'Link name', 'Status', 'Reference', 'Path type', 'Saved path', 'Count']) {
      const th = document.createElement('th');
      th.textContent = h;
      head.appendChild(th);
    }
    const body = table.createTBody();
    for (const row of rows) {
      const tr = body.insertRow();
      tr.dataset.id = String(row.id);
      tr.dataset.kind = row.kind;
      tr.dataset.status = statusOf(row);
      const cells = [
        row.kind,
        row.link_name,
        statusOf(row),
        row.reference_type,
        row.path_type,
        row.saved_path,
        String(row.count),
      ];
      cells.forEach((text, i) => {
        const td = tr.insertCell();
        td.textContent = text;
        if (i === 5) td.title = text;
      });
      if ((row.kind === 'revit' || row.kind === 'ifc') && row.count > 0) {
        const td = tr.insertCell();
        const b = document.createElement('button');
        b.type = 'button';
        b.className = 'side-btn';
        b.dataset.linkOpen = String(row.id);
        b.textContent = placed.has(row.id) ? 'Remove' : 'Open file…';
        b.title = placed.has(row.id)
          ? `Remove ${row.link_name} from the scene`
          : `Give ${row.link_name} (a .rvt) to place it where this model links it; nothing is fetched`;
        b.addEventListener('click', () => {
          if (placed.has(row.id)) {
            for (const g of placed.get(row.id) ?? []) host.scene.remove(g);
            placed.delete(row.id);
            host.requestRender();
            render();
            return;
          }
          pickFor = row.id;
          picker.value = '';
          picker.click();
        });
        td.appendChild(b);
        // Its placements, as the scene tree shows a link's instances.
        for (const inst of data?.links.instances.filter((i) => i.type_id === row.id) ?? []) {
          const sub = body.insertRow();
          sub.className = 'links-instance';
          sub.dataset.instance = String(inst.id);
          const c = sub.insertCell();
          c.colSpan = 8;
          const o = inst.transform[3];
          c.textContent = `↳ ${row.link_name} : ${inst.name ?? inst.id} · at (${o.map((v) => (v * 0.3048).toFixed(2)).join(', ')}) m`;
        }
      }
    }
    listEl.appendChild(table);
  };

  const place = async (typeId: number, glb: Uint8Array): Promise<void> => {
    const loader = new GLTFLoader();
    const gltf = await loader.parseAsync(glb.buffer.slice(glb.byteOffset, glb.byteOffset + glb.byteLength) as ArrayBuffer, '');
    const groups: THREE.Group[] = [];
    const instances = data?.links.instances.filter((i) => i.type_id === typeId) ?? [];
    instances.forEach((inst, k) => {
      const g = new THREE.Group();
      g.name = `link ${typeId} ${inst.id}`;
      g.userData.linkInstance = inst.id;
      g.add(k === 0 ? gltf.scene : gltf.scene.clone(true));
      g.matrixAutoUpdate = false;
      g.matrix.copy(linkMatrix(inst.transform));
      g.updateMatrixWorld(true);
      host.scene.add(g);
      groups.push(g);
    });
    placed.set(typeId, groups);
    host.requestRender();
    render();
  };

  const onMessage = (ev: MessageEvent<Reply>): void => {
    const msg = ev.data;
    if (msg.type === 'links') {
      try {
        data = JSON.parse(msg.json) as LinksJson;
      } catch {
        data = null;
      }
      render();
    } else if (msg.type === 'link-glb') {
      const typeId = waiting.get(msg.req);
      waiting.delete(msg.req);
      if (typeId === undefined) return;
      void place(typeId, msg.glb).then(
        () => host.toast(`Placed ${msg.name} where this model links it`, 'ok'),
        (err) => host.toast(`Could not place ${msg.name}: ${(err as Error).message}`, 'bad'),
      );
      host.setStatus(`Linked ${msg.name}`);
    } else if (msg.type === 'links-error') {
      if (msg.req !== undefined) waiting.delete(msg.req);
      listEl.removeAttribute('aria-busy');
      host.toast(`Links: ${msg.message}`, 'bad');
    }
  };

  const listen = (): Worker | null => {
    const w = host.worker();
    if (w && w !== listening) {
      listening?.removeEventListener('message', onMessage as EventListener);
      w.addEventListener('message', onMessage as EventListener);
      listening = w;
    }
    return w;
  };

  /** Read `file` and place it as link type `typeId`. */
  const openAs = async (typeId: number, file: File): Promise<void> => {
    const w = listen();
    if (!w) return;
    const bytes = new Uint8Array(await file.arrayBuffer());
    const req = ++nextReq;
    waiting.set(req, typeId);
    host.setStatus(`Reading linked file ${file.name}…`);
    listEl.setAttribute('aria-busy', 'true');
    w.postMessage({ type: 'link-glb', bytes, name: file.name, req }, [bytes.buffer]);
  };

  picker.addEventListener('change', () => {
    const f = picker.files?.[0];
    if (f && pickFor !== null) void openAs(pickFor, f);
    pickFor = null;
  });

  return {
    async load(): Promise<void> {
      const w = listen();
      const file = host.file();
      if (!w || !file || !/\.(rvt|rte)$/i.test(file.name)) {
        data = null;
        render();
        return;
      }
      listEl.setAttribute('aria-busy', 'true');
      const bytes = new Uint8Array(await file.arrayBuffer());
      if (host.file() !== file) return;
      w.postMessage({ type: 'links', bytes }, [bytes.buffer]);
    },
    reset(): void {
      clearPlaced();
      waiting.clear();
      data = null;
      render();
    },
    /**
     * A dropped file that is one of the open model's Revit links (by file name): placed as that
     * link, and true; anything else is false, for the page to open as a model.
     */
    tryFile(file: File): boolean {
      if (!data || !/\.rvt$/i.test(file.name)) return false;
      const name = file.name.toLowerCase();
      const row = data.manage_links.find(
        (r) => (r.kind === 'revit' || r.kind === 'ifc') && r.count > 0 && (r.link_name.toLowerCase() === name || baseName(r.saved_path) === name),
      );
      if (!row) return false;
      void openAs(row.id, file);
      return true;
    },
    rows: () => data?.manage_links ?? [],
  };
}
