//! In-loop filters of an intra picture (H.265 8.7): the deblocking filter
//! across transform and prediction block edges on the 8x8 grid (every
//! such edge of an intra picture has boundary strength 2), then the sample
//! adaptive offset per CTB.

use super::decode::{Decoder, Unit};

const BETA: [i32; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
    20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 60, 62, 64,
];

const TC: [i32; 54] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3,
    3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 24,
];

/// The boundary strength of every intra edge.
const STRENGTH: i32 = 2;

/// Whether the edge between the block holding (px, py) and the one holding
/// (qx, qy), luma positions, is filtered: a block edge, inside the
/// picture, and not across a slice or tile boundary the q side's slice
/// keeps filters from.
fn filters_edge(decoder: &Decoder, px: usize, py: usize, qx: usize, qy: usize) -> bool {
    let q_ctb = (qy >> decoder.ctb_log2) * decoder.width_ctbs + (qx >> decoder.ctb_log2);
    let p_ctb = (py >> decoder.ctb_log2) * decoder.width_ctbs + (px >> decoder.ctb_log2);
    let q_slice = &decoder.slices[decoder.slice_of[q_ctb]];
    if q_slice.deblocking_disabled {
        return false;
    }
    if decoder.slice_of[p_ctb] != decoder.slice_of[q_ctb]
        && !q_slice.loop_filter_across_slices
        && slice_start(decoder, p_ctb) != slice_start(decoder, q_ctb)
    {
        return false;
    }
    if !decoder.pps.loop_filter_across_tiles && decoder.tile_of[p_ctb] != decoder.tile_of[q_ctb] {
        return false;
    }
    true
}

/// The raster address of the first CTB of the slice (not segment) holding
/// a CTB: dependent segments share their slice's.
fn slice_start(decoder: &Decoder, ctb: usize) -> usize {
    let mut index = decoder.slice_of[ctb];
    while index > 0 && decoder.slices[index].dependent {
        index -= 1;
    }
    decoder.slices[index].address as usize
}

pub fn deblock(decoder: &mut Decoder) {
    for vertical in [true, false] {
        deblock_luma(decoder, vertical);
        if decoder.chroma != 0 {
            for component in 1..3 {
                deblock_chroma(decoder, component, vertical);
            }
        }
    }
}

