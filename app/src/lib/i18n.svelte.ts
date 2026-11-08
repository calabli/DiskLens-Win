import { invoke } from "@tauri-apps/api/core";
import en from "./locales/en.json";
import zhTW from "./locales/zh-TW.json";

/**
 * 界面语言。简体中文原文就是翻译的键：`t("清理建议")` 在简体中文下原样返回，
 * 在其他语言下查 `locales/*.json`，查不到时退回原文。同一份词典也被 Rust 侧
 * （dl-i18n）用来翻译错误提示和清理规则。
 */
export type Locale = "zh-CN" | "zh-TW" | "en";
/** 语言设置：跟随系统，或固定某种语言。 */
export type LocaleMode = "system" | Locale;

const KEY = "disklens.locale";
const DICTS: Record<Exclude<Locale, "zh-CN">, Record<string, string>> = { en, "zh-TW": zhTW };

/** 系统语言对应的界面语言：繁体中文地区用繁体，其他中文用简体，其余用英文。 */
function systemLocale(): Locale {
  const langs = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const raw of langs) {
    const l = raw.toLowerCase();
    if (/^zh-(tw|hk|mo)|^zh-hant/.test(l)) return "zh-TW";
    if (l.startsWith("zh")) return "zh-CN";
    if (l.startsWith("en")) return "en";
  }
  return "en";
}

function load(): LocaleMode {
  try {
    const saved = localStorage.getItem(KEY);
    return saved === "zh-CN" || saved === "zh-TW" || saved === "en" ? saved : "system";
  } catch {
    return "system";
  }
}

const state = $state<{ mode: LocaleMode; locale: Locale }>({ mode: "system", locale: "zh-CN" });

/** 把语言告诉页面和 Rust 侧。 */
function apply() {
  state.locale = state.mode === "system" ? systemLocale() : state.mode;
  document.documentElement.lang = state.locale;
  invoke("set_locale", { locale: state.locale }).catch(() => {});
}

/** 启动时调用一次：读取上次的选择。 */
export function initLocale() {
  state.mode = load();
  apply();
}

export function localeMode(): LocaleMode {
  return state.mode;
}

/** 当前界面语言；也用于日期、数字的格式。 */
export function locale(): Locale {
  return state.locale;
}

export function setLocaleMode(mode: LocaleMode) {
  state.mode = mode;
  try {
    localStorage.setItem(KEY, mode);
  } catch {
    // 存不下也不影响这次使用。
  }
  apply();
}

/**
 * 标记一个翻译键，原样返回。用于先存在常量里、显示时再 `t(…)` 的文字，
 * 让检查脚本知道它需要翻译。
 */
export const tk = (key: string) => key;

/**
 * 翻译 `key`（简体中文原文），把其中的 `{name}` 换成 `params` 里的值。
 * 在模板里调用时会随语言切换自动刷新。
 */
export function t(key: string, params?: Record<string, string | number>): string {
  const dict = state.locale === "zh-CN" ? null : DICTS[state.locale];
  const text = dict?.[key] ?? key;
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (m, name: string) =>
    name in params ? String(params[name]) : m,
  );
}
