//! Scaling and transformation (H.265 8.6.2 to 8.6.4): coefficient levels
//! to residuals, through the inverse DCT (4 to 32 points), the 4x4 DST of
//! intra luma, transform skip, or neither (transquant bypass).

const DST4: [[i32; 4]; 4] = [
    [29, 55, 74, 84],
    [74, 74, 0, -74],
    [84, -29, -74, 55],
    [55, -84, 74, -29],
];

/// The DCT matrix widened, so its rows feed multiply-adds directly.
const DCT32_WIDE: [[i32; 32]; 32] = {
    let mut wide = [[0i32; 32]; 32];
    let mut row = 0;
    while row < 32 {
        let mut column = 0;
        while column < 32 {
            wide[row][column] = DCT32[row][column] as i32;
            column += 1;
        }
        row += 1;
    }
    wide
};

/// The inverse transform of an `n` by `n` block of scaled coefficients,
/// row-major, in place: columns first, the intermediate clipped to 16
/// bits, then rows, then the final shift (8.6.4.2, 8.6.2).
pub fn inverse_transform(block: &mut [i32], log2: u32, dst: bool, bit_depth: u32) {
    let n = 1usize << log2;
    let step = 32 >> log2;
    let shift = 20 - bit_depth as i32;
    let round = 1i32 << (shift - 1);
    // Rows with any coefficient, and the highest frequency used, bound the work.
    let mut last_row = 0;
    let mut last_column = 0;
    for (y, row) in block[..n * n].chunks_exact(n).enumerate() {
        if let Some(x) = row.iter().rposition(|&level| level != 0) {
            last_row = y + 1;
            last_column = last_column.max(x + 1);
        }
    }
    if last_row == 0 {
        block[..n * n].fill(0);
        return;
    }
    let block = &mut block[..n * n];
    // A lone DC coefficient (most blocks): a flat block.
    if !dst && last_row == 1 && last_column == 1 {
        let column = ((64 * block[0] + 64) >> 7).clamp(-32768, 32767);
        block.fill((64 * column + round) >> shift);
        return;
    }
    let matrix = |frequency: usize| -> &[i32] {
        if dst {
            &DST4[frequency]
        } else {
            &DCT32_WIDE[frequency * step][..n]
        }
    };
    // Columns: e[y][x] = sum over j of M[j][y] * d[j][x], a row of the
    // intermediate at a time so the inner loop runs along x.
    let mut intermediate = [0i32; 32 * 32];
    for y in 0..n {
        let target = &mut intermediate[y * n..y * n + last_column];
        for j in 0..last_row {
            let weight = matrix(j)[y];
            let source = &block[j * n..j * n + last_column];
            for (value, &level) in target.iter_mut().zip(source) {
                *value += weight * level;
            }
        }
        for value in target.iter_mut() {
            *value = ((*value + 64) >> 7).clamp(-32768, 32767);
        }
    }
    // Rows: r[y][x] = sum over j of M[j][x] * e[y][j].
    let mut row = [0i32; 32];
    for y in 0..n {
        let row = &mut row[..n];
        row.fill(0);
        for j in 0..last_column {
            let value = intermediate[y * n + j];
            if value == 0 {
                continue;
            }
            for (sum, &weight) in row.iter_mut().zip(matrix(j)) {
                *sum += weight * value;
            }
        }
        for (out, &sum) in block[y * n..(y + 1) * n].iter_mut().zip(row.iter()) {
            *out = (sum + round) >> shift;
        }
    }
}

/// Transform skip (8.6.4.2): the scaled coefficients shifted into residuals,
/// rotated half a turn first when `rotate` (range extension).
pub fn transform_skip(block: &mut [i32], log2: u32, bit_depth: u32, rotate: bool) {
    let n = 1usize << log2;
    if rotate {
        block[..n * n].reverse();
    }
    let ts_shift = 5 + log2 as i32;
    let shift = 20 - bit_depth as i32;
    let round = 1i32 << (shift - 1);
    for value in block.iter_mut().take(n * n) {
        *value = ((*value << ts_shift) + round) >> shift;
    }
}

/// The level scale of each `qP % 6` (8.6.3).
pub const LEVEL_SCALE: [i32; 6] = [40, 45, 51, 57, 64, 72];

