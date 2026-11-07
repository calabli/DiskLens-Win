//! 开发项目：找出扫描范围里的开发项目，以及每个项目可以删掉、之后能重新生成的构建产物
//! （依赖包、编译输出、虚拟环境等）。
//!
//! 一个文件夹里直接有某种项目的标记文件（`package.json`、`Cargo.toml` 等）就算一个项目。
//! 构建产物只认项目文件夹下面一层、和项目类型对应的文件夹，例如 Node 项目的 `node_modules`、
//! Rust 项目的 `target`，避免把同名的普通文件夹当成产物。不进入构建产物里面找项目
//! （`node_modules` 里到处都是 `package.json`）。

use dl_tree::{FileTree, NodeId, NodeKind};

/// 项目类型：标记文件（`*` 开头的按扩展名匹配）和它的构建产物文件夹。
struct Kind {
    name: &'static str,
    markers: &'static [&'static str],
    artifacts: &'static [&'static str],
}

const KINDS: &[Kind] = &[
    Kind {
        name: "Node.js",
        markers: &["package.json"],
        artifacts: &[
            "node_modules",
            ".next",
            ".nuxt",
            ".svelte-kit",
            ".turbo",
            ".parcel-cache",
        ],
    },
    Kind {
        name: "Rust",
        markers: &["Cargo.toml"],
        artifacts: &["target"],
    },
    Kind {
        name: "Python",
        markers: &["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"],
        artifacts: &[".venv", "venv", ".tox", ".pytest_cache", ".mypy_cache"],
    },
    Kind {
        name: ".NET",
        markers: &["*.csproj", "*.fsproj", "*.vbproj"],
        artifacts: &["bin", "obj"],
    },
    Kind {
        name: "Maven",
        markers: &["pom.xml"],
        artifacts: &["target"],
    },
    Kind {
        name: "Gradle",
        markers: &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
        artifacts: &["build", ".gradle"],
    },
    Kind {
        name: "Go",
        markers: &["go.mod"],
        artifacts: &[],
    },
    Kind {
        name: "PHP",
        markers: &["composer.json"],
        artifacts: &["vendor"],
    },
    Kind {
        name: "Flutter",
        markers: &["pubspec.yaml"],
        artifacts: &["build", ".dart_tool"],
    },
    Kind {
        name: "CMake",
        markers: &["CMakeLists.txt"],
        artifacts: &["build", "out"],
    },
];

/// 一个项目里的一个构建产物文件夹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    pub id: NodeId,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: NodeId,
    /// 项目类型，例如 `["Node.js"]`；一个文件夹可以同时是几种。
    pub kinds: Vec<&'static str>,
    /// 整个项目文件夹的大小（含构建产物）。
    pub size: u64,
    /// 构建产物，从大到小。
    pub artifacts: Vec<Artifact>,
    /// 构建产物以外最近一次修改的时间（Unix 秒，0 表示不知道）。
    pub modified: u32,
}

impl Project {
    /// 删掉构建产物能释放的大小。
    pub fn reclaimable(&self) -> u64 {
        self.artifacts.iter().map(|a| a.size).sum()
    }
}

/// `root` 子树里的开发项目，按能释放的大小从大到小。
pub fn projects(tree: &FileTree, root: NodeId) -> Vec<Project> {
    let mut out = Vec::new();
    visit(tree, root, &mut out);
    out.sort_by_key(|p| std::cmp::Reverse(p.reclaimable()));
    out
}

fn matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_prefix('*') {
        // 按字节截取末尾可能落在多字节字符中间（中文文件名），用 get 避免越界。
        Some(ext) => {
            name.len() > ext.len()
                && name
                    .get(name.len() - ext.len()..)
                    .is_some_and(|tail| tail.eq_ignore_ascii_case(ext))
        }
        None => name.eq_ignore_ascii_case(pattern),
    }
}

