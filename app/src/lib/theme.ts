import { getCurrentWindow } from "@tauri-apps/api/window";

/** 外观：跟随系统，或固定浅色、深色。 */
export type ThemeMode = "system" | "light" | "dark";

const KEY = "disklens.theme";

let mode: ThemeMode = "system";
let media: MediaQueryList | null = null;

function load(): ThemeMode {
  try {
    const saved = localStorage.getItem(KEY);
    return saved === "light" || saved === "dark" ? saved : "system";
  } catch {
    return "system";
  }
}

/**
 * 把外观落到页面上：`<html data-theme>` 总是具体的 light / dark，CSS 只看它。
 * 窗口标题栏也跟着切换；跟随系统时交还给系统决定。
 */
function apply() {
  const dark = mode === "dark" || (mode === "system" && !!media?.matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
  getCurrentWindow()
    .setTheme(mode === "system" ? null : mode)
    .catch(() => {});
}

/** 启动时调用一次：读取上次的选择，并在跟随系统时响应系统切换。 */
export function initTheme() {
  mode = load();
  media = window.matchMedia("(prefers-color-scheme: dark)");
  media.addEventListener("change", () => {
    if (mode === "system") apply();
  });
  apply();
}

export function themeMode(): ThemeMode {
  return mode;
}

export function setThemeMode(next: ThemeMode) {
  mode = next;
  try {
    localStorage.setItem(KEY, next);
  } catch {
    // 存不下也不影响这次使用。
  }
  apply();
}

/** 页面外观变化时调用 `fn`（Canvas 要重新读取配色）；返回取消监听的函数。 */
export function onThemeChange(fn: () => void): () => void {
  const observer = new MutationObserver(fn);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => observer.disconnect();
}