fn deblock_luma(decoder: &mut Decoder, vertical: bool) {
    let (width, height) = (decoder.width, decoder.height);
    let stride = decoder.strides[0];
    let bit_depth = decoder.sps.bit_depth_luma;
    let max = (1i32 << bit_depth) - 1;
    let scale = 1 << (bit_depth - 8);
    // Each edge segment of 4 lines on the 8x8 grid.
    let (outer, inner) = if vertical {
        (width, height)
    } else {
        (height, width)
    };
    for edge in (8..outer).step_by(8) {
        for segment in (0..inner).step_by(4) {
            let (qx, qy) = if vertical {
                (edge, segment)
            } else {
                (segment, edge)
            };
            let (px, py) = if vertical {
                (edge - 1, segment)
            } else {
                (segment, edge - 1)
            };
            let q_unit = *decoder.unit(qx, qy);
            let is_edge = if vertical {
                q_unit.left_edge
            } else {
                q_unit.top_edge
            };
            if !is_edge || !filters_edge(decoder, px, py, qx, qy) {
                continue;
            }
            let p_unit = *decoder.unit(px, py);
            let q_slice = &decoder.slices[decoder.slice_of
                [(qy >> decoder.ctb_log2) * decoder.width_ctbs + (qx >> decoder.ctb_log2)]];
            let qp = (i32::from(p_unit.qp) + i32::from(q_unit.qp) + 1) >> 1;
            let beta = BETA[(qp + q_slice.beta_offset).clamp(0, 51) as usize] * scale;
            let tc =
                TC[(qp + 2 * (STRENGTH - 1) + q_slice.tc_offset).clamp(0, 53) as usize] * scale;
            let plane = &mut decoder.planes[0];
            // Sample k across the edge (p side negative) on line `line`.
            let at = |line: usize, k: isize| -> usize {
                if vertical {
                    (segment + line) * stride + (edge as isize + k) as usize
                } else {
                    (edge as isize + k) as usize * stride + segment + line
                }
            };
            let lines = 4.min(inner - segment);
            let get = |plane: &[u16], line: usize, k: isize| i32::from(plane[at(line, k)]);
            let line_d = |plane: &[u16], line: usize| -> (i32, i32) {
                let dp =
                    (get(plane, line, -3) - 2 * get(plane, line, -2) + get(plane, line, -1)).abs();
                let dq =
                    (get(plane, line, 2) - 2 * get(plane, line, 1) + get(plane, line, 0)).abs();
                (dp, dq)
            };
            let last = lines - 1;
            let (dp0, dq0) = line_d(plane, 0);
            let (dp3, dq3) = line_d(plane, last.min(3));
            let d = dp0 + dq0 + dp3 + dq3;
            if d >= beta {
                continue;
            }
            let strong_line = |plane: &[u16], line: usize, dpq: i32| -> bool {
                2 * dpq < (beta >> 2)
                    && (get(plane, line, -4) - get(plane, line, -1)).abs()
                        + (get(plane, line, 0) - get(plane, line, 3)).abs()
                        < (beta >> 3)
                    && (get(plane, line, -1) - get(plane, line, 0)).abs() < ((5 * tc + 1) >> 1)
            };
            let strong =
                strong_line(plane, 0, dp0 + dq0) && strong_line(plane, last.min(3), dp3 + dq3);
            let side_threshold = (beta + (beta >> 1)) >> 3;
            let filter_p1 = dp0 + dp3 < side_threshold;
            let filter_q1 = dq0 + dq3 < side_threshold;
            let keep_p = p_unit.unfiltered;
            let keep_q = q_unit.unfiltered;
            for line in 0..lines {
                let p = [
                    get(plane, line, -1),
                    get(plane, line, -2),
                    get(plane, line, -3),
                    get(plane, line, -4),
                ];
                let q = [
                    get(plane, line, 0),
                    get(plane, line, 1),
                    get(plane, line, 2),
                    get(plane, line, 3),
                ];
                let set = |plane: &mut [u16], k: isize, value: i32| {
                    plane[at(line, k)] = value.clamp(0, max) as u16;
                };
                if strong {
                    if !keep_p {
                        set(
                            plane,
                            -1,
                            ((p[2] + 2 * p[1] + 2 * p[0] + 2 * q[0] + q[1] + 4) >> 3)
                                .clamp(p[0] - 2 * tc, p[0] + 2 * tc),
                        );
                        set(
                            plane,
                            -2,
                            ((p[2] + p[1] + p[0] + q[0] + 2) >> 2)
                                .clamp(p[1] - 2 * tc, p[1] + 2 * tc),
                        );
                        set(
                            plane,
                            -3,
                            ((2 * p[3] + 3 * p[2] + p[1] + p[0] + q[0] + 4) >> 3)
                                .clamp(p[2] - 2 * tc, p[2] + 2 * tc),
                        );
                    }
                    if !keep_q {
                        set(
                            plane,
                            0,
                            ((p[1] + 2 * p[0] + 2 * q[0] + 2 * q[1] + q[2] + 4) >> 3)
                                .clamp(q[0] - 2 * tc, q[0] + 2 * tc),
                        );
                        set(
                            plane,
                            1,
                            ((p[0] + q[0] + q[1] + q[2] + 2) >> 2)
                                .clamp(q[1] - 2 * tc, q[1] + 2 * tc),
                        );
                        set(
                            plane,
                            2,
                            ((p[0] + q[0] + q[1] + 3 * q[2] + 2 * q[3] + 4) >> 3)
                                .clamp(q[2] - 2 * tc, q[2] + 2 * tc),
                        );
                    }
                } else {
                    let mut delta = (9 * (q[0] - p[0]) - 3 * (q[1] - p[1]) + 8) >> 4;
                    if delta.abs() >= tc * 10 {
                        continue;
                    }
                    delta = delta.clamp(-tc, tc);
                    if !keep_p {
                        set(plane, -1, p[0] + delta);
                    }
                    if !keep_q {
                        set(plane, 0, q[0] - delta);
                    }
                    if filter_p1 && !keep_p {
                        let delta_p = ((((p[2] + p[0] + 1) >> 1) - p[1] + delta) >> 1)
                            .clamp(-(tc >> 1), tc >> 1);
                        set(plane, -2, p[1] + delta_p);
                    }
                    if filter_q1 && !keep_q {
                        let delta_q = ((((q[2] + q[0] + 1) >> 1) - q[1] - delta) >> 1)
                            .clamp(-(tc >> 1), tc >> 1);
                        set(plane, 1, q[1] + delta_q);
                    }
                }
            }
        }
    }
}

