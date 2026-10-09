//! In-loop filters of an intra picture (H.265 8.7): the deblocking filter
//! across transform and prediction block edges on the 8x8 grid (every
//! such edge of an intra picture has boundary strength 2), then the sample
//! adaptive offset per CTB.

use super::decode::{Band, Neighbourhood, above_line, with_band};
use super::decode::{Decoder, Unit};
use super::sample::Sample;
use super::sample::Samples;

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
fn filters_edge<N: Neighbourhood + ?Sized>(
    near: &N,
    px: usize,
    py: usize,
    qx: usize,
    qy: usize,
) -> bool {
    let layout = near.layout();
    let q_ctb = (qy >> layout.ctb_log2) * layout.width_ctbs + (qx >> layout.ctb_log2);
    let p_ctb = (py >> layout.ctb_log2) * layout.width_ctbs + (px >> layout.ctb_log2);
    let q_slice = &near.slices()[near.slice_index(q_ctb)];
    if q_slice.deblocking_disabled {
        return false;
    }
    if near.slice_index(p_ctb) != near.slice_index(q_ctb)
        && !q_slice.loop_filter_across_slices
        && slice_start(near, p_ctb) != slice_start(near, q_ctb)
    {
        return false;
    }
    if !layout.pps.loop_filter_across_tiles && layout.tile_of[p_ctb] != layout.tile_of[q_ctb] {
        return false;
    }
    true
}

/// The raster address of the first CTB of the slice (not segment) holding
/// a CTB: dependent segments share their slice's.
fn slice_start<N: Neighbourhood + ?Sized>(near: &N, ctb: usize) -> usize {
    let mut index = near.slice_index(ctb);
    while index > 0 && near.slices()[index].dependent {
        index -= 1;
    }
    near.slices()[index].address as usize
}

/// The deblocking filter, its edges shared out by bands of rows: first
/// every vertical edge, a CTB row's band at a time; then every horizontal
/// edge, from bands that start four rows above each CTB row so an edge and
/// every sample it reads or changes lie in one band (edges are 8 apart and
/// reach 4 rows).
pub fn deblock(decoder: &mut Decoder) {
    // The planes come out of the decoder so the edge decisions can read
    // it while the samples change.
    let mut planes = std::mem::take(&mut decoder.planes);
    {
        let decoder: &Decoder = decoder;
        let ctb = 1usize << decoder.ctb_log2;
        for vertical in [true, false] {
            let mut tasks = Vec::new();
            for (component, plane) in planes.iter_mut().enumerate() {
                if component > 0 && decoder.chroma == 0 {
                    break;
                }
                let shift_y = if component == 0 { 0 } else { decoder.sub_y };
                let rows = decoder.height >> shift_y;
                let band = ctb >> shift_y;
                let stride = decoder.strides[component];
                // Bands start at each CTB row, or four rows above it.
                let lead = if vertical { 0 } else { 4 };
                let mut starts: Vec<usize> = (0..rows.div_ceil(band))
                    .map(|row| (row * band).saturating_sub(lead))
                    .collect();
                starts.dedup();
                let mut rest = plane.band();
                let mut taken = 0;
                for (index, &start) in starts.iter().enumerate() {
                    let end = starts.get(index + 1).copied().unwrap_or(rows);
                    let (piece, after) = rest.split_at((end - start) * stride);
                    rest = after;
                    debug_assert_eq!(taken, start);
                    taken = end;
                    // The edges this band filters: rows across for vertical
                    // edges, the edges' own rows for horizontal ones.
                    let edges = if vertical {
                        start..end
                    } else {
                        (index * band)..((index + 1) * band).min(rows)
                    };
                    tasks.push((component, start, piece, edges));
                }
            }
            run_parallel(
                decoder.threads,
                tasks,
                |(component, start, piece, edges)| {
                    with_band!(piece, plane => {
                        if component == 0 {
                            deblock_luma(decoder, plane, start, vertical, edges)
                        } else {
                            deblock_chroma(decoder, plane, start, component, vertical, edges)
                        }
                    })
                },
            );
        }
    }
    decoder.planes = planes;
}

