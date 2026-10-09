//! Intra sample prediction (H.265 8.4.4.2): reference samples substituted
//! and filtered, then planar, DC, or one of 33 angles.

/// The reference samples of an `n` by `n` block: `corner` is p[-1][-1],
/// `top[x]` p[x][-1] and `left[y]` p[-1][y] for 0 to 2n - 1.
pub struct References {
    pub corner: i32,
    pub top: [i32; 64],
    pub left: [i32; 64],
}

/// The most reference samples a block has: 2n left, the corner, 2n top.
pub const LINE: usize = 4 * 32 + 1;

impl References {
    /// Fills unavailable samples (8.4.4.2.2). `line` holds the samples in
    /// that clause's order (the left column from the bottom up, the
    /// corner, the top row left to right) and `known` which are
    /// available; each missing one takes the last available one before
    /// it, the first from the first available; with none, the mid-level.
    pub fn substitute(n: usize, bit_depth: u32, line: &mut [i32], known: &[bool]) -> References {
        let total = 4 * n + 1;
        let (line, known) = (&mut line[..total], &known[..total]);
        match known.iter().position(|&flag| flag) {
            None => line.fill(1 << (bit_depth - 1)),
            Some(first) => {
                let mut last = line[first];
                for (value, &flag) in line.iter_mut().zip(known) {
                    if flag {
                        last = *value;
                    } else {
                        *value = last;
                    }
                }
            }
        }
        let mut references = References {
            corner: line[2 * n],
            top: [0; 64],
            left: [0; 64],
        };
        references.top[..2 * n].copy_from_slice(&line[2 * n + 1..]);
        for (value, &sample) in references.left[..2 * n]
            .iter_mut()
            .zip(line[..2 * n].iter().rev())
        {
            *value = sample;
        }
        references
    }

    /// The filtering of 8.4.4.2.3: [1 2 1], or for 32x32 luma with strong
    /// intra smoothing on a smooth enough edge, a straight line.
    pub fn filter(&mut self, n: usize, strong: bool, bit_depth: u32) {
        let last = 2 * n - 1;
        let threshold = 1 << (bit_depth - 5);
        if strong
            && n == 32
            && (self.corner + self.top[last] - 2 * self.top[n - 1]).abs() < threshold
            && (self.corner + self.left[last] - 2 * self.left[n - 1]).abs() < threshold
        {
            let corner = self.corner;
            let (top_end, left_end) = (self.top[last], self.left[last]);
            for index in 0..last {
                let weight = index as i32 + 1;
                self.top[index] = ((64 - weight) * corner + weight * top_end + 32) >> 6;
                self.left[index] = ((64 - weight) * corner + weight * left_end + 32) >> 6;
            }
            return;
        }
        let corner = self.corner;
        let smooth = |line: &mut [i32; 64]| {
            let mut before = corner;
            for index in 0..last {
                let here = line[index];
                line[index] = (before + 2 * here + line[index + 1] + 2) >> 2;
                before = here;
            }
        };
        let new_corner = (self.left[0] + 2 * corner + self.top[0] + 2) >> 2;
        smooth(&mut self.top);
        smooth(&mut self.left);
        self.corner = new_corner;
    }
}

/// Whether the references of a block are filtered (8.4.4.2.3).
pub fn filters(mode: u32, n: usize) -> bool {
    if mode == 1 || n == 4 {
        return false;
    }
    let distance = (mode as i32 - 26).abs().min((mode as i32 - 10).abs());
    let threshold = match n {
        8 => 7,
        16 => 1,
        _ => 0,
    };
    distance > threshold
}

const ANGLES: [i32; 35] = [
    0, 0, 32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26, -32, -26, -21, -17, -13,
    -9, -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32,
];

const INVERSE_ANGLES: [i32; 15] = [
    -4096, -1638, -910, -630, -482, -390, -315, -256, -315, -390, -482, -630, -910, -1638, -4096,
];

