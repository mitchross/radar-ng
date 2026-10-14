//! Connected-component labelling that matches `scipy.ndimage.label` (4-connectivity).
//!
//! scipy numbers components in raster order of their first pixel. A flood fill
//! started from each unvisited mask pixel in raster order gives the same
//! numbering, which fixes the tie order in the stable peak sort (spec-shared 6.3 step 2).

/// Per-component statistics needed by the detector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Component {
    /// Number of pixels in the component.
    pub size: u64,
    /// Maximum data value over the component's pixels.
    pub peak: f32,
    /// Sum of row indices (for the centroid, exact in u64).
    pub sum_row: u64,
    /// Sum of column indices.
    pub sum_col: u64,
    /// First row (inclusive).
    pub row_start: usize,
    /// Last row plus one (exclusive), as `scipy.ndimage.find_objects` reports.
    pub row_stop: usize,
    /// First column (inclusive).
    pub col_start: usize,
    /// Last column plus one (exclusive).
    pub col_stop: usize,
}

/// Label the `true` cells of `mask` (row-major, `height x width`) and return the
/// components in scipy label order (label 1 first).
///
/// # Panics
/// Panics if `mask` or `values` is shorter than `height * width`. The caller
/// validates lengths through [`crate::Grid`].
#[must_use]
pub fn label_components(
    mask: &[bool],
    values: &[f32],
    height: usize,
    width: usize,
) -> Vec<Component> {
    let total = height * width;
    let mut visited = vec![false; total];
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..total {
        if !mask[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        stack.push(start);
        let mut comp = Component {
            size: 0,
            peak: f32::NEG_INFINITY,
            sum_row: 0,
            sum_col: 0,
            row_start: usize::MAX,
            row_stop: 0,
            col_start: usize::MAX,
            col_stop: 0,
        };
        while let Some(idx) = stack.pop() {
            let r = idx / width;
            let c = idx % width;
            comp.size += 1;
            comp.sum_row += r as u64;
            comp.sum_col += c as u64;
            comp.peak = comp.peak.max(values[idx]);
            comp.row_start = comp.row_start.min(r);
            comp.row_stop = comp.row_stop.max(r + 1);
            comp.col_start = comp.col_start.min(c);
            comp.col_stop = comp.col_stop.max(c + 1);
            let mut visit = |n: usize, stack: &mut Vec<usize>| {
                if mask[n] && !visited[n] {
                    visited[n] = true;
                    stack.push(n);
                }
            };
            if r > 0 {
                visit(idx - width, &mut stack);
            }
            if r + 1 < height {
                visit(idx + width, &mut stack);
            }
            if c > 0 {
                visit(idx - 1, &mut stack);
            }
            if c + 1 < width {
                visit(idx + 1, &mut stack);
            }
        }
        out.push(comp);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mask_from(rows: &[&str]) -> (Vec<bool>, usize, usize) {
        let h = rows.len();
        let w = rows[0].len();
        let mask = rows
            .iter()
            .flat_map(|r| r.chars().map(|c| c == '#'))
            .collect();
        (mask, h, w)
    }

    #[test]
    fn diagonal_pixels_are_separate_under_4_connectivity() {
        let (mask, h, w) = mask_from(&["#.", ".#"]);
        let values = vec![50.0_f32; h * w];
        let comps = label_components(&mask, &values, h, w);
        assert_eq!(comps.len(), 2);
        assert_eq!(comps[0].size, 1);
        assert_eq!(comps[1].size, 1);
    }

    #[test]
    fn labels_follow_raster_order_of_first_pixel() {
        // Component B starts at (0,3); component A starts at (1,0). Raster order: B then A.
        let (mask, h, w) = mask_from(&["...##", "#....", "#...."]);
        let values = vec![50.0_f32; h * w];
        let comps = label_components(&mask, &values, h, w);
        assert_eq!(comps.len(), 2);
        assert_eq!((comps[0].row_start, comps[0].col_start), (0, 3));
        assert_eq!((comps[1].row_start, comps[1].col_start), (1, 0));
        assert_eq!(comps[1].size, 2);
    }

    #[test]
    fn bounds_are_half_open_and_peak_is_max_over_component() {
        let (mask, h, w) = mask_from(&["..", "##", ".."]);
        let values = vec![1.0, 2.0, 40.0, 41.5, 3.0, 4.0];
        let comps = label_components(&mask, &values, h, w);
        assert_eq!(comps.len(), 1);
        let c = comps[0];
        assert_eq!(
            (c.row_start, c.row_stop, c.col_start, c.col_stop),
            (1, 2, 0, 2)
        );
        assert_eq!(c.peak, 41.5);
        assert_eq!((c.sum_row, c.sum_col), (2, 1));
    }
}
