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
  /** 填充色。 */
  color: string;
  /** 在填充色上写字用的颜色，保证对比度。 */
  ink: string;
}

export const CATEGORIES: Record<CategoryKey, CategoryStyle> = {
  system: { label: "系统文件", color: "#8a94a6", ink: "#141413" },
  app: { label: "应用程序", color: "#3a6ac8", ink: "#ffffff" },
  dev: { label: "开发缓存", color: "#e08a3c", ink: "#141413" },
  media: { label: "音视频", color: "#6a4bb8", ink: "#ffffff" },
  package: { label: "安装包与镜像", color: "#d9b54a", ink: "#141413" },
  document: { label: "文档与图片", color: "#277a68", ink: "#ffffff" },
  other: { label: "其他", color: "#c9c4b8", ink: "#141413" },
};
