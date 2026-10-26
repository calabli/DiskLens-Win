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

/** Canvas 不认 CSS 变量，绘制前把它们解析成具体颜色。 */
export function resolvePalette(el: Element) {
  const style = getComputedStyle(el);
  const read = (name: string) => style.getPropertyValue(name).trim();
  return {
    fill: Object.fromEntries(CATEGORY_KEYS.map((k) => [k, read(`--cat-${k}`)])) as Record<
      CategoryKey,
      string
    >,
    ink: read("--tile-ink"),
    gap: read("--tile-gap"),
    accent: read("--accent"),
  };
}

export type Palette = ReturnType<typeof resolvePalette>;