fn deblock_chroma(decoder: &mut Decoder, component: usize, vertical: bool) {
    let (sub_x, sub_y) = (decoder.sub_x, decoder.sub_y);
    let width = decoder.width >> sub_x;
    let height = decoder.height >> sub_y;
    let stride = decoder.strides[component];
    let bit_depth = decoder.sps.bit_depth_chroma;
    let max = (1i32 << bit_depth) - 1;
    let scale = 1 << (bit_depth - 8);
    let picture_offset = if component == 1 {
        decoder.pps.cb_qp_offset
    } else {
        decoder.pps.cr_qp_offset
    };
    let (outer, inner) = if vertical {
        (width, height)
    } else {
        (height, width)
    };
    for edge in (8..outer).step_by(8) {
        for line in 0..inner {
            let (cx, cy) = if vertical { (edge, line) } else { (line, edge) };
            let (qx, qy) = (cx << sub_x, cy << sub_y);
            let (px, py) = if vertical { (qx - 1, qy) } else { (qx, qy - 1) };
            let q_unit: Unit = *decoder.unit(qx, qy);
            let is_edge = if vertical {
                q_unit.left_edge
            } else {
                q_unit.top_edge
            };
            if !is_edge || !filters_edge(decoder, px, py, qx, qy) {
                continue;
            }
            let p_unit = *decoder.unit(px, py);
            let q_slice = &decoder.slices[decoder.slice_of
                [(qy >> decoder.ctb_log2) * decoder.width_ctbs + (qx >> decoder.ctb_log2)]];
            let qpi = ((i32::from(p_unit.qp) + i32::from(q_unit.qp) + 1) >> 1) + picture_offset;
            let qpc = if decoder.chroma == 1 {
                match qpi {
                    ..30 => qpi,
                    30..=43 => [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37]
                        [(qpi - 30) as usize],
                    _ => qpi - 6,
                }
            } else {
                qpi.min(51)
            };
            let tc =
                TC[(qpc + 2 * (STRENGTH - 1) + q_slice.tc_offset).clamp(0, 53) as usize] * scale;
            if tc == 0 {
                continue;
            }
            let plane = &mut decoder.planes[component];
            let at = |k: isize| -> usize {
                if vertical {
                    line * stride + (edge as isize + k) as usize
                } else {
                    (edge as isize + k) as usize * stride + line
                }
            };
            let (p0, p1) = (i32::from(plane[at(-1)]), i32::from(plane[at(-2)]));
            let (q0, q1) = (i32::from(plane[at(0)]), i32::from(plane[at(1)]));
            let delta = ((((q0 - p0) << 2) + p1 - q1 + 4) >> 3).clamp(-tc, tc);
            if !p_unit.unfiltered {
                plane[at(-1)] = (p0 + delta).clamp(0, max) as u16;
            }
            if !q_unit.unfiltered {
                plane[at(0)] = (q0 - delta).clamp(0, max) as u16;
            }
        }
    }
}