/// The 32-point inverse transform's coefficients (8.6.4.2, the matrix of
/// equation 8-319); smaller transforms take every 2nd, 4th, or 8th row.
pub const DCT32: [[i8; 32]; 32] = [
    [
        64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64, 64,
        64, 64, 64, 64, 64, 64, 64, 64, 64,
    ],
    [
        90, 90, 88, 85, 82, 78, 73, 67, 61, 54, 46, 38, 31, 22, 13, 4, -4, -13, -22, -31, -38, -46,
        -54, -61, -67, -73, -78, -82, -85, -88, -90, -90,
    ],
    [
        90, 87, 80, 70, 57, 43, 25, 9, -9, -25, -43, -57, -70, -80, -87, -90, -90, -87, -80, -70,
        -57, -43, -25, -9, 9, 25, 43, 57, 70, 80, 87, 90,
    ],
    [
        90, 82, 67, 46, 22, -4, -31, -54, -73, -85, -90, -88, -78, -61, -38, -13, 13, 38, 61, 78,
        88, 90, 85, 73, 54, 31, 4, -22, -46, -67, -82, -90,
    ],
    [
        89, 75, 50, 18, -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89, 89, 75, 50, 18,
        -18, -50, -75, -89, -89, -75, -50, -18, 18, 50, 75, 89,
    ],
    [
        88, 67, 31, -13, -54, -82, -90, -78, -46, -4, 38, 73, 90, 85, 61, 22, -22, -61, -85, -90,
        -73, -38, 4, 46, 78, 90, 82, 54, 13, -31, -67, -88,
    ],
    [
        87, 57, 9, -43, -80, -90, -70, -25, 25, 70, 90, 80, 43, -9, -57, -87, -87, -57, -9, 43, 80,
        90, 70, 25, -25, -70, -90, -80, -43, 9, 57, 87,
    ],
    [
        85, 46, -13, -67, -90, -73, -22, 38, 82, 88, 54, -4, -61, -90, -78, -31, 31, 78, 90, 61, 4,
        -54, -88, -82, -38, 22, 73, 90, 67, 13, -46, -85,
    ],
    [
        83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83, 83, 36, -36, -83,
        -83, -36, 36, 83, 83, 36, -36, -83, -83, -36, 36, 83,
    ],
    [
        82, 22, -54, -90, -61, 13, 78, 85, 31, -46, -90, -67, 4, 73, 88, 38, -38, -88, -73, -4, 67,
        90, 46, -31, -85, -78, -13, 61, 90, 54, -22, -82,
    ],
    [
        80, 9, -70, -87, -25, 57, 90, 43, -43, -90, -57, 25, 87, 70, -9, -80, -80, -9, 70, 87, 25,
        -57, -90, -43, 43, 90, 57, -25, -87, -70, 9, 80,
    ],
    [
        78, -4, -82, -73, 13, 85, 67, -22, -88, -61, 31, 90, 54, -38, -90, -46, 46, 90, 38, -54,
        -90, -31, 61, 88, 22, -67, -85, -13, 73, 82, 4, -78,
    ],
    [
        75, -18, -89, -50, 50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75, 75, -18, -89, -50,
        50, 89, 18, -75, -75, 18, 89, 50, -50, -89, -18, 75,
    ],
    [
        73, -31, -90, -22, 78, 67, -38, -90, -13, 82, 61, -46, -88, -4, 85, 54, -54, -85, 4, 88,
        46, -61, -82, 13, 90, 38, -67, -78, 22, 90, 31, -73,
    ],
    [
        70, -43, -87, 9, 90, 25, -80, -57, 57, 80, -25, -90, -9, 87, 43, -70, -70, 43, 87, -9, -90,
        -25, 80, 57, -57, -80, 25, 90, 9, -87, -43, 70,
    ],
    [
        67, -54, -78, 38, 85, -22, -90, 4, 90, 13, -88, -31, 82, 46, -73, -61, 61, 73, -46, -82,
        31, 88, -13, -90, -4, 90, 22, -85, -38, 78, 54, -67,
    ],
    [
        64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64,
        64, -64, -64, 64, 64, -64, -64, 64, 64, -64, -64, 64,
    ],
    [
        61, -73, -46, 82, 31, -88, -13, 90, -4, -90, 22, 85, -38, -78, 54, 67, -67, -54, 78, 38,
        -85, -22, 90, 4, -90, 13, 88, -31, -82, 46, 73, -61,
    ],
    [
        57, -80, -25, 90, -9, -87, 43, 70, -70, -43, 87, 9, -90, 25, 80, -57, -57, 80, 25, -90, 9,
        87, -43, -70, 70, 43, -87, -9, 90, -25, -80, 57,
    ],
    [
        54, -85, -4, 88, -46, -61, 82, 13, -90, 38, 67, -78, -22, 90, -31, -73, 73, 31, -90, 22,
        78, -67, -38, 90, -13, -82, 61, 46, -88, 4, 85, -54,
    ],
    [
        50, -89, 18, 75, -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50, 50, -89, 18, 75,
        -75, -18, 89, -50, -50, 89, -18, -75, 75, 18, -89, 50,
    ],
    [
        46, -90, 38, 54, -90, 31, 61, -88, 22, 67, -85, 13, 73, -82, 4, 78, -78, -4, 82, -73, -13,
        85, -67, -22, 88, -61, -31, 90, -54, -38, 90, -46,
    ],
    [
        43, -90, 57, 25, -87, 70, 9, -80, 80, -9, -70, 87, -25, -57, 90, -43, -43, 90, -57, -25,
        87, -70, -9, 80, -80, 9, 70, -87, 25, 57, -90, 43,
    ],
    [
        38, -88, 73, -4, -67, 90, -46, -31, 85, -78, 13, 61, -90, 54, 22, -82, 82, -22, -54, 90,
        -61, -13, 78, -85, 31, 46, -90, 67, 4, -73, 88, -38,
    ],
    [
        36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36, 36, -83, 83, -36,
        -36, 83, -83, 36, 36, -83, 83, -36, -36, 83, -83, 36,
    ],
    [
        31, -78, 90, -61, 4, 54, -88, 82, -38, -22, 73, -90, 67, -13, -46, 85, -85, 46, 13, -67,
        90, -73, 22, 38, -82, 88, -54, -4, 61, -90, 78, -31,
    ],
    [
        25, -70, 90, -80, 43, 9, -57, 87, -87, 57, -9, -43, 80, -90, 70, -25, -25, 70, -90, 80,
        -43, -9, 57, -87, 87, -57, 9, 43, -80, 90, -70, 25,
    ],
    [
        22, -61, 85, -90, 73, -38, -4, 46, -78, 90, -82, 54, -13, -31, 67, -88, 88, -67, 31, 13,
        -54, 82, -90, 78, -46, 4, 38, -73, 90, -85, 61, -22,
    ],
    [
        18, -50, 75, -89, 89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18, 18, -50, 75, -89,
        89, -75, 50, -18, -18, 50, -75, 89, -89, 75, -50, 18,
    ],
    [
        13, -38, 61, -78, 88, -90, 85, -73, 54, -31, 4, 22, -46, 67, -82, 90, -90, 82, -67, 46,
        -22, -4, 31, -54, 73, -85, 90, -88, 78, -61, 38, -13,
    ],
    [
        9, -25, 43, -57, 70, -80, 87, -90, 90, -87, 80, -70, 57, -43, 25, -9, -9, 25, -43, 57, -70,
        80, -87, 90, -90, 87, -80, 70, -57, 43, -25, 9,
    ],
    [
        4, -13, 22, -31, 38, -46, 54, -61, 67, -73, 78, -82, 85, -88, 90, -90, 90, -90, 88, -85,
        82, -78, 73, -67, 61, -54, 46, -38, 31, -22, 13, -4,
    ],
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dc_coefficient_spreads_evenly() {
        // A lone DC level spreads to a flat block.
        for log2 in 2..=5 {
            let n = 1usize << log2;
            let mut block = vec![0i32; n * n];
            block[0] = 64 << 6;
            inverse_transform(&mut block, log2, false, 8);
            assert!(block.iter().all(|&value| value == block[0]), "size {n}");
        }
        let mut block = vec![0i32; 16];
        block[0] = 1 << 10;
        transform_skip(&mut block, 2, 8, false);
        assert_eq!(block[0], ((1 << 10 << 7) + (1 << 11)) >> 12);
    }
}
