//! 矩形树图布局（squarified 算法，Bruls 等，2000）。
//!
//! 按权重从大到小，逐行往剩余区域的短边上排；每加入一项都检查这一行的最差长宽比，
//! 变差了就另起一行。得到的矩形尽量接近正方形，标签和点击目标都更好用。

use crate::{FileTree, NodeId};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn area(&self) -> f32 {
        self.w * self.h
    }
}

/// 把 `area` 按 `weights` 划分，返回与 `weights` 一一对应的矩形。
/// `weights` 应按从大到小排列；权重为 0 的项得到面积为 0 的矩形。
pub fn squarify(weights: &[u64], area: Rect) -> Vec<Rect> {
    let total: f64 = weights.iter().map(|&w| w as f64).sum();
    let mut out = vec![
        Rect {
            x: area.x,
            y: area.y,
            w: 0.0,
            h: 0.0
        };
        weights.len()
    ];
    if total <= 0.0 || area.w <= 0.0 || area.h <= 0.0 {
        return out;
    }

    // 权重换算成面积。
    let scale = f64::from(area.w) * f64::from(area.h) / total;
    let areas: Vec<f64> = weights.iter().map(|&w| w as f64 * scale).collect();

    let (mut x, mut y) = (f64::from(area.x), f64::from(area.y));
    let (mut w, mut h) = (f64::from(area.w), f64::from(area.h));
    let mut start = 0;
    while start < areas.len() && areas[start] > 0.0 {
        let side = w.min(h);
        let mut end = start + 1;
        let mut row_sum = areas[start];
        while end < areas.len() && areas[end] > 0.0 {
            let next_sum = row_sum + areas[end];
            if worst(&areas[start..=end], next_sum, side) > worst(&areas[start..end], row_sum, side)
            {
                break;
            }
            row_sum = next_sum;
            end += 1;
        }

        // 这一行沿短边排开，厚度 = 行面积 / 短边。
        let thickness = row_sum / side;
        let mut offset = 0.0;
        for i in start..end {
            let len = areas[i] / thickness;
            out[i] = if w >= h {
                Rect {
                    x: x as f32,
                    y: (y + offset) as f32,
                    w: thickness as f32,
                    h: len as f32,
                }
            } else {
                Rect {
                    x: (x + offset) as f32,
                    y: y as f32,
                    w: len as f32,
                    h: thickness as f32,
                }
            };
            offset += len;
        }
        if w >= h {
            x += thickness;
            w -= thickness;
        } else {
            y += thickness;
            h -= thickness;
        }
        start = end;
    }
    out
}

/// 一行中最差（最大）的长宽比。
fn worst(row: &[f64], sum: f64, side: f64) -> f64 {
    let max = row.iter().copied().fold(0.0, f64::max);
    let min = row.iter().copied().fold(f64::INFINITY, f64::min);
    let s2 = side * side;
    let sum2 = sum * sum;
    (s2 * max / sum2).max(sum2 / (s2 * min))
}

impl FileTree {
    /// `id` 的子项在 `area` 内的树图布局，按大小从大到小。
    ///
    /// 最多布局 `max` 项；面积小于 `min_area` 的矩形不返回（它们的位置留空，
    /// 但仍占着按比例分到的面积，所以大块的比例不失真）。
    pub fn treemap(
        &self,
        id: NodeId,
        area: Rect,
        max: usize,
        min_area: f32,
    ) -> Vec<(NodeId, Rect)> {
        let kids: Vec<NodeId> = self
            .children_by_size(id)
            .into_iter()
            .take_while(|&c| self.size(c) > 0)
            .collect();
        // 超出 max 的部分合并成剩余权重，参与布局但不返回。
        let mut weights: Vec<u64> = kids.iter().take(max).map(|&c| self.size(c)).collect();
        let rest: u64 = kids.iter().skip(max).map(|&c| self.size(c)).sum();
        if rest > 0 {
            weights.push(rest);
        }
        let rects = squarify(&weights, area);
        kids.into_iter()
            .take(max)
            .zip(rects)
            .filter(|(_, r)| r.area() >= min_area)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: 10.0,
        y: 20.0,
        w: 600.0,
        h: 400.0,
    };

    fn inside(r: &Rect, a: &Rect) -> bool {
        let eps = 0.01;
        r.x >= a.x - eps
            && r.y >= a.y - eps
            && r.x + r.w <= a.x + a.w + eps
            && r.y + r.h <= a.y + a.h + eps
    }

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        let eps = 0.01;
        a.x + eps < b.x + b.w
            && b.x + eps < a.x + a.w
            && a.y + eps < b.y + b.h
            && b.y + eps < a.y + a.h
    }

    #[test]
    fn areas_are_proportional_and_fill_the_space() {
        let weights = [600u64, 600, 400, 200, 200, 100, 50, 25, 25];
        let rects = squarify(&weights, AREA);
        let total: u64 = weights.iter().sum();
        for (w, r) in weights.iter().zip(&rects) {
            let want = AREA.area() * *w as f32 / total as f32;
            assert!((r.area() - want).abs() < 0.5, "{r:?} want {want}");
            assert!(inside(r, &AREA), "{r:?}");
        }
        let sum: f32 = rects.iter().map(Rect::area).sum();
        assert!((sum - AREA.area()).abs() < 1.0);
        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                assert!(!overlaps(&rects[i], &rects[j]), "{i} {j}");
            }
        }
    }

    #[test]
    fn rectangles_stay_reasonably_square() {
        let weights: Vec<u64> = (1..=40).rev().map(|i| i * 10).collect();
        for r in squarify(&weights, AREA) {
            let ratio = r.w.max(r.h) / r.w.min(r.h);
            assert!(ratio < 5.0, "{r:?}");
        }
    }

    #[test]
    fn handles_empty_and_zero_weights() {
        assert!(squarify(&[], AREA).is_empty());
        let rects = squarify(&[0, 0], AREA);
        assert!(rects.iter().all(|r| r.area() == 0.0));
        let rects = squarify(&[5, 0], AREA);
        assert!((rects[0].area() - AREA.area()).abs() < 0.5);
        assert_eq!(rects[1].area(), 0.0);
    }

    #[test]
    fn tree_treemap_limits_and_filters() {
        let mut t = FileTree::new("root");
        t.add_file(0, "big", 10_000, 0);
        for i in 0..50 {
            t.add_file(0, &format!("small{i}"), 1, 0);
        }
        t.add_file(0, "empty", 0, 0);
        t.finalize();

        let all = t.treemap(0, AREA, 100, 0.0);
        assert_eq!(all.len(), 51); // 空文件不参与
        assert_eq!(t.name(all[0].0), "big");

        let few = t.treemap(0, AREA, 5, 0.0);
        assert_eq!(few.len(), 5);
        // 其余 46 项合并成剩余权重，big 的面积比例不变。
        let big = few[0].1.area() / AREA.area();
        assert!((big - 10_000.0 / 10_050.0).abs() < 0.001);

        // 每个小文件约 240000 / 10050 ≈ 24 平方像素，低于 100 的阈值被过滤。
        let visible = t.treemap(0, AREA, 100, 100.0);
        assert_eq!(visible.len(), 1);
    }
}
