//! 清理规则：从 YAML 读入规则，在扫描结果里找出每条规则匹配到的位置。
//!
//! 规则的写法见 `rules/default.yaml` 开头的说明。匹配只看扫描结果，不访问磁盘。
//! 读入时会检查规则是否自洽：高风险的只能给指引、默认勾选的必须是低风险等，
//! 保证“高风险项无法直接删除”。

use std::collections::{HashMap, HashSet};

use dl_tree::{FileTree, NodeId, NodeKind};
use serde::Deserialize;

/// 内置规则。
pub const BUILTIN: &str = include_str!("../../../rules/default.yaml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// 清空目录里的内容，目录本身保留。
    Contents,
    /// 删除匹配到的目录或文件本身。
    #[serde(rename = "self")]
    Remove,
    /// 不删除，只给出处理方法。
    Guide,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub category: String,
    pub risk: Risk,
    #[serde(default)]
    pub default: bool,
    pub description: String,
    pub effect: String,
    pub action: Action,
    #[serde(default)]
    pub guide: Option<String>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub dir_names: Vec<String>,
    #[serde(default)]
    pub requires: Vec<String>,
}

/// 读入并检查规则。
pub fn parse(yaml: &str) -> Result<Vec<Rule>, String> {
    let rules: Vec<Rule> = serde_yaml::from_str(yaml).map_err(|e| format!("规则格式不对：{e}"))?;
    let mut ids = HashSet::new();
    for r in &rules {
        let bad = |msg: &str| Err(format!("规则 {}：{msg}", r.id));
        if !ids.insert(r.id.as_str()) {
            return bad("id 重复");
        }
        if r.risk == Risk::High && r.action != Action::Guide {
            return bad("高风险规则只能给出指引（action: guide）");
        }
        if r.default && (r.risk != Risk::Low || r.action == Action::Guide) {
            return bad("只有可以直接清理的低风险规则才能默认勾选");
        }
        if (r.action == Action::Guide) != r.guide.is_some() {
            return bad("action 为 guide 时必须、且只有这时才写 guide");
        }
        if r.paths.is_empty() == r.dir_names.is_empty() {
            return bad("paths 和 dir_names 必须有且只有一个");
        }
        if !r.files.is_empty() && r.paths.is_empty() {
            return bad("files 要和 paths 一起用");
        }
        if !r.requires.is_empty() && r.dir_names.is_empty() {
            return bad("requires 要和 dir_names 一起用");
        }
        if r.action == Action::Contents && !r.files.is_empty() {
            return bad("按文件名匹配时不能用 contents");
        }
    }
    Ok(rules)
}

/// 内置规则。内置文件由测试保证格式正确。
pub fn builtin() -> Vec<Rule> {
    parse(BUILTIN).expect("built-in rules are valid")
}

/// 一条规则在扫描结果里匹配到的位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Matched {
    /// 在规则列表里的下标。
    pub rule: usize,
    /// 匹配到的目录或文件，互不包含。
    pub nodes: Vec<NodeId>,
    pub size: u64,
}

/// 找出每条规则匹配到的位置，只返回大小不为 0 的。`env` 用来展开 `%变量%`。
pub fn match_rules(
    rules: &[Rule],
    tree: &FileTree,
    env: &dyn Fn(&str) -> Option<String>,
) -> Vec<Matched> {
    let mut found: Vec<Vec<NodeId>> = vec![Vec::new(); rules.len()];
    for (i, rule) in rules.iter().enumerate() {
        for pattern in &rule.paths {
            let Some(path) = expand(pattern, env) else {
                continue;
            };
            for node in resolve(tree, &path) {
                if rule.files.is_empty() {
                    found[i].push(node);
                } else if tree.kind(node) == NodeKind::Dir {
                    found[i].extend(tree.children(node).filter(|&c| {
                        tree.kind(c) == NodeKind::File
                            && rule
                                .files
                                .iter()
                                .any(|g| glob(&lower(g), &lower(tree.name(c))))
                    }));
                }
            }
        }
    }
    match_dir_names(rules, tree, &mut found);

    found
        .into_iter()
        .enumerate()
        .filter_map(|(rule, nodes)| {
            let nodes = outermost(tree, nodes);
            let size = nodes.iter().map(|&n| tree.size(n)).sum();
            (size > 0).then_some(Matched { rule, nodes, size })
        })
        .collect()
}