/// The deblocking within band `band` (its CTB row) of every component:
/// its vertical edges, then the horizontal edges inside it (not the one at
/// its top, which needs the band above).
pub(super) fn deblock_inside<N: Neighbourhood + ?Sized>(
    near: &N,
    planes: &mut [Samples],
    band: usize,
) {
    let layout = near.layout();
    let ctb = 1usize << layout.ctb_log2;
    for (component, plane) in planes.iter_mut().enumerate() {
        let shift_y = if component == 0 { 0 } else { layout.sub_y };
        let rows =
            (layout.height >> shift_y).min((band + 1) * (ctb >> shift_y)) - band * (ctb >> shift_y);
        let top = band * (ctb >> shift_y);
        with_band!(plane.band(), samples => {
            for vertical in [true, false] {
                let edges = if vertical { top..top + rows } else { top + 8..top + rows };
                if component == 0 {
                    deblock_luma(near, samples, top, vertical, edges);
                } else {
                    deblock_chroma(near, samples, top, component, vertical, edges);
                }
            }
        });
    }
}

/// The horizontal edge between band `lower_band` and the band above it,
/// through a scratch copy of the four rows on each side.
pub(super) fn deblock_between<N: Neighbourhood + ?Sized>(
    near: &N,
    upper: &mut [Samples],
    lower: &mut [Samples],
    lower_band: usize,
) {
    let layout = near.layout();
    let ctb = 1usize << layout.ctb_log2;
    for (component, (upper, lower)) in upper.iter_mut().zip(lower.iter_mut()).enumerate() {
        let shift_y = if component == 0 { 0 } else { layout.sub_y };
        let band_rows = ctb >> shift_y;
        let edge = lower_band * band_rows;
        let stride = layout.strides[component];
        let mut scratch = upper.slice_of((band_rows - 4) * stride, 4 * stride);
        scratch.extend(&lower.slice_of(0, 4 * stride));
        with_band!(scratch.band(), samples => {
            if component == 0 {
                deblock_luma(near, samples, edge - 4, false, edge..edge + 1);
            } else {
                deblock_chroma(near, samples, edge - 4, component, false, edge..edge + 1);
            }
        });
        upper.copy_from((band_rows - 4) * stride, &scratch, 0, 4 * stride);
        lower.copy_from(0, &scratch, 4 * stride, 4 * stride);
    }
}

/// The offsets of band `band` of every component, from the line above
/// and the line below it as deblocked.
pub(super) fn sao_band_planes<N: Neighbourhood + ?Sized>(
    near: &N,
    planes: &mut [Samples],
    band: usize,
    above: Option<&[Samples]>,
    below: Option<&[Samples]>,
    unfiltered: bool,
) {
    let rules = SaoRules::of(near, unfiltered);
    for (component, plane) in planes.iter_mut().enumerate() {
        let above = above.map(|lines| &lines[component]);
        let below = below.map(|lines| &lines[component]);
        with_band!(plane.band(), samples => {
            sao_band(near, &rules, component, band, samples, above_line(above), above_line(below))
        });
    }
}

/// Runs `work` on every task, on up to `threads` threads.
fn run_parallel<T: Send>(threads: usize, tasks: Vec<T>, work: impl Fn(T) + Sync) {
    let threads = threads.min(tasks.len());
    if threads <= 1 {
        tasks.into_iter().for_each(work);
        return;
    }
    let queue = std::sync::Mutex::new(tasks.into_iter());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let task = queue
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .next();
                    match task {
                        Some(task) => work(task),
                        None => break,
                    }
                }
            });
        }
    });
}

