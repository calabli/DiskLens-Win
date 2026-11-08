//! 界面语言：Rust 侧给用户看的文字（错误提示、清理规则等）按当前语言翻译。
//!
//! 简体中文原文就是翻译的键，与前端共用 `app/src/lib/locales/*.json` 两份词典：
//! 简体中文下原样返回，其他语言查词典，查不到时退回原文。语言由界面在启动和切换时设置。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// 界面语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    ZhCn,
    ZhTw,
    En,
}

static LOCALE: AtomicU8 = AtomicU8::new(0);

const EN: &str = include_str!("../../../app/src/lib/locales/en.json");
const ZH_TW: &str = include_str!("../../../app/src/lib/locales/zh-TW.json");

/// 设置界面语言：`zh-CN`、`zh-TW`、`en`，其他值当作简体中文。
pub fn set_locale(tag: &str) {
    let l = match tag {
        "en" => Locale::En,
        "zh-TW" => Locale::ZhTw,
        _ => Locale::ZhCn,
    };
    LOCALE.store(l as u8, Ordering::Relaxed);
}

pub fn locale() -> Locale {
    match LOCALE.load(Ordering::Relaxed) {
        1 => Locale::ZhTw,
        2 => Locale::En,
        _ => Locale::ZhCn,
    }
}

fn dict(l: Locale) -> Option<&'static HashMap<String, String>> {
    static EN_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    static TW_MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    let parse = |json: &str| serde_json::from_str(json).unwrap_or_default();
    match l {
        Locale::ZhCn => None,
        Locale::En => Some(EN_MAP.get_or_init(|| parse(EN))),
        Locale::ZhTw => Some(TW_MAP.get_or_init(|| parse(ZH_TW))),
    }
}

/// 翻译固定的一句话（简体中文原文）。
pub fn tr(key: &'static str) -> &'static str {
    dict(locale())
        .and_then(|d| d.get(key))
        .map_or(key, String::as_str)
}

/// 标记一个翻译键，原样返回：用在常量里、由界面显示时再翻译的文字，让检查脚本知道它需要翻译。
pub const fn tk(key: &'static str) -> &'static str {
    key
}

/// 翻译运行时才知道的文字（例如清理规则的名称）。
pub fn tr_owned(key: &str) -> String {
    dict(locale())
        .and_then(|d| d.get(key))
        .map_or_else(|| key.to_owned(), Clone::clone)
}

/// 翻译带占位符的一句话：`trf!("无法扫描 {path}：{e}", path = p, e = err)`。
/// 占位符写在原文和译文里，按名称替换，译文可以调整它们的顺序。
#[macro_export]
macro_rules! trf {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        #[allow(unused_mut)]
        let mut s = $crate::tr($key).to_owned();
        $( s = s.replace(concat!("{", stringify!($name), "}"), &($value).to_string()); )*
        s
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictionaries_parse() {
        // 两份词典都必须是合法的 JSON 对象，否则翻译会悄悄失效。
        for json in [EN, ZH_TW] {
            let m: HashMap<String, String> = serde_json::from_str(json).expect("valid dictionary");
            assert!(m.values().all(|v| !v.is_empty()));
        }
    }

    #[test]
    fn falls_back_and_fills_placeholders() {
        set_locale("zh-CN");
        assert_eq!(tr("这句话不在词典里"), "这句话不在词典里");
        assert_eq!(
            trf!("共 {n} 项，{size}", n = 3, size = "1 GB"),
            "共 3 项，1 GB"
        );
        set_locale("en");
        assert_eq!(tr("这句话不在词典里"), "这句话不在词典里");
        set_locale("zh-CN");
    }
}
