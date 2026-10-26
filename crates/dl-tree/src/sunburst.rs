//! 旭日图布局：当前目录在圆心，往外每一圈是下一层，每项按大小分得父项角度的相应比例。

use std::collections::VecDeque;

use crate::{FileTree, NodeId, NodeKind};

/// 旭日图上的一段弧。角度用圆周的比例表示，`0..1` 对应 `0..360°`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub id: NodeId,
    /// 第几圈，从 1 开始（第 1 圈是直接子项）。
    pub depth: u32,
    pub start: f32,
    pub end: f32,
}

impl Segment {
    pub fn sweep(&self) -> f32 {
        self.end - self.start
    }
}

impl FileTree {
    /// `id` 往外 `max_depth` 圈的旭日图，由内向外、每圈按大小从大到小。
    ///
    /// 张角小于 `min_sweep` 的弧不返回，也不再展开它的子项；最多返回 `max` 段。
    /// 每一项的角度按它在父项中的大小占比计算，所以略去的小项只是留空，不会让大项失真。
    pub fn sunburst(&self, id: NodeId, max_depth: u32, min_sweep: f32, max: usize) -> Vec<Segment> {
        let mut out = Vec::new();
        let mut queue = VecDeque::from([(id, 0u32, 0.0f32, 1.0f32)]);
        while let Some((parent, depth, start, end)) = queue.pop_front() {
            let total = self.size(parent);
            if total == 0 || depth >= max_depth {
                continue;
            }
            let span = end - start;
            let mut at = start;
            for child in self.children_by_size(parent) {
                let size = self.size(child);
                if size == 0 {
                    break;
                }
                let sweep = (size as f64 / total as f64) as f32 * span;
                if sweep < min_sweep {
                    break;
                }
                if out.len() >= max {
                    return out;
                }
                let seg = Segment {
                    id: child,
                    depth: depth + 1,
                    start: at,
                    end: at + sweep,
                };
                out.push(seg);
                if self.kind(child) == NodeKind::Dir {
                    queue.push_back((child, depth + 1, seg.start, seg.end));
                }
                at += sweep;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> FileTree {
        let mut t = FileTree::new("root");
        let a = t.add_dir(0, "a");
        t.add_file(a, "a1", 300, 0);
        t.add_file(a, "a2", 100, 0);
        let deep = t.add_dir(a, "deep");
        t.add_file(deep, "d1", 100, 0);
        t.add_file(0, "b", 400, 0);
        t.add_file(0, "tiny", 1, 0);
        t.add_file(0, "empty", 0, 0);
        t.finalize();
        t
    }

    fn find(segs: &[Segment], t: &FileTree, name: &str) -> Segment {
        *segs.iter().find(|s| t.name(s.id) == name).unwrap()
    }

    #[test]
    fn angles_follow_sizes() {
        let t = tree();
        let segs = t.sunburst(0, 3, 0.0, 100);
        let total = 901.0;
        let a = find(&segs, &t, "a");
        assert_eq!((a.depth, a.start), (1, 0.0));
        assert!((a.sweep() - 500.0 / total).abs() < 1e-6);
        let b = find(&segs, &t, "b");
        assert!((b.start - a.end).abs() < 1e-6);
        // 第 2 圈在父项的角度范围内。
        let a1 = find(&segs, &t, "a1");
        assert_eq!(a1.depth, 2);
        assert!(a1.start >= a.start && a1.end <= a.end + 1e-6);
        assert!((a1.sweep() - a.sweep() * 0.6).abs() < 1e-6);
        assert_eq!(find(&segs, &t, "d1").depth, 3);
        assert!(segs.iter().all(|s| t.name(s.id) != "empty"));
        // 由内向外排列。
        assert!(segs.windows(2).all(|w| w[0].depth <= w[1].depth));
    }

    #[test]
    fn limits_depth_size_and_count() {
        let t = tree();
        let segs = t.sunburst(0, 2, 0.0, 100);
        assert!(segs.iter().all(|s| s.depth <= 2));
        let segs = t.sunburst(0, 3, 0.01, 100);
        assert!(segs.iter().all(|s| t.name(s.id) != "tiny"));
        assert_eq!(t.sunburst(0, 3, 0.0, 2).len(), 2);
        let empty = t.child_by_name(0, "empty").unwrap();
        assert!(t.sunburst(empty, 3, 0.0, 100).is_empty());
    }
}