/// Predicts an `n` by `n` block into `out` (row-major). `edge_filters`
/// is whether DC and the pure horizontal and vertical modes smooth their
/// first row or column (luma below 32x32).
pub fn predict(
    references: &References,
    mode: u32,
    n: usize,
    edge_filters: bool,
    bit_depth: u32,
    out: &mut [i32],
) {
    let max = (1 << bit_depth) - 1;
    let log2 = n.trailing_zeros();
    match mode {
        0 => {
            let (top_right, bottom_left) = (references.top[n], references.left[n]);
            for y in 0..n {
                for x in 0..n {
                    let value = (n - 1 - x) as i32 * references.left[y]
                        + (x + 1) as i32 * top_right
                        + (n - 1 - y) as i32 * references.top[x]
                        + (y + 1) as i32 * bottom_left
                        + n as i32;
                    out[y * n + x] = value >> (log2 + 1);
                }
            }
        }
        1 => {
            let sum: i32 =
                references.top[..n].iter().sum::<i32>() + references.left[..n].iter().sum::<i32>();
            let dc = (sum + n as i32) >> (log2 + 1);
            out[..n * n].fill(dc);
            if edge_filters && n < 32 {
                out[0] = (references.left[0] + 2 * dc + references.top[0] + 2) >> 2;
                for (value, &top) in out[1..n].iter_mut().zip(&references.top[1..n]) {
                    *value = (top + 3 * dc + 2) >> 2;
                }
                for y in 1..n {
                    out[y * n] = (references.left[y] + 3 * dc + 2) >> 2;
                }
            }
        }
        _ => {
            let angle = ANGLES[mode as usize];
            let vertical = mode >= 18;
            // The main reference (top for vertical modes, left otherwise),
            // indexed from -n: ref[0] is the corner.
            let (main, side) = if vertical {
                (&references.top, &references.left)
            } else {
                (&references.left, &references.top)
            };
            let mut line = [0i32; 3 * 32 + 1];
            let origin = n;
            line[origin] = references.corner;
            for index in 0..2 * n {
                line[origin + 1 + index] = main[index];
            }
            if angle < 0 {
                let inverse = INVERSE_ANGLES[(mode - 11) as usize];
                let last = (n as i32 * angle) >> 5;
                if last < -1 {
                    for k in last..=-1 {
                        let index = ((k * inverse + 128) >> 8) - 1;
                        line[(origin as i32 + k) as usize] = side[index as usize];
                    }
                }
            }
            // Vertical modes run i across and j down: each j is a row of
            // `out`. Horizontal ones are worked the same way into rows and
            // turned after.
            for j in 0..n {
                let position = (j as i32 + 1) * angle;
                let offset = position >> 5;
                let fraction = position & 31;
                let start = (origin as i32 + offset + 1) as usize;
                let row = &mut out[j * n..(j + 1) * n];
                if fraction == 0 {
                    row.copy_from_slice(&line[start..start + n]);
                } else {
                    for (value, pair) in row.iter_mut().zip(line[start..start + n + 1].windows(2)) {
                        *value = ((32 - fraction) * pair[0] + fraction * pair[1] + 16) >> 5;
                    }
                }
            }
            if !vertical {
                for y in 0..n {
                    for x in y + 1..n {
                        out.swap(y * n + x, x * n + y);
                    }
                }
            }
            if edge_filters && n < 32 {
                if mode == 26 {
                    for y in 0..n {
                        out[y * n] = (references.top[0]
                            + ((references.left[y] - references.corner) >> 1))
                            .clamp(0, max);
                    }
                } else if mode == 10 {
                    for (value, &top) in out[..n].iter_mut().zip(&references.top[..n]) {
                        *value =
                            (references.left[0] + ((top - references.corner) >> 1)).clamp(0, max);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(_n: usize, value: i32) -> References {
        References {
            corner: value,
            top: [value; 64],
            left: [value; 64],
        }
    }

    #[test]
    fn flat_references_predict_flat_blocks() {
        for mode in 0..35 {
            for n in [4usize, 8, 16, 32] {
                let mut out = vec![0; n * n];
                predict(&flat(n, 77), mode, n, true, 8, &mut out);
                assert!(out.iter().all(|&value| value == 77), "mode {mode} size {n}");
            }
        }
    }

    #[test]
    fn missing_references_take_their_neighbours() {
        // Only the top row available: the left column and corner take top[0].
        let mut line = [0i32; LINE];
        let mut known = [false; LINE];
        for x in 0..8 {
            line[9 + x] = x as i32 + 10;
            known[9 + x] = true;
        }
        let references = References::substitute(4, 8, &mut line, &known);
        assert_eq!(references.corner, 10);
        assert!(references.left[..8].iter().all(|&value| value == 10));
        assert_eq!(references.top[7], 17);
        let none = References::substitute(4, 10, &mut [0; LINE], &[false; LINE]);
        assert_eq!(none.corner, 512);
    }
}
