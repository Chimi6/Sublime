//! Scan orders (H.265 6.5.3 to 6.5.5) and the constant tables of the
//! decoding processes.

use std::sync::OnceLock;

/// Up-right diagonal, horizontal, and vertical scans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scan {
    Diagonal = 0,
    Horizontal = 1,
    Vertical = 2,
}

/// The positions (x, y) of a `1 << log2` square block in `scan` order,
/// for block sizes 1 to 32.
pub fn scan(log2: u32, scan: Scan) -> &'static [(u8, u8)] {
    /// By size, then kind: the positions in order.
    type Scans = Vec<Vec<Vec<(u8, u8)>>>;
    static SCANS: OnceLock<Scans> = OnceLock::new();
    let scans = SCANS.get_or_init(|| {
        (0..6u32)
            .map(|log2| {
                let size = 1usize << log2;
                let mut diagonal = Vec::with_capacity(size * size);
                let (mut x, mut y) = (0i32, 0i32);
                while diagonal.len() < size * size {
                    while y >= 0 {
                        if (x as usize) < size && (y as usize) < size {
                            diagonal.push((x as u8, y as u8));
                        }
                        y -= 1;
                        x += 1;
                    }
                    y = x;
                    x = 0;
                }
                let horizontal = (0..size * size)
                    .map(|index| ((index % size) as u8, (index / size) as u8))
                    .collect();
                let vertical = (0..size * size)
                    .map(|index| ((index / size) as u8, (index % size) as u8))
                    .collect();
                vec![diagonal, horizontal, vertical]
            })
            .collect()
    });
    &scans[log2 as usize][scan as usize]
}

/// The 4x4 and 8x8 diagonal scans the scaling lists are coded in.
pub struct DiagonalScan(u32);

pub const DIAGONAL_4X4: DiagonalScan = DiagonalScan(2);
pub const DIAGONAL_8X8: DiagonalScan = DiagonalScan(3);

impl DiagonalScan {
    pub fn iter(&self) -> std::slice::Iter<'static, (u8, u8)> {
        scan(self.0, Scan::Diagonal).iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_scans_run_up_and_right() {
        assert_eq!(
            &scan(2, Scan::Diagonal)[..6],
            &[(0, 0), (0, 1), (1, 0), (0, 2), (1, 1), (2, 0)]
        );
        assert_eq!(scan(2, Scan::Diagonal)[15], (3, 3));
        assert_eq!(scan(1, Scan::Horizontal), &[(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert_eq!(scan(1, Scan::Vertical), &[(0, 0), (0, 1), (1, 0), (1, 1)]);
    }
}