/// 处理目录 `dir`：判断它是不是项目，再往下找。返回它（不含构建产物）里最近的修改时间。
fn visit(tree: &FileTree, dir: NodeId, out: &mut Vec<Project>) -> u32 {
    let children: Vec<NodeId> = tree.children(dir).collect();
    let kinds: Vec<&Kind> = KINDS
        .iter()
        .filter(|k| {
            children.iter().any(|&c| {
                tree.kind(c) == NodeKind::File && k.markers.iter().any(|m| matches(m, tree.name(c)))
            })
        })
        .collect();
    let is_artifact = |c: NodeId| {
        tree.kind(c) == NodeKind::Dir
            && kinds.iter().any(|k| {
                k.artifacts
                    .iter()
                    .any(|a| tree.name(c).eq_ignore_ascii_case(a))
            })
    };

    let mut modified = tree.modified(dir);
    let mut artifacts = Vec::new();
    for &c in &children {
        if is_artifact(c) {
            artifacts.push(Artifact {
                id: c,
                name: tree.name(c).to_owned(),
                size: tree.size(c),
            });
        } else if tree.kind(c) == NodeKind::Dir {
            modified = modified.max(visit(tree, c, out));
        } else {
            modified = modified.max(tree.modified(c));
        }
    }
    if !kinds.is_empty() {
        artifacts.sort_by_key(|a| std::cmp::Reverse(a.size));
        out.push(Project {
            id: dir,
            kinds: kinds.iter().map(|k| k.name).collect(),
            size: tree.size(dir),
            artifacts,
            modified,
        });
    }
    modified
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_projects_and_their_artifacts() {
        let mut t = FileTree::new(r"D:\code");
        // Node 项目：node_modules 里的 package.json 不算项目；dist 不当作产物。
        let web = t.add_dir(0, "web");
        t.add_file(web, "package.json", 1_000, 4_096);
        let nm = t.add_dir(web, "node_modules");
        let dep = t.add_dir(nm, "left-pad");
        t.add_file(dep, "package.json", 500, 4_096);
        t.add_file(dep, "index.js", 9_000, 12_288);
        let dist = t.add_dir(web, "dist");
        t.add_file(dist, "app.js", 3_000, 4_096);
        // Rust 项目。
        let tool = t.add_dir(0, "tool");
        t.add_file(tool, "Cargo.toml", 300, 4_096);
        let target = t.add_dir(tool, "target");
        t.add_file(target, "tool.exe", 50_000, 53_248);
        // 不是项目的文件夹，里面的 target 不算产物。
        let notes = t.add_dir(0, "notes");
        let tdir = t.add_dir(notes, "target");
        t.add_file(tdir, "plan.md", 100, 4_096);
        // .NET 项目按扩展名识别。
        let app = t.add_dir(0, "App");
        t.add_file(app, "App.CSPROJ", 200, 4_096);
        let obj = t.add_dir(app, "obj");
        t.add_file(obj, "a.dll", 7_000, 8_192);
        t.finalize();

        let list = projects(&t, 0);
        let summary: Vec<(String, Vec<&str>, Vec<String>, u64)> = list
            .iter()
            .map(|p| {
                (
                    t.name(p.id).to_owned(),
                    p.kinds.clone(),
                    p.artifacts.iter().map(|a| a.name.clone()).collect(),
                    p.reclaimable(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "tool".into(),
                    vec!["Rust"],
                    vec!["target".to_string()],
                    50_000
                ),
                (
                    "web".into(),
                    vec!["Node.js"],
                    vec!["node_modules".to_string()],
                    9_500
                ),
                ("App".into(), vec![".NET"], vec!["obj".to_string()], 7_000),
            ]
        );
    }

    #[test]
    fn non_ascii_names_do_not_break_extension_markers() {
        assert!(!matches("*.csproj", "回答问题.txt"));
        assert!(!matches("*.csproj", "答"));
        assert!(matches("*.csproj", "项目.csproj"));
        assert!(matches("*.csproj", "a.CSPROJ"));
    }

    #[test]
    fn last_change_ignores_artifacts() {
        let mut t = FileTree::new("r");
        let p = t.add_dir(0, "p");
        let src = t.add_file(p, "Cargo.toml", 1, 1);
        let target = t.add_dir(p, "target");
        let out = t.add_file(target, "x", 1, 1);
        t.set_times(src, 1_000, 0);
        t.set_times(out, 9_000, 0);
        t.finalize();
        let list = projects(&t, 0);
        assert_eq!(list[0].modified, 1_000);
    }
}
