// 检查翻译是否完整：
// 1. 前端（.svelte / .ts）和 Rust 代码里给用户看的中文都要包在 t() / tr() / trf!() 里；
// 2. 所有翻译键（加上清理规则的名称、说明等）在 en.json 和 zh-TW.json 里都要有；
// 3. 词典里没人用的条目给出提示。
// 不需要翻译的行在行尾写 `i18n-ignore`（例如搜索语法里的中文别名）；
// 成段的写在 `i18n-ignore-start` 和 `i18n-ignore-end` 两行注释之间。
//
// 用法：node scripts/check-i18n.mjs [--missing]
//   --missing  把缺少的键按 JSON 打印出来，方便补词典

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const app = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = path.resolve(app, "..");
const HAN = /[\u4e00-\u9fff]/;

function walk(dir, exts, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) {
      // examples 是开发用的命令行工具，tests 是测试，都不给用户看。
      if (
        !["node_modules", "target", "locales", ".svelte-kit", "examples", "tests"].includes(e.name)
      )
        walk(p, exts, out);
    } else if (exts.some((x) => e.name.endsWith(x))) out.push(p);
  }
  return out;
}

/** 把字符串里的转义还原成实际文字，和运行时的键一致。 */
function unescape(s) {
  return s.replace(/\\(["'\\nrt])/g, (_, c) => ({ n: "\n", r: "\r", t: "\t" })[c] ?? c);
}

const keys = new Map(); // 键 -> 第一次出现的位置
const problems = [];

function addKey(k, where) {
  if (!keys.has(k)) keys.set(k, where);
}

/** 去掉注释，保留行号（注释内容替换成空格）。 */
function blank(text, re) {
  return text.replace(re, (m) => m.replace(/[^\n]/g, " "));
}

function scan(file, { calls, stripTests }) {
  let text = fs.readFileSync(file, "utf8");
  const original = text.split(String.fromCharCode(10));
  const rel = path.relative(repo, file).replaceAll("\\", "/");
  if (file.endsWith(".svelte")) {
    text = blank(text, /<style[\s\S]*?<\/style>/g);
    text = blank(text, /<!--[\s\S]*?-->/g);
  }
  if (stripTests) {
    const i = text.search(/#\[cfg\(test\)\]\s*mod tests/);
    if (i >= 0) text = text.slice(0, i);
  }
  text = blank(text, /\/\*[\s\S]*?\*\//g);
  text = blank(text, /(^|[^:"'\w])\/\/[^\n]*/gm);

  // 收集翻译调用里的键，并把它们从文本里抹掉。
  const callRe = new RegExp(`\\b(?:${calls})\\(\\s*"((?:[^"\\\\]|\\\\.)*)"`, "g");
  text = text.replace(callRe, (m, k, offset) => {
    const line = text.slice(0, offset).split("\n").length;
    addKey(unescape(k), `${rel}:${line}`);
    return m.replace(/[^\n]/g, " ");
  });

  let skipping = false;
  text.split("\n").forEach((l, i) => {
    if (original[i].includes("i18n-ignore-start")) skipping = true;
    if (original[i].includes("i18n-ignore-end")) skipping = false;
    if (HAN.test(l) && !skipping && !original[i].includes("i18n-ignore")) {
      problems.push(`${rel}:${i + 1}: 未翻译：${l.trim().slice(0, 100)}`);
    }
  });
}

for (const f of walk(path.join(app, "src"), [".svelte", ".ts"])) {
  scan(f, { calls: "t|tk", stripTests: false });
}
for (const f of [
  ...walk(path.join(app, "src-tauri", "src"), [".rs"]),
  ...walk(path.join(repo, "crates"), [".rs"]),
]) {
  scan(f, { calls: "tr|trf!|tr_owned|tk", stripTests: true });
}

// 清理规则里给用户看的文字。
const rules = fs.readFileSync(path.join(repo, "rules", "default.yaml"), "utf8");
for (const m of rules.matchAll(/^\s+(name|description|effect|guide): (.+)$/gm)) {
  addKey(m[2].trim().replace(/^'(.*)'$/, "$1"), "rules/default.yaml");
}

const dicts = Object.fromEntries(
  ["en", "zh-TW"].map((l) => [
    l,
    JSON.parse(fs.readFileSync(path.join(app, "src", "lib", "locales", `${l}.json`), "utf8")),
  ]),
);
const missing = {};
for (const [l, d] of Object.entries(dicts)) {
  const miss = [...keys.keys()].filter((k) => !(k in d));
  missing[l] = miss;
  for (const k of miss) problems.push(`${l}.json 缺少：${JSON.stringify(k)}（${keys.get(k)}）`);
  for (const k of Object.keys(d))
    if (!keys.has(k)) console.warn(`提示：${l}.json 里没人用的条目：${JSON.stringify(k)}`);
}

if (process.argv.includes("--missing")) {
  console.log(
    JSON.stringify(
      Object.fromEntries([...new Set(Object.values(missing).flat())].map((k) => [k, ""])),
      null,
      2,
    ),
  );
  process.exit(0);
}
if (problems.length) {
  console.error(problems.join("\n"));
  console.error(`\n共 ${problems.length} 处问题，翻译键 ${keys.size} 个。`);
  process.exit(1);
}
console.log(`翻译完整：${keys.size} 个键，en 和 zh-TW 都齐全。`);