/// The luma edges of a band of rows starting at row `start`: vertical
/// edges on the rows of `edges`, or the horizontal edges on rows `edges`.
fn deblock_luma<T: Sample, N: Neighbourhood + ?Sized>(
    near: &N,
    plane: &mut [T],
    start: usize,
    vertical: bool,
    edges: std::ops::Range<usize>,
) {
    let layout = near.layout();
    let (width, height) = (layout.width, layout.height);
    let stride = layout.strides[0];
    let bit_depth = layout.sps.bit_depth_luma;
    let max = (1i32 << bit_depth) - 1;
    let scale = 1 << (bit_depth - 8);
    // Each edge segment of 4 lines on the 8x8 grid.
    let (outer, inner) = if vertical {
        (width, height)
    } else {
        (height, width)
    };
    let _ = outer;
    let (edge_range, segment_range) = if vertical {
        (8..width, edges.start..edges.end.min(inner))
    } else {
        (edges.start.max(8)..edges.end, 0..inner)
    };
    let base = start * stride;
    for edge in edge_range.step_by(8) {
        for segment in segment_range.clone().step_by(4) {
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
            let q_unit = near.unit(qx, qy);
            let is_edge = if vertical {
                q_unit.left_edge
            } else {
                q_unit.top_edge
            };
            if !is_edge || !filters_edge(near, px, py, qx, qy) {
                continue;
            }
            let p_unit = near.unit(px, py);
            let q_slice = &near.slices()[near.slice_index(
                (qy >> layout.ctb_log2) * layout.width_ctbs + (qx >> layout.ctb_log2),
            )];
            let qp = (i32::from(p_unit.qp) + i32::from(q_unit.qp) + 1) >> 1;
            let beta = BETA[(qp + q_slice.beta_offset).clamp(0, 51) as usize] * scale;
            let tc =
                TC[(qp + 2 * (STRENGTH - 1) + q_slice.tc_offset).clamp(0, 53) as usize] * scale;
            // Sample k across the edge (p side negative) on line `line`.
            let at = |line: usize, k: isize| -> usize {
                if vertical {
                    (segment + line) * stride + (edge as isize + k) as usize - base
                } else {
                    (edge as isize + k) as usize * stride + segment + line - base
                }
            };
            let lines = 4.min(inner - segment);
            let get = |plane: &[T], line: usize, k: isize| plane[at(line, k)].value();
            let line_d = |plane: &[T], line: usize| -> (i32, i32) {
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
            let strong_line = |plane: &[T], line: usize, dpq: i32| -> bool {
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
                let set = |plane: &mut [T], k: isize, value: i32| {
                    plane[at(line, k)] = T::of(value.clamp(0, max));
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

/// The chroma edges of a band, as `deblock_luma`.
fn deblock_chroma<T: Sample, N: Neighbourhood + ?Sized>(
    near: &N,
    plane: &mut [T],
    start: usize,
    component: usize,
    vertical: bool,
    edges: std::ops::Range<usize>,
) {
    let layout = near.layout();
    let (sub_x, sub_y) = (layout.sub_x, layout.sub_y);
    let width = layout.width >> sub_x;
    let height = layout.height >> sub_y;
    let stride = layout.strides[component];
    let bit_depth = layout.sps.bit_depth_chroma;
    let max = (1i32 << bit_depth) - 1;
    let scale = 1 << (bit_depth - 8);
    let picture_offset = if component == 1 {
        layout.pps.cb_qp_offset
    } else {
        layout.pps.cr_qp_offset
    };
    let (outer, inner) = if vertical {
        (width, height)
    } else {
        (height, width)
    };
    let _ = outer;
    let (edge_range, line_range) = if vertical {
        (8..width, edges.start..edges.end.min(inner))
    } else {
        (edges.start.max(8)..edges.end, 0..inner)
    };
    let base = start * stride;
    for edge in edge_range.step_by(8) {
        for line in line_range.clone() {
            let (cx, cy) = if vertical { (edge, line) } else { (line, edge) };
            let (qx, qy) = (cx << sub_x, cy << sub_y);
            let (px, py) = if vertical { (qx - 1, qy) } else { (qx, qy - 1) };
            let q_unit: Unit = near.unit(qx, qy);
            let is_edge = if vertical {
                q_unit.left_edge
            } else {
                q_unit.top_edge
            };
            if !is_edge || !filters_edge(near, px, py, qx, qy) {
                continue;
            }
            let p_unit = near.unit(px, py);
            let q_slice = &near.slices()[near.slice_index(
                (qy >> layout.ctb_log2) * layout.width_ctbs + (qx >> layout.ctb_log2),
            )];
            let qpi = ((i32::from(p_unit.qp) + i32::from(q_unit.qp) + 1) >> 1) + picture_offset;
            let qpc = if layout.chroma == 1 {
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
            let at = |k: isize| -> usize {
                if vertical {
                    line * stride + (edge as isize + k) as usize - base
                } else {
                    (edge as isize + k) as usize * stride + line - base
                }
            };
            let (p0, p1) = (plane[at(-1)].value(), plane[at(-2)].value());
            let (q0, q1) = (plane[at(0)].value(), plane[at(1)].value());
            let delta = ((((q0 - p0) << 2) + p1 - q1 + 4) >> 3).clamp(-tc, tc);
            if !p_unit.unfiltered {
                plane[at(-1)] = T::of((p0 + delta).clamp(0, max));
            }
            if !q_unit.unfiltered {
                plane[at(0)] = T::of((q0 - delta).clamp(0, max));
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
    let mut planes = std::mem::take(&mut decoder.planes);
    {
        let decoder: &Decoder = decoder;
        let rules = SaoRules::of(decoder, decoder.units.iter().any(|unit| unit.unfiltered));
        let ctb = 1usize << decoder.ctb_log2;
        let mut tasks = Vec::new();
        for (component, plane) in planes.iter_mut().enumerate() {
            let shift_y = if component == 0 { 0 } else { decoder.sub_y };
            let rows = decoder.height >> shift_y;
            let band = ctb >> shift_y;
            let stride = decoder.strides[component];
            // Each band's line above and line below as deblocked, before
            // any band takes its offsets.
            let edges: Vec<(Samples, Samples)> = (0..decoder.height_ctbs)
                .map(|ctb_y| {
                    let top = ctb_y * band;
                    let bottom = (top + band).min(rows);
                    let line = |y: Option<usize>| match y {
                        Some(y) if y < rows => plane.slice_of(y * stride, stride),
                        _ => Samples::new(8, 0),
                    };
                    (line(top.checked_sub(1)), line(Some(bottom)))
                })
                .collect();
            for ((ctb_y, piece), (above, below)) in plane
                .band()
                .split(band * stride)
                .into_iter()
                .enumerate()
                .zip(edges)
            {
                tasks.push((component, ctb_y, piece, above, below));
            }
        }
        run_parallel(
            decoder.threads,
            tasks,
            |(component, ctb_y, piece, above, below)| {
                with_band!(piece, band => {
                    sao_band(decoder, &rules, component, ctb_y, band, Sample::slice(&above), Sample::slice(&below))
                })
            },
        );
    }
    decoder.planes = planes;
}

/// What a picture's slices, tiles, and blocks let the offsets do.
struct SaoRules {
    /// Every edge offset may read across every CTB boundary: one slice
    /// (or every slice filtering across), one tile (or filtering across).
    open: bool,
    /// Some samples are left unfiltered (PCM or transquant bypass).
    unfiltered: bool,
}

impl SaoRules {
    fn of<N: Neighbourhood + ?Sized>(near: &N, unfiltered: bool) -> SaoRules {
        let layout = near.layout();
        let independent = near
            .slices()
            .iter()
            .filter(|slice| !slice.dependent)
            .count();
        let slices_open = independent <= 1
            || near
                .slices()
                .iter()
                .all(|slice| slice.loop_filter_across_slices);
        let tiles_open =
            layout.pps.loop_filter_across_tiles || layout.tile_of.iter().all(|&tile| tile == 0);
        SaoRules {
            open: slices_open && tiles_open,
            unfiltered,
        }
    }
}

/// The offsets of one component's band of CTB row `ctb_y`: a window holds
/// the band's deblocked lines with the line above and the line below
/// (copied before any band changed), and the band takes the results.
fn sao_band<T: Sample, N: Neighbourhood + ?Sized>(
    near: &N,
    rules: &SaoRules,
    component: usize,
    ctb_y: usize,
    band: &mut [T],
    above_line: &[T],
    below_line: &[T],
) {
    let layout = near.layout();
    let (shift_x, shift_y) = if component == 0 {
        (0, 0)
    } else {
        (layout.sub_x, layout.sub_y)
    };
    let width = layout.width >> shift_x;
    let height = layout.height >> shift_y;
    let stride = layout.strides[component];
    let bit_depth = if component == 0 {
        layout.sps.bit_depth_luma
    } else {
        layout.sps.bit_depth_chroma
    };
    let max = (1i32 << bit_depth) - 1;
    let ctb = 1usize << layout.ctb_log2;
    let band_height = ctb >> shift_y;
    T::with_scratch((band_height + 2) * stride, |window| {
        let band_top = ctb_y * band_height;
        let band_bottom = (band_top + band_height).min(height);
        // Window line k holds picture line band_top - 1 + k.
        if !above_line.is_empty() {
            window[..stride].copy_from_slice(above_line);
        }
        let rows = band_bottom - band_top;
        window[stride..(1 + rows) * stride].copy_from_slice(&band[..rows * stride]);
        if !below_line.is_empty() {
            window[(1 + rows) * stride..(2 + rows) * stride].copy_from_slice(below_line);
        }
        let plane = band;
        let base = band_top * stride;
        let line = |y: usize| -> &[T] {
            let k = y + 1 - band_top;
            &window[k * stride..(k + 1) * stride]
        };
        for ctb_x in 0..layout.width_ctbs {
            let rs = ctb_y * layout.width_ctbs + ctb_x;
            let slice = &near.slices()[near.slice_index(rs)];
            let enabled = if component == 0 {
                slice.sao_luma
            } else {
                slice.sao_chroma
            };
            let params = near.sao_params(rs)[component];
            if !enabled || params.kind == 0 {
                continue;
            }
            let x0 = (ctb_x * ctb) >> shift_x;
            let x1 = (((ctb_x + 1) * ctb) >> shift_x).min(width);
            if params.kind == 1 {
                let mut table = [0i32; 32];
                for k in 0..4 {
                    table[(k + usize::from(params.band)) & 31] = params.offsets[k + 1];
                }
                let shift = bit_depth - 5;
                for y in band_top..band_bottom {
                    let source = &line(y)[x0..x1];
                    let target = &mut plane[y * stride + x0 - base..y * stride + x1 - base];
                    for (out, &value) in target.iter_mut().zip(source) {
                        let value = value.value();
                        *out = T::of((value + table[(value >> shift) as usize]).clamp(0, max));
                    }
                    if rules.unfiltered {
                        restore_unfiltered(
                            near, plane, source, y, x0, x1, shift_x, shift_y, stride, base,
                        );
                    }
                }
                continue;
            }
            // Edge offsets: neighbours a and b along the class's direction.
            let (dx, dy): ([isize; 2], [isize; 2]) = match params.class {
                0 => ([-1, 1], [0, 0]),
                1 => ([0, 0], [-1, 1]),
                2 => ([-1, 1], [-1, 1]),
                _ => ([1, -1], [-1, 1]),
            };
            // edgeIdx from 2 + the signs' sum, then the offset it selects.
            let offsets = params.offsets;
            let pick = [offsets[1], offsets[2], offsets[0], offsets[3], offsets[4]];
            // Columns whose neighbours are inside the picture.
            let left = if dx[0] != 0 { x0.max(1) } else { x0 };
            let right = if dx[0] != 0 { x1.min(width - 1) } else { x1 };
            for y in band_top..band_bottom {
                if dy[0] != 0 && (y == 0 || y + 1 >= height) {
                    continue;
                }
                let rows = [(y as isize + dy[0]) as usize, (y as isize + dy[1]) as usize];
                // Inside the CTB every neighbour is readable; at its edges
                // the slice and tile rules decide, sample by sample.
                let checked = !rules.open;
                let edge_row = checked && (y == band_top || y + 1 == band_bottom);
                let (inner_left, inner_right) = if checked {
                    ((x0 + 1).max(left), (x1 - 1).min(right))
                } else {
                    (left, right)
                };
                let target = &mut plane[y * stride - base..(y + 1) * stride - base];
                if !edge_row && inner_left < inner_right {
                    let current = &line(y)[inner_left..inner_right];
                    let a = &line(rows[0])[(inner_left as isize + dx[0]) as usize..];
                    let b = &line(rows[1])[(inner_left as isize + dx[1]) as usize..];
                    for (((out, &value), &a), &b) in target[inner_left..inner_right]
                        .iter_mut()
                        .zip(current)
                        .zip(a)
                        .zip(b)
                    {
                        let value = value.value();
                        let index = 2 + (value - a.value()).signum() + (value - b.value()).signum();
                        *out = T::of((value + pick[index as usize]).clamp(0, max));
                    }
                }
                if checked {
                    // The CTB's border samples, each with its own checks.
                    let columns: Vec<usize> = if edge_row {
                        (left..right).collect()
                    } else {
                        [x0, x1 - 1]
                            .into_iter()
                            .filter(|&x| x >= left && x < right)
                            .collect()
                    };
                    for x in columns {
                        let luma = (x << shift_x, y << shift_y);
                        let mut readable = true;
                        for k in 0..2 {
                            let neighbour = (
                                ((x as isize + dx[k]) as usize) << shift_x,
                                rows[k] << shift_y,
                            );
                            readable &= sao_crosses(near, luma, neighbour);
                        }
                        if !readable {
                            target[x] = line(y)[x];
                            continue;
                        }
                        let value = line(y)[x].value();
                        let a = line(rows[0])[(x as isize + dx[0]) as usize].value();
                        let b = line(rows[1])[(x as isize + dx[1]) as usize].value();
                        let index = 2 + (value - a).signum() + (value - b).signum();
                        target[x] = T::of((value + pick[index as usize]).clamp(0, max));
                    }
                }
                if rules.unfiltered {
                    restore_unfiltered(
                        near,
                        plane,
                        &line(y)[x0..x1],
                        y,
                        x0,
                        x1,
                        shift_x,
                        shift_y,
                        stride,
                        base,
                    );
                }
            }
        }
    });
}

/// Puts back the deblocked samples of PCM and bypass blocks the loop
/// filters leave alone.
#[allow(clippy::too_many_arguments)]
fn restore_unfiltered<T: Sample, N: Neighbourhood + ?Sized>(
    near: &N,
    plane: &mut [T],
    source: &[T],
    y: usize,
    x0: usize,
    x1: usize,
    shift_x: u32,
    shift_y: u32,
    stride: usize,
    base: usize,
) {
    for x in x0..x1 {
        if near.unit(x << shift_x, y << shift_y).unfiltered {
            plane[y * stride + x - base] = source[x - x0];
        }
    }
}

/// Whether an edge offset may read the sample at `neighbour` for the one at
/// `current` (luma positions): the slice and tile rules of 8.7.3.
fn sao_crosses<N: Neighbourhood + ?Sized>(
    near: &N,
    current: (usize, usize),
    neighbour: (usize, usize),
) -> bool {
    let layout = near.layout();
    let ctb_of = |(x, y): (usize, usize)| {
        (y >> layout.ctb_log2) * layout.width_ctbs + (x >> layout.ctb_log2)
    };
    let (a, b) = (ctb_of(current), ctb_of(neighbour));
    if a == b {
        return true;
    }
    if slice_start(near, a) != slice_start(near, b) {
        let current_first = layout.rs_to_ts[a] < layout.rs_to_ts[b];
        let flag_holder = if current_first { b } else { a };
        if !near.slices()[near.slice_index(flag_holder)].loop_filter_across_slices {
            return false;
        }
    }
    if !layout.pps.loop_filter_across_tiles && layout.tile_of[a] != layout.tile_of[b] {
        return false;
    }
    true
}