/// 执行时实际要删除的项：`contents` 是各目录的子项，`self` 是匹配项本身，`guide` 为空。
pub fn targets(rule: &Rule, tree: &FileTree, nodes: &[NodeId]) -> Vec<NodeId> {
    match rule.action {
        Action::Contents => nodes
            .iter()
            .filter(|&&n| tree.kind(n) == NodeKind::Dir)
            .flat_map(|&n| tree.children(n))
            .collect(),
        Action::Remove => nodes.to_vec(),
        Action::Guide => Vec::new(),
    }
}

/// 一次遍历扫描范围，处理所有按目录名匹配的规则。匹配到的目录不再往里找。
fn match_dir_names(rules: &[Rule], tree: &FileTree, found: &mut [Vec<NodeId>]) {
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, r) in rules.iter().enumerate() {
        for name in &r.dir_names {
            by_name.entry(lower(name)).or_default().push(i);
        }
    }
    if by_name.is_empty() {
        return;
    }
    let mut stack = vec![tree.root()];
    while let Some(dir) = stack.pop() {
        for child in tree.children(dir) {
            if tree.kind(child) != NodeKind::Dir {
                continue;
            }
            let hit = by_name.get(&lower(tree.name(child))).and_then(|rs| {
                rs.iter().copied().find(|&i| {
                    let req = &rules[i].requires;
                    req.is_empty()
                        || tree.children(dir).any(|s| {
                            tree.kind(s) == NodeKind::File
                                && req.iter().any(|g| glob(&lower(g), &lower(tree.name(s))))
                        })
                })
            });
            match hit {
                Some(i) => found[i].push(child),
                None => stack.push(child),
            }
        }
    }
}

/// 去重，并去掉被其他匹配项包含的项，避免重复计算。
fn outermost(tree: &FileTree, mut nodes: Vec<NodeId>) -> Vec<NodeId> {
    nodes.sort_unstable();
    nodes.dedup();
    let set: HashSet<NodeId> = nodes.iter().copied().collect();
    nodes.retain(|&n| {
        let mut cur = tree.parent(n);
        while let Some(p) = cur {
            if set.contains(&p) {
                return false;
            }
            cur = tree.parent(p);
        }
        true
    });
    nodes
}

