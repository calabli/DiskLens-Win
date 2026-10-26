import { Channel, invoke } from "@tauri-apps/api/core";
import { CATEGORY_KEYS, type CategoryKey } from "./categories";

export type ScanMode = "mft" | "walk";

export interface Volume {
  root: string;
  label: string;
  fileSystem: string;
  total: number;
  free: number;
}

export type ScanEvent =
  | { kind: "started"; mode: ScanMode; fallbackReason: string | null }
  | { kind: "progress"; processed: number; elapsedMs: number };

export interface ScanSummary {
  root: string;
  rootId: number;
  mode: ScanMode;
  fallbackReason: string | null;
  totalSize: number;
  totalAllocated: number;
  fileCount: number;
  nodeCount: number;
  skipped: number;
  elapsedMs: number;
}

export interface Crumb {
  id: number;
  name: string;
}

export interface NodeInfo {
  id: number;
  name: string;
  path: string;
  size: number;
  allocated: number;
  files: number;
  isDir: boolean;
  parent: number | null;
  crumbs: Crumb[];
}

export interface Entry {
  id: number;
  name: string;
  size: number;
  allocated: number;
  files: number;
  isDir: boolean;
  hasChildren: boolean;
  category: CategoryKey;
}

export interface ChildList {
  total: number;
  entries: Entry[];
}

export interface TypeStat {
  key: CategoryKey;
  label: string;
  size: number;
  allocated: number;
  files: number;
}

export interface SearchHit {
  id: number;
  name: string;
  path: string;
  parent: number | null;
  size: number;
  isDir: boolean;
  category: CategoryKey;
}

export interface TreemapRect {
  id: number;
  category: CategoryKey;
  x: number;
  y: number;
  w: number;
  h: number;
}

export const listVolumes = () => invoke<Volume[]>("list_volumes");

/** 开始扫描。扫描被取消时 Promise 以“已取消”拒绝。 */
export function startScan(path: string, onEvent: (e: ScanEvent) => void) {
  const channel = new Channel<ScanEvent>();
  channel.onmessage = onEvent;
  return invoke<ScanSummary>("start_scan", { path, onEvent: channel });
}

export const cancelScan = () => invoke<void>("cancel_scan");

export const nodeInfo = (id: number) => invoke<NodeInfo>("node_info", { id });

export const listChildren = (id: number, filter: string | null) =>
  invoke<ChildList>("list_children", { id, filter });

export const typeStats = (id: number) => invoke<TypeStat[]>("type_stats", { id });

export const search = (id: number, query: string, limit: number) =>
  invoke<SearchHit[]>("search", { id, query, limit });

/** 每块 24 字节：id u32、类型下标 u32、x/y/w/h f32，小端。见 Rust 侧 `commands::treemap`。 */
const RECT_BYTES = 24;

export async function treemap(id: number, width: number, height: number): Promise<TreemapRect[]> {
  const buf = await invoke<ArrayBuffer>("treemap", { id, width, height });
  const view = new DataView(buf);
  const rects: TreemapRect[] = [];
  for (let off = 0; off + RECT_BYTES <= buf.byteLength; off += RECT_BYTES) {
    rects.push({
      id: view.getUint32(off, true),
      category: CATEGORY_KEYS[view.getUint32(off + 4, true)] ?? "other",
      x: view.getFloat32(off + 8, true),
      y: view.getFloat32(off + 12, true),
      w: view.getFloat32(off + 16, true),
      h: view.getFloat32(off + 20, true),
    });
  }
  return rects;
}
