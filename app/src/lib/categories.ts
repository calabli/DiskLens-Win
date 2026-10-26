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
 * 同一类别的相邻方块轮流使用这几档深浅，避免连成一片。
 * 正数向白色混合，负数向黑色混合。
 */
const SHADES = [0, 0.2, -0.12, 0.34];

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
    /** 每个类别的几档深浅，按方块序号轮流取用。 */
    fill: Object.fromEntries(
      CATEGORY_KEYS.map((k) => {
        const base = read(`--cat-${k}`);
        return [k, SHADES.map((a) => shade(base, a))];
      }),
    ) as Record<CategoryKey, string[]>,
    ink: read("--tile-ink"),
    gap: read("--tile-gap"),
    accent: read("--accent"),
  };
}

export type Palette = ReturnType<typeof resolvePalette>;
