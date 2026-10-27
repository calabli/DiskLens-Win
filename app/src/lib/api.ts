import { Channel, invoke } from "@tauri-apps/api/core";
import { CATEGORY_KEYS, type CategoryKey } from "./categories";

export type ScanMode = "mft" | "walk";

/** 列表里的选择方式：只选这一项、加选或取消（Ctrl）、连续选择（Shift）。 */
export type SelectMode = "only" | "toggle" | "range";

export interface Volume {
  root: string;
  label: string;
  fileSystem: string;
  total: number;
  free: number;
}

/** 边扫边看：一个顶层项目目前累计的大小。 */
export interface LiveEntry {
  name: string;
  isDir: boolean;
  size: number;
  files: number;
}

export type ScanEvent =
  | { kind: "started"; mode: ScanMode; fallbackReason: string | null }
  | { kind: "progress"; processed: number; elapsedMs: number }
  | { kind: "live"; items: LiveEntry[]; totalSize: number };

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
  /** 扫描完成的时间（Unix 毫秒）。 */
  scannedAt: number;
  /** 是否是从缓存读回的上次结果。 */
  fromCache: boolean;
}

export interface CachedScan {
  root: string;
  mode: ScanMode;
  scannedAt: number;
  totalSize: number;
  fileCount: number;
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

export interface PlanItem {
  id: number;
  name: string;
  path: string;
  size: number;
  files: number;
  isDir: boolean;
  /** 不能删除时的原因。 */
  blocked: string | null;
}

export interface DeletePlan {
  items: PlanItem[];
  totalSize: number;
  totalFiles: number;
  logDir: string;
}

export interface DeleteResult {
  deleted: number[];
  failed: { path: string; reason: string }[];
  freed: number;
  logError: string | null;
}

export const listVolumes = () => invoke<Volume[]>("list_volumes");

export const planDelete = (ids: number[]) => invoke<DeletePlan>("plan_delete", { ids });

export const deleteItems = (ids: number[], permanent: boolean) =>
  invoke<DeleteResult>("delete_items", { ids, permanent });

/** 开始扫描。扫描被取消时 Promise 以“已取消”拒绝。 */
export function startScan(path: string, onEvent: (e: ScanEvent) => void) {
  const channel = new Channel<ScanEvent>();
  channel.onmessage = onEvent;
  return invoke<ScanSummary>("start_scan", { path, onEvent: channel });
}

export const cancelScan = () => invoke<void>("cancel_scan");

export type RefreshResult =
  | { kind: "updated"; added: number; removed: number; updated: number; elapsedMs: number }
  | { kind: "needsFullScan"; reason: string };

/** 按 USN 日志增量刷新当前结果。不能增量刷新时返回原因，由调用方决定是否完整重扫。 */
export const refreshScan = () => invoke<RefreshResult>("refresh_scan");

export interface DupProgress {
  phase: "checking" | "hashing";
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
}

export interface DupFile {
  id: number;
  path: string;
  modifiedMs: number;
  /** 硬链接数；大于 1 时删除这个名称不释放空间。 */
  links: number;
  /** 所在卷的序列号；只有同一个卷上的文件才能替换为硬链接。 */
  volume: number;
}

export interface DupGroup {
  size: number;
  files: DupFile[];
}

export interface DupResult {
  groups: DupGroup[];
  moreGroups: number;
  wasted: number;
  candidates: number;
  skipped: number;
  sameFile: number;
  bytesRead: number;
  elapsedMs: number;
}

/** 在目录 `id` 下查找重复文件。被取消时 Promise 以“已取消”拒绝。 */
export function findDuplicates(id: number, minSize: number, onProgress: (p: DupProgress) => void) {
  const channel = new Channel<DupProgress>();
  channel.onmessage = onProgress;
  return invoke<DupResult>("find_duplicates", { id, minSize, onEvent: channel });
}

export const cancelDuplicates = () => invoke<void>("cancel_duplicates");

/** 一组重复文件：保留 `keep`，把 `replace` 替换为指向它的硬链接。 */
export interface LinkGroup {
  keep: number;
  replace: number[];
}

export interface LinkResult {
  linked: number[];
  failed: { path: string; reason: string }[];
  freed: number;
  logError: string | null;
}

export const linkItems = (groups: LinkGroup[]) => invoke<LinkResult>("link_items", { groups });

export type FindingKind = "emptyDir" | "emptyFile" | "brokenLink" | "brokenShortcut";

export interface Finding {
  id: number;
  path: string;
  kind: FindingKind;
  /** 链接或快捷方式原本指向的目标。 */
  target: string | null;
}

export interface FindResult {
  items: Finding[];
  more: number;
  checked: number;
  elapsedMs: number;
}

/** 在目录 `id` 下查找空文件夹、空文件和失效的链接。被取消时以“已取消”拒绝。 */
export function findIssues(id: number, onProgress: (done: number, total: number) => void) {
  const channel = new Channel<{ done: number; total: number }>();
  channel.onmessage = (p) => onProgress(p.done, p.total);
  return invoke<FindResult>("find_issues", { id, onEvent: channel });
}

export const cancelIssues = () => invoke<void>("cancel_issues");

export interface SimilarImage {
  id: number;
  path: string;
  size: number;
  modifiedMs: number;
  width: number;
  height: number;
  /** 与组里第一张（画质最好的）相差的位数。 */
  distance: number;
}

export interface SimilarResult {
  groups: { files: SimilarImage[] }[];
  moreGroups: number;
  wasted: number;
  images: number;
  skipped: number;
  decoded: number;
  elapsedMs: number;
}

/** 在目录 `id` 下查找相似图片。被取消时以“已取消”拒绝。 */
export function findSimilar(
  id: number,
  minSize: number,
  maxDistance: number,
  onProgress: (done: number, total: number) => void,
) {
  const channel = new Channel<{ done: number; total: number }>();
  channel.onmessage = (p) => onProgress(p.done, p.total);
  return invoke<SimilarResult>("find_similar", { id, minSize, maxDistance, onEvent: channel });
}

export const cancelSimilar = () => invoke<void>("cancel_similar");

/** 图片缩略图的 blob URL，用完后应调用 `URL.revokeObjectURL`。 */
export async function thumbnailUrl(id: number, max: number): Promise<string> {
  const buf = await invoke<ArrayBuffer>("thumbnail", { id, max });
  return URL.createObjectURL(new Blob([buf], { type: "image/png" }));
}

/** 一条清理建议（一条规则在当前结果中匹配到的位置）。 */
export interface Suggestion {
  id: string;
  name: string;
  category: string;
  risk: "low" | "medium" | "high";
  /** 默认勾选。 */
  default: boolean;
  description: string;
  effect: string;
  /** `guide` 只给处理方法，不能直接清理。 */
  action: "contents" | "self" | "guide";
  guide: string | null;
  size: number;
  count: number;
  locations: { id: number; path: string; size: number }[];
}

export const cleanSuggestions = () => invoke<Suggestion[]>("clean_suggestions");

/** 按规则清理前的汇总，格式同删除。 */
export const planClean = (ruleIds: string[]) => invoke<DeletePlan>("plan_clean", { ruleIds });

export const cleanRules = (ruleIds: string[], permanent: boolean) =>
  invoke<DeleteResult>("clean_rules", { ruleIds, permanent });

/** 缓存的扫描结果，最近的在前。 */
export const cachedScans = () => invoke<CachedScan[]>("cached_scans");

/** 打开某个路径上次的扫描结果。 */
export const openCached = (root: string) => invoke<ScanSummary>("open_cached", { root });

export const nodeInfo = (id: number) => invoke<NodeInfo>("node_info", { id });

export const listChildren = (id: number, filter: string | null) =>
  invoke<ChildList>("list_children", { id, filter });

export const typeStats = (id: number) => invoke<TypeStat[]>("type_stats", { id });

export const search = (id: number, query: string, limit: number) =>
  invoke<SearchHit[]>("search", { id, query, limit });

export interface SunburstSegment {
  id: number;
  category: CategoryKey;
  isDir: boolean;
  /** 第几圈，从 1 开始。 */
  depth: number;
  /** 起止角度，圆周的比例（0..1，从 12 点方向顺时针）。 */
  start: number;
  end: number;
}

/**
 * 每段 20 字节：id u32、标志 u32（低 8 位类型下标，第 8 位表示目录）、圈 u32、起止角度 f32，小端。
 * 见 Rust 侧 `commands::sunburst`。
 */
const SEGMENT_BYTES = 20;

export async function sunburst(id: number): Promise<SunburstSegment[]> {
  const buf = await invoke<ArrayBuffer>("sunburst", { id });
  const view = new DataView(buf);
  const segments: SunburstSegment[] = [];
  for (let off = 0; off + SEGMENT_BYTES <= buf.byteLength; off += SEGMENT_BYTES) {
    const flags = view.getUint32(off + 4, true);
    segments.push({
      id: view.getUint32(off, true),
      category: CATEGORY_KEYS[flags & 0xff] ?? "other",
      isDir: (flags & 0x100) !== 0,
      depth: view.getUint32(off + 8, true),
      start: view.getFloat32(off + 12, true),
      end: view.getFloat32(off + 16, true),
    });
  }
  return segments;
}

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