/// 展开 `%变量%`。用到的变量不存在时返回 `None`（这一条路径跳过）。
pub fn expand(pattern: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('%')?;
        out.push_str(env(&after[..end])?.trim_end_matches('\\'));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// 在扫描结果里找出与 `path` 对应的节点；`path` 的某一级可以含 `*`、`?`。
/// 不在扫描范围内时返回空。
fn resolve(tree: &FileTree, path: &str) -> Vec<NodeId> {
    let root = lower(tree.name(tree.root()));
    let root = root.trim_end_matches('\\');
    let want = lower(path.trim_end_matches('\\'));
    let rest = if want == root {
        ""
    } else if let Some(r) = want.strip_prefix(root).and_then(|r| r.strip_prefix('\\')) {
        r
    } else {
        return Vec::new();
    };

    let mut current = vec![tree.root()];
    for part in rest.split('\\').filter(|p| !p.is_empty()) {
        let wild = part.contains(['*', '?']);
        current = current
            .into_iter()
            .flat_map(|n| tree.children(n))
            .filter(|&c| {
                let name = lower(tree.name(c));
                if wild {
                    glob(part, &name)
                } else {
                    name == part
                }
            })
            .collect();
        if current.is_empty() {
            break;
        }
    }
    current
}

fn lower(s: &str) -> String {
    s.to_lowercase()
}

/// 通配符匹配：`*` 匹配任意多个字符，`?` 匹配一个字符。调用方负责统一大小写。
fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_rules_are_valid_and_cover_the_spec() {
        let rules = builtin();
        let ids: HashSet<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        // 功能说明书 F11 表中的每一类，以及 F10 列出的开发者大户。
        for id in [
            "temp-user",
            "temp-windows",
            "browser-chrome",
            "browser-edge",
            "browser-firefox",
            "update-download",
            "delivery-optimization",
            "crash-dumps",
            "error-reports",
            "thumbnail-cache",
            "windows-old",
            "component-store",
            "hibernation-file",
            "page-file",
            "node-modules",
            "gradle-cache",
            "maven-repository",
            "package-caches",
            "conda-pkgs",
            "huggingface-cache",
            "ollama-models",
            "virtual-disks",
            "dotnet-obj-bin",
        ] {
            assert!(ids.contains(id), "missing rule {id}");
        }
        assert!(
            rules
                .iter()
                .filter(|r| r.risk == Risk::High)
                .all(|r| r.action == Action::Guide)
        );
        assert!(
            rules
                .iter()
                .filter(|r| r.default)
                .all(|r| r.risk == Risk::Low)
        );
    }

    #[test]
    fn rejects_unsafe_rules() {
        let base = "- {id: x, name: n, category: c, description: d, effect: e";
        let check = |extra: &str| parse(&format!("{base}, {extra}}}"));
        assert!(check("risk: high, action: self, paths: ['C:\\x']").is_err());
        assert!(check("risk: medium, default: true, action: self, paths: ['C:\\x']").is_err());
        assert!(check("risk: low, action: guide, paths: ['C:\\x']").is_err());
        assert!(check("risk: low, action: self").is_err());
        assert!(check("risk: low, action: self, paths: ['C:\\x'], typo: 1").is_err());
        assert!(check("risk: low, action: self, paths: ['C:\\x']").is_ok());
    }

    fn env(name: &str) -> Option<String> {
        match name {
            "LOCALAPPDATA" => Some(r"C:\Users\me\AppData\Local".into()),
            "TEMP" => Some(r"C:\Users\me\AppData\Local\Temp\".into()),
            "SystemRoot" => Some(r"C:\WINDOWS".into()),
            _ => None,
        }
    }

    /// 一棵模拟的 C 盘。
    fn tree() -> FileTree {
        let mut t = FileTree::new(r"C:\");
        let users = t.add_dir(0, "Users");
        let me = t.add_dir(users, "me");
        let local = {
            let a = t.add_dir(me, "AppData");
            t.add_dir(a, "Local")
        };
        let temp = t.add_dir(local, "Temp");
        t.add_file(temp, "a.tmp", 100, 0);
        let sub = t.add_dir(temp, "setup");
        t.add_file(sub, "b.tmp", 50, 0);
        let chrome = {
            let g = t.add_dir(local, "Google");
            let c = t.add_dir(g, "Chrome");
            t.add_dir(c, "User Data")
        };
        for profile in ["Default", "Profile 1"] {
            let p = t.add_dir(chrome, profile);
            let cache = t.add_dir(p, "Cache");
            t.add_file(cache, "data_1", 1_000, 0);
        }
        let explorer = {
            let m = t.add_dir(local, "Microsoft");
            let w = t.add_dir(m, "Windows");
            t.add_dir(w, "Explorer")
        };
        t.add_file(explorer, "thumbcache_256.db", 300, 0);
        t.add_file(explorer, "iconcache_16.db", 30, 0);
        let windows = t.add_dir(0, "Windows");
        let wt = t.add_dir(windows, "Temp");
        t.add_file(wt, "x.log", 7, 0);
        // 两个 Node 项目：一个是真项目，另一个只是叫 node_modules 的普通目录。
        let proj = t.add_dir(0, "proj");
        t.add_file(proj, "package.json", 1, 0);
        let nm = t.add_dir(proj, "node_modules");
        let inner = t.add_dir(nm, "dep");
        let nested = t.add_dir(inner, "node_modules");
        t.add_file(nested, "big.js", 5_000, 0);
        t.add_file(inner, "package.json", 1, 0);
        let other = t.add_dir(0, "other");
        let fake = t.add_dir(other, "NODE_MODULES");
        t.add_file(fake, "keep.txt", 9, 0);
        t.finalize();
        t
    }

    fn rule(yaml: &str) -> Rule {
        let base = "- {id: x, name: n, category: c, description: d, effect: e, risk: low";
        parse(&format!("{base}, {yaml}}}")).unwrap().remove(0)
    }

    fn names(t: &FileTree, nodes: &[NodeId]) -> Vec<String> {
        let mut v: Vec<String> = nodes.iter().map(|&n| t.path(n)).collect();
        v.sort();
        v
    }

    #[test]
    fn matches_paths_wildcards_and_file_patterns() {
        let t = tree();
        let rules = vec![
            rule(r"action: contents, paths: ['%TEMP%', '%SystemRoot%\Temp']"),
            rule(r"action: contents, paths: ['%LOCALAPPDATA%\Google\Chrome\User Data\*\Cache']"),
            rule(
                r"action: self, paths: ['%LOCALAPPDATA%\Microsoft\Windows\Explorer'], files: ['thumbcache_*.db']",
            ),
            rule(r"action: contents, paths: ['%NOT_SET%\x', 'D:\elsewhere']"),
        ];
        let m = match_rules(&rules, &t, &env);
        assert_eq!(m.len(), 3, "{m:?}");
        assert_eq!(m[0].size, 157);
        assert_eq!(
            names(&t, &m[0].nodes),
            [r"C:\Users\me\AppData\Local\Temp", r"C:\Windows\Temp"],
            "case-insensitive (WINDOWS vs Windows)"
        );
        assert_eq!(m[1].nodes.len(), 2);
        assert_eq!(m[1].size, 2_000);
        assert_eq!(
            names(&t, &m[2].nodes),
            [r"C:\Users\me\AppData\Local\Microsoft\Windows\Explorer\thumbcache_256.db"]
        );

        // contents：删除的是目录里的东西。
        let targets = targets(&rules[0], &t, &m[0].nodes);
        assert_eq!(targets.len(), 3);
        assert!(targets.iter().all(|&n| !m[0].nodes.contains(&n)));
    }

    #[test]
    fn matches_project_folders_by_name_with_requirements() {
        let t = tree();
        let rules = vec![rule(
            "action: self, dir_names: [node_modules], requires: [package.json]",
        )];
        let m = match_rules(&rules, &t, &env);
        // 只有 proj 下的那个；嵌套的不重复计算；other 下没有 package.json，不算。
        assert_eq!(names(&t, &m[0].nodes), [r"C:\proj\node_modules"]);
        assert_eq!(m[0].size, 5_001);
    }

    #[test]
    fn scan_of_a_subfolder_only_matches_inside_it() {
        let mut t = FileTree::new(r"C:\Users\me\AppData\Local");
        let temp = t.add_dir(0, "Temp");
        t.add_file(temp, "a", 5, 0);
        t.finalize();
        let rules = vec![
            rule("action: contents, paths: ['%TEMP%']"),
            rule(r"action: contents, paths: ['%SystemRoot%\Temp']"),
        ];
        let m = match_rules(&rules, &t, &env);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].rule, 0);
    }

    #[test]
    fn globbing() {
        assert!(glob("thumbcache_*.db", "thumbcache_256.db"));
        assert!(!glob("thumbcache_*.db", "iconcache_16.db"));
        assert!(glob("*.csproj", "app.csproj"));
        assert!(glob("a?c*", "abcdef"));
        assert!(!glob("a?c", "ac"));
        assert!(glob("*", ""));
        assert_eq!(
            expand(r"%SystemRoot%\Temp", &env).as_deref(),
            Some(r"C:\WINDOWS\Temp")
        );
        assert_eq!(
            expand(r"%TEMP%\x", &env).as_deref(),
            Some(r"C:\Users\me\AppData\Local\Temp\x")
        );
        assert_eq!(expand(r"%MISSING%\x", &env), None);
    }
}
