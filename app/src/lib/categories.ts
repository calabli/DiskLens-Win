import type { TreemapRect } from "$lib/api";

/** 与 Rust 侧 `dl_analyze::Category::ALL` 的顺序一致，树图二进制数据里存的是这里的下标。 */
export const CATEGORY_KEYS = [
  "system",
  "app",
  "dev",
  "media",
  "package",
  "document",
  "other",
] as const;

export type CategoryKey = (typeof CATEGORY_KEYS)[number];

interface CategoryStyle {
  label: string;
  /** CSS 颜色值（引用 app.css 里的变量，随深浅色切换）。 */
  color: string;
}

const LABELS: Record<CategoryKey, string> = {
  system: "系统文件",
  app: "应用程序",
  dev: "开发缓存",
  media: "音视频",
  package: "安装包与镜像",
  document: "文档与图片",
  other: "其他",
};

export const CATEGORIES = Object.fromEntries(
  CATEGORY_KEYS.map((k) => [k, { label: LABELS[k], color: `var(--cat-${k})` }]),
) as Record<CategoryKey, CategoryStyle>;

/**
 * 同一类别的方块用这几档深浅区分，相邻的两块一定不同档（见 assignShades）。
 * 正数向白色混合，负数向黑色混合；变暗不超过 0.12，保证深色文字依然清楚。
 * 前三档差别最大，绝大多数方块只会用到它们。
 */
const SHADES = [0, 0.34, -0.12, 0.17, 0.5, -0.06];

/** 把 `#rrggbb` 向白色（amount > 0）或黑色（amount < 0）混合。其他写法原样返回。 */
function shade(color: string, amount: number): string {
  const m = /^#([0-9a-f]{6})$/i.exec(color);
  if (!m || amount === 0) return color;
  const n = parseInt(m[1], 16);
  const target = amount > 0 ? 255 : 0;
  const t = Math.abs(amount);
  const mix = (c: number) => Math.round(c + (target - c) * t);
  return `rgb(${mix(n >> 16)} ${mix((n >> 8) & 255)} ${mix(n & 255)})`;
}

/** Canvas 不认 CSS 变量，绘制前把它们解析成具体颜色。 */
export function resolvePalette(el: Element) {
  const style = getComputedStyle(el);
  const read = (name: string) => style.getPropertyValue(name).trim();
  return {
    /** 每个类别的几档深浅，下标由 assignShades 给出。 */
    fill: Object.fromEntries(
      CATEGORY_KEYS.map((k) => {
        const base = read(`--cat-${k}`);
        return [k, SHADES.map((a) => shade(base, a))];
      }),
    ) as Record<CategoryKey, string[]>,
    ink: read("--tile-ink"),
    muted: read("--tile-muted"),
    gap: read("--tile-gap"),
    accent: read("--accent"),
  };
}

export type Palette = ReturnType<typeof resolvePalette>;

/**
 * 给每个方块挑一档深浅：与它相邻（共用一段边）或包住它的同类方块都不同档。
 * 贪心着色，按顺序取第一档没被占用的；邻居太多用完时退回按序号轮换。
 */
export function assignShades(list: readonly TreemapRect[]): Uint8Array {
  const n = list.length;
  const eps = 0.75;
  const neighbors: number[][] = Array.from({ length: n }, () => []);
  const order = [...list.keys()].sort((a, b) => list[a].x - list[b].x);
  for (let p = 0; p < n; p++) {
    const i = order[p];
    const a = list[i];
    for (let q = p + 1; q < n; q++) {
      const j = order[q];
      const b = list[j];
      if (b.x > a.x + a.w + eps) break;
      if (a.category !== b.category) continue;
      const ox = Math.min(a.x + a.w, b.x + b.w) - Math.max(a.x, b.x);
      const oy = Math.min(a.y + a.h, b.y + b.h) - Math.max(a.y, b.y);
      // 共用一段边，或者一块叠在另一块上面（嵌套）。
      if ((ox > eps && oy > -eps) || (oy > eps && ox > -eps)) {
        neighbors[i].push(j);
        neighbors[j].push(i);
      }
    }
  }
  const out = new Uint8Array(n).fill(255);
  for (let i = 0; i < n; i++) {
    let used = 0;
    for (const j of neighbors[i]) if (out[j] !== 255) used |= 1 << out[j];
    let k = 0;
    while (k < SHADES.length && used & (1 << k)) k++;
    out[i] = k < SHADES.length ? k : i % SHADES.length;
  }
  return out;
}

/**
 * 搜索或筛选时哪些方块保持原色：本身命中（标记 2）的方块连同它里面嵌套的方块，
 * 以及里面有命中项（标记 1）的方块本身。`list` 按深度优先排列，嵌套的块紧跟在所属块后面。
 */
export function litTiles(
  list: readonly { id: number; level: number }[],
  marks: ReadonlyMap<number, number>,
): Uint8Array {
  const out = new Uint8Array(list.length);
  /** whole[level]：这一层最近的块是否整块命中。 */
  const whole: boolean[] = [];
  list.forEach((r, i) => {
    const all = (r.level > 1 && whole[r.level - 1]) || marks.get(r.id) === 2;
    whole[r.level] = all;
    out[i] = all || marks.get(r.id) === 1 ? 1 : 0;
  });
  return out;
}