/// Sample adaptive offset (8.7.3), from the deblocked picture.
pub fn sample_adaptive_offset(decoder: &mut Decoder) {
    if !decoder
        .slices
        .iter()
        .any(|slice| slice.sao_luma || slice.sao_chroma)
    {
        return;
    }
    let components = if decoder.chroma == 0 { 1 } else { 3 };
    for component in 0..components {
        let source = decoder.planes[component].clone();
        let (shift_x, shift_y) = if component == 0 {
            (0, 0)
        } else {
            (decoder.sub_x, decoder.sub_y)
        };
        let width = decoder.width >> shift_x;
        let height = decoder.height >> shift_y;
        let stride = decoder.strides[component];
        let bit_depth = if component == 0 {
            decoder.sps.bit_depth_luma
        } else {
            decoder.sps.bit_depth_chroma
        };
        let max = (1i32 << bit_depth) - 1;
        let ctb = 1usize << decoder.ctb_log2;
        for ctb_y in 0..decoder.height_ctbs {
            for ctb_x in 0..decoder.width_ctbs {
                let rs = ctb_y * decoder.width_ctbs + ctb_x;
                let slice = &decoder.slices[decoder.slice_of[rs]];
                let enabled = if component == 0 {
                    slice.sao_luma
                } else {
                    slice.sao_chroma
                };
                let params = decoder.sao[rs][component];
                if !enabled || params.kind == 0 {
                    continue;
                }
                let x0 = (ctb_x * ctb) >> shift_x;
                let y0 = (ctb_y * ctb) >> shift_y;
                let x1 = (((ctb_x + 1) * ctb) >> shift_x).min(width);
                let y1 = (((ctb_y + 1) * ctb) >> shift_y).min(height);
                let mut band_table = [0usize; 32];
                for k in 0..4 {
                    band_table[(k + usize::from(params.band)) & 31] = k + 1;
                }
                let (dx, dy): ([isize; 2], [isize; 2]) = match params.class {
                    0 => ([-1, 1], [0, 0]),
                    1 => ([0, 0], [-1, 1]),
                    2 => ([-1, 1], [-1, 1]),
                    _ => ([1, -1], [-1, 1]),
                };
                for y in y0..y1 {
                    for x in x0..x1 {
                        let luma = (x << shift_x, y << shift_y);
                        if decoder.unit(luma.0, luma.1).unfiltered {
                            continue;
                        }
                        let value = i32::from(source[y * stride + x]);
                        let offset = if params.kind == 1 {
                            params.offsets[band_table[(value >> (bit_depth - 5)) as usize]]
                        } else {
                            let mut signs = 0;
                            let mut skip = false;
                            for k in 0..2 {
                                let nx = x as isize + dx[k];
                                let ny = y as isize + dy[k];
                                if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height
                                {
                                    skip = true;
                                    break;
                                }
                                let neighbour_luma =
                                    ((nx as usize) << shift_x, (ny as usize) << shift_y);
                                if !sao_crosses(decoder, luma, neighbour_luma) {
                                    skip = true;
                                    break;
                                }
                                let neighbour =
                                    i32::from(source[ny as usize * stride + nx as usize]);
                                signs += (value - neighbour).signum();
                            }
                            if skip {
                                continue;
                            }
                            let index = match 2 + signs {
                                0 => 1,
                                1 => 2,
                                2 => 0,
                                other => other as usize,
                            };
                            params.offsets[index]
                        };
                        if offset != 0 {
                            decoder.planes[component][y * stride + x] =
                                (value + offset).clamp(0, max) as u16;
                        }
                    }
                }
            }
        }
    }
}

/// Whether an edge offset may read the sample at `neighbour` for the one at
/// `current` (luma positions): the slice and tile rules of 8.7.3.
fn sao_crosses(decoder: &Decoder, current: (usize, usize), neighbour: (usize, usize)) -> bool {
    let ctb_of = |(x, y): (usize, usize)| {
        (y >> decoder.ctb_log2) * decoder.width_ctbs + (x >> decoder.ctb_log2)
    };
    let (a, b) = (ctb_of(current), ctb_of(neighbour));
    if a == b {
        return true;
    }
    if slice_start(decoder, a) != slice_start(decoder, b) {
        let current_first = decoder.rs_to_ts[a] < decoder.rs_to_ts[b];
        let flag_holder = if current_first { b } else { a };
        if !decoder.slices[decoder.slice_of[flag_holder]].loop_filter_across_slices {
            return false;
        }
    }
    if !decoder.pps.loop_filter_across_tiles && decoder.tile_of[a] != decoder.tile_of[b] {
        return false;
    }
    let _: &Unit = decoder.unit(current.0, current.1);
    true
}
