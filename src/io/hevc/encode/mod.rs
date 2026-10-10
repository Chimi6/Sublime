//! HEVC intra picture encoding, for HEIF images: one IDR picture of
//! Main or Main 10 4:2:0, in one slice of wavefront rows, its coding
//! decided by rate and distortion (`search`), written with CABAC
//! (`cabac`, `syntax`) around the parameter sets and slice header
//! (`bitstream`). The decoder's own prediction, scans, and inverse
//! transforms make the reconstruction, so what the encoder predicts
//! from is what every decoder will have.

mod aq;
mod bitstream;
mod cabac;
mod quant;
mod rdoq;
mod search;
mod syntax;

use super::cabac::{Contexts, SPLIT_CU};
use cabac::{Coder, Encoder};
use search::{CtbDecisions, Picture, Plane, Rd, Units};

/// How a picture is coded.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The picture's size, and its coded size (whole 8x8 blocks).
    pub width: usize,
    pub height: usize,
    pub coded_width: usize,
    pub coded_height: usize,
    pub bit_depth: u32,
    pub ctb_log2: u32,
    /// `max_transform_hierarchy_depth_intra`.
    pub transform_depth: u32,
    pub qp: i32,
    pub chroma_qp_offset: i32,
    pub sao: bool,
    pub deblocking: bool,
    /// `beta_offset_div2` and `tc_offset_div2`.
    pub deblocking_offsets: (i32, i32),
    pub sign_hiding: bool,
    pub transform_skip: bool,
    /// `diff_cu_qp_delta_depth`, when the QP changes within the picture.
    pub qp_delta_depth: Option<u32>,
    /// How far the QP moves with a CTB's detail (`aq`), 0 for not at all.
    pub aq_strength: f64,
    pub full_range: bool,
    /// Colour primaries, transfer characteristics, matrix coefficients.
    pub colour: (u8, u8, u8),
}

impl Settings {
    /// The settings for a picture of `width` by `height` at `qp`.
    pub fn new(width: usize, height: usize, bit_depth: u32, qp: i32) -> Settings {
        // 32x32 CTBs code smooth areas far better (9% to 24% smaller on
        // large photographs than 16x16), but their wavefront runs each
        // row two CTBs behind the one above: a picture whose 32x32 CTBs
        // shared among eight workers number fewer than that diagonal's
        // length (W/32 + 2H/32) would leave the workers waiting, so it
        // codes with 16x16 CTBs instead (a Kodak photograph, 768 by 512:
        // 0.1% to 1.1% larger, by the measure, in two thirds of the time). By the
        // picture alone, so a file is the same on every machine.
        let small = width * height < 256 * (width + 2 * height);
        Settings {
            width,
            height,
            coded_width: width.div_ceil(8) * 8,
            coded_height: height.div_ceil(8) * 8,
            bit_depth,
            ctb_log2: if small { 4 } else { 5 },
            transform_depth: 1,
            qp,
            chroma_qp_offset: 0,
            sao: false,
            deblocking: true,
            // A little lighter than the standard's: measured kinder to
            // detail (0.2% at equal SSIM and RGB PSNR on Kodak).
            deblocking_offsets: (-1, -1),
            sign_hiding: true,
            transform_skip: true,
            qp_delta_depth: Some(0),
            // Measured on Kodak against x265 at each CTB size: weaker
            // with 16x16 groups, which follow detail more closely.
            aq_strength: if small { 0.7 } else { 0.95 },
            full_range: true,
            colour: (1, 13, 6),
        }
    }

    fn ctbs(&self) -> (usize, usize) {
        let size = 1usize << self.ctb_log2;
        (
            self.coded_width.div_ceil(size),
            self.coded_height.div_ceil(size),
        )
    }
}

/// The encoded picture: its parameter sets and its slice, each a NAL
/// unit without start code or length; and the reconstruction a decoder
/// makes before its loop filters.
pub struct Encoded {
    pub vps: Vec<u8>,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
    pub slice: Vec<u8>,
    pub recon: [Vec<u16>; 3],
}

impl Encoded {
    /// The `hvcC` record of the parameter sets.
    pub fn hvcc(&self, settings: &Settings) -> Vec<u8> {
        bitstream::hvcc(settings, &self.vps, &self.sps, &self.pps)
    }
}

/// What a row tells the row below as it goes: each CTB's last line of
/// samples and units, and its contexts after its second CTB.
enum RowUpdate {
    Ctb {
        ctb_x: usize,
        lines: [Vec<u16>; 3],
        depth: Vec<u8>,
    },
    Contexts(Box<Contexts>),
}

/// A coded row: its substream, and its band of the reconstruction when
/// kept.
type RowDone = (usize, Vec<u8>, Option<[Vec<u16>; 3]>);

/// A CTB row being coded: its band, its CABAC state and substream, and
/// what it hears from the row above and tells the row below.
struct Row<'p> {
    picture: Picture<'p>,
    contexts: Contexts,
    encoder: Encoder,
    qp: search::QpState,
    above: Option<std::sync::mpsc::Receiver<RowUpdate>>,
    below: Option<std::sync::mpsc::Sender<RowUpdate>>,
    /// CTBs of the row above heard of, and its contexts once sent.
    heard: usize,
    synced: Option<Contexts>,
}

impl Row<'_> {
    /// Takes in what the row above has sent (all of it is there: a row
    /// runs only once the row above has gone far enough).
    fn hear(&mut self, size: usize) {
        let Some(above) = &self.above else {
            return;
        };
        while let Ok(update) = above.try_recv() {
            match update {
                RowUpdate::Ctb {
                    ctb_x,
                    lines,
                    depth,
                } => {
                    let edge = self
                        .picture
                        .above
                        .as_mut()
                        .expect("an edge with a row above");
                    for (component, line) in lines.iter().enumerate() {
                        let start = (ctb_x * size) >> usize::from(component > 0);
                        edge.lines[component][start..start + line.len()].copy_from_slice(line);
                    }
                    let start = ctb_x * size / 4;
                    edge.depth[start..start + depth.len()].copy_from_slice(&depth);
                    self.heard = ctb_x + 1;
                }
                RowUpdate::Contexts(contexts) => self.synced = Some(*contexts),
            }
        }
    }
}

/// Encodes a 4:2:0 picture given as its coded-size planes (luma, then
/// Cb and Cr at half size), the padding past the picture's edges filled.
/// CTB rows code on every thread, each two CTBs behind the row above, as
/// wavefront substreams.
pub fn encode(settings: &Settings, planes: [Vec<u16>; 3]) -> Encoded {
    encode_with(settings, planes, false)
}

/// `encode`, keeping the reconstruction (for tests).
pub fn encode_keeping(settings: &Settings, planes: [Vec<u16>; 3]) -> Encoded {
    encode_with(settings, planes, true)
}

/// The picture's rows and what the workers share while coding them.
struct Wavefront<'p> {
    settings: &'p Settings,
    source: &'p [Plane; 3],
    qp_map: &'p aq::QpMap,
    rd: &'p Rd,
    wide: usize,
    high: usize,
    keep: bool,
    /// Each row's link from the row above and to the row below, taken
    /// when the row starts.
    links: Vec<std::sync::Mutex<RowLinks>>,
    /// Each row's CTBs done, and its state while it is being coded.
    progress: Vec<std::sync::atomic::AtomicUsize>,
    rows: Vec<std::sync::Mutex<Option<Row<'p>>>>,
    done: std::sync::Mutex<Vec<RowDone>>,
    /// Bumped whenever a row goes on, for the workers waiting for one.
    changed: (std::sync::Mutex<u64>, std::sync::Condvar),
}

type RowLinks = (
    Option<std::sync::mpsc::Receiver<RowUpdate>>,
    Option<std::sync::mpsc::Sender<RowUpdate>>,
);

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

impl<'p> Wavefront<'p> {
    fn size(&self) -> usize {
        1usize << self.settings.ctb_log2
    }

    fn new_row(&self, ctb_y: usize) -> Row<'p> {
        let settings = self.settings;
        let (cw, ch) = (settings.coded_width, settings.coded_height);
        let size = self.size();
        let top = ctb_y * size;
        let rows = size.min(ch - top);
        let (above, below) = std::mem::take(&mut *lock(&self.links[ctb_y]));
        let units = (cw / 4) * rows.div_ceil(4);
        let picture = Picture {
            settings,
            source: self.source,
            qp_map: self.qp_map,
            top,
            recon: [
                Plane::new(cw, rows),
                Plane::new(cw / 2, rows / 2),
                Plane::new(cw / 2, rows / 2),
            ],
            units: Units {
                depth: vec![0; units],
                mode: vec![1; units],
                qp: vec![settings.qp as i8; units],
                wide: cw / 4,
            },
            above: above.as_ref().map(|_| search::Edge::new(settings)),
            scratch: search::Scratch::default(),
            qp: settings.qp,
            level: self.rd.level(settings.qp),
            block_contexts: None,
            rates: None,
            skip_trials: true,
            neighbours: None,
        };
        let mut row = Row {
            picture,
            contexts: Contexts::new(settings.qp),
            encoder: Encoder::new(Vec::new()),
            // The QP each group's delta is predicted from: the slice's at
            // a row's start, then the last CU's (8.6.1, wavefront rows).
            qp: search::QpState::new(settings.qp_delta_depth.is_some(), settings.qp),
            above,
            below,
            heard: 0,
            synced: None,
        };
        // The row's contexts: the row above's after its second CTB.
        row.hear(size);
        if row.above.is_some() && self.wide > 1 {
            row.contexts = row
                .synced
                .take()
                .unwrap_or_else(|| Contexts::new(settings.qp));
        }
        row
    }

    /// Whether `row`'s next CTB may start: the row above has finished the
    /// CTB above it and the one above-right (and, to start, its second).
    fn runnable(&self, row: usize) -> bool {
        use std::sync::atomic::Ordering;
        let next = self.progress[row].load(Ordering::Acquire);
        next < self.wide
            && (row == 0
                || self.progress[row - 1].load(Ordering::Acquire) >= (next + 2).min(self.wide))
    }

    /// Codes CTBs of `row` while it can go on, and its end.
    fn run(&self, row: usize, state: &mut Option<Row<'p>>) {
        use std::sync::atomic::Ordering;
        let (wide, high, size) = (self.wide, self.high, self.size());
        // Another worker may have finished the row since it looked runnable.
        if !self.runnable(row) {
            return;
        }
        let row_state = state.get_or_insert_with(|| self.new_row(row));
        while self.runnable(row) {
            let ctb_x = self.progress[row].load(Ordering::Acquire);
            row_state.hear(size);
            let Row {
                picture,
                contexts,
                encoder,
                qp,
                below,
                ..
            } = row_state;
            let decisions = picture.search_ctb(self.rd, contexts, ctb_x, row);
            picture.write_ctb(encoder, contexts, &decisions, ctb_x, row, qp);
            if let Some(below) = below {
                let _ = below.send(picture.bottom_of(ctb_x));
                if ctb_x == 1 {
                    let _ = below.send(RowUpdate::Contexts(Box::new(contexts.clone())));
                }
            }
            // end_of_slice_segment_flag, and end_of_subset_one_bit at a
            // row's end.
            let last = ctb_x + 1 == wide && row + 1 == high;
            encoder.terminate(u32::from(last));
            if !last && ctb_x + 1 == wide {
                encoder.terminate(1);
            }
            self.progress[row].store(ctb_x + 1, Ordering::Release);
            // The row below may go on now: wake whoever waits for work.
            *lock(&self.changed.0) += 1;
            self.changed.1.notify_all();
        }
        if self.progress[row].load(Ordering::Acquire) == wide {
            let mut finished = state.take().expect("a row being coded");
            let recon = self.keep.then(|| {
                let [luma, cb, cr] = std::mem::replace(
                    &mut finished.picture.recon,
                    [Plane::new(0, 0), Plane::new(0, 0), Plane::new(0, 0)],
                );
                [luma.samples, cb.samples, cr.samples]
            });
            lock(&self.done).push((row, finished.encoder.finish(), recon));
        }
    }

    /// A worker: takes any row that can go on, the earliest first, so none
    /// waits on a row above while another row could run.
    fn work(&self) {
        use std::sync::atomic::Ordering;
        loop {
            let seen = *lock(&self.changed.0);
            if self
                .progress
                .iter()
                .all(|count| count.load(Ordering::Acquire) == self.wide)
            {
                break;
            }
            let mut ran = false;
            for row in 0..self.high {
                if !self.runnable(row) {
                    continue;
                }
                let Ok(mut state) = self.rows[row].try_lock() else {
                    continue;
                };
                self.run(row, &mut state);
                drop(state);
                ran = true;
                *lock(&self.changed.0) += 1;
                self.changed.1.notify_all();
                break;
            }
            if !ran {
                let generation = lock(&self.changed.0);
                if *generation == seen {
                    let _ = self
                        .changed
                        .1
                        .wait_timeout(generation, std::time::Duration::from_millis(5));
                }
            }
        }
    }
}

fn encode_with(settings: &Settings, planes: [Vec<u16>; 3], keep: bool) -> Encoded {
    let (cw, ch) = (settings.coded_width, settings.coded_height);
    let [luma, cb, cr] = planes;
    let source = [
        Plane {
            samples: luma,
            width: cw,
            height: ch,
        },
        Plane {
            samples: cb,
            width: cw / 2,
            height: ch / 2,
        },
        Plane {
            samples: cr,
            width: cw / 2,
            height: ch / 2,
        },
    ];
    let rd = Rd::new(settings);
    let (wide, high) = settings.ctbs();
    let qp_map = aq::qp_map(settings, &source);
    // Row r tells row r + 1.
    let mut links: Vec<RowLinks> = (0..high).map(|_| (None, None)).collect();
    for row in 1..high {
        let (sender, receiver) = std::sync::mpsc::channel();
        links[row - 1].1 = Some(sender);
        links[row].0 = Some(receiver);
    }
    let wavefront = Wavefront {
        settings,
        source: &source,
        qp_map: &qp_map,
        rd: &rd,
        wide,
        high,
        keep,
        links: links.into_iter().map(std::sync::Mutex::new).collect(),
        progress: (0..high)
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect(),
        rows: (0..high).map(|_| std::sync::Mutex::new(None)).collect(),
        done: std::sync::Mutex::new(Vec::with_capacity(high)),
        changed: (std::sync::Mutex::new(0), std::sync::Condvar::new()),
    };
    // The calling thread is one of the workers; WebAssembly has no others.
    let threads = if cfg!(target_family = "wasm") {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(1, |count| count.get())
            .min(high)
    };
    std::thread::scope(|scope| {
        for _ in 1..threads {
            scope.spawn(|| wavefront.work());
        }
        wavefront.work();
    });
    let mut finished = wavefront
        .done
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner());
    finished.sort_by_key(|row| row.0);
    let mut recon = [Vec::new(), Vec::new(), Vec::new()];
    let mut substreams = Vec::with_capacity(high);
    for (_, substream, band) in finished {
        substreams.push(substream);
        if let Some(band) = band {
            for (whole, part) in recon.iter_mut().zip(band) {
                whole.extend_from_slice(&part);
            }
        }
    }
    Encoded {
        vps: bitstream::vps(settings),
        sps: bitstream::sps(settings),
        pps: bitstream::pps(settings),
        slice: bitstream::slice(settings, &substreams),
        recon,
    }
}

impl Picture<'_> {
    /// `coding_quadtree()` of a CTB from its decisions.
    fn write_ctb<C: Coder>(
        &mut self,
        coder: &mut C,
        contexts: &mut Contexts,
        decisions: &CtbDecisions,
        ctb_x: usize,
        ctb_y: usize,
        qp: &mut search::QpState,
    ) {
        let log2 = self.settings.ctb_log2;
        let mut splits = decisions.splits.iter();
        let mut cus = decisions.cus.iter();
        self.write_quadtree(
            coder,
            contexts,
            &mut splits,
            &mut cus,
            ctb_x << log2,
            ctb_y << log2,
            log2,
            0,
            qp,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn write_quadtree<'d, C: Coder>(
        &mut self,
        coder: &mut C,
        contexts: &mut Contexts,
        splits: &mut impl Iterator<Item = &'d bool>,
        cus: &mut impl Iterator<Item = &'d search::Cu>,
        x: usize,
        y: usize,
        log2: u32,
        depth: u32,
        qp: &mut search::QpState,
    ) {
        let size = 1usize << log2;
        if log2 >= self.qp_map.log2 {
            qp.coded = false;
        }
        let inside =
            x + size <= self.settings.coded_width && y + size <= self.settings.coded_height;
        let split = if inside && log2 > 3 {
            let split = *splits.next().expect("a split flag for each node");
            let increment = self.split_context(x, y, depth);
            coder.decision(
                &mut contexts.contexts[SPLIT_CU + increment],
                u32::from(split),
            );
            split
        } else {
            log2 > 3
        };
        if split {
            let half = size / 2;
            for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
                if x + dx < self.settings.coded_width && y + dy < self.settings.coded_height {
                    self.write_quadtree(
                        coder,
                        contexts,
                        splits,
                        cus,
                        x + dx,
                        y + dy,
                        log2 - 1,
                        depth + 1,
                        qp,
                    );
                }
            }
            return;
        }
        let cu = cus.next().expect("a CU for each leaf");
        self.write_cu(coder, contexts, cu, qp);
        if qp.enabled {
            self.mark_qp(cu, qp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test picture: gradients, edges, and noise, at a size off the
    /// CTB grid.
    fn picture(width: usize, height: usize, bit_depth: u32) -> [Vec<u16>; 3] {
        let (cw, ch) = (width.div_ceil(8) * 8, height.div_ceil(8) * 8);
        let max = (1u32 << bit_depth) - 1;
        let mut seed = 12345u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) % 32
        };
        let mut plane = |w: usize, h: usize, phase: u32| -> Vec<u16> {
            let mut samples = vec![0u16; w * h];
            for y in 0..h {
                for x in 0..w {
                    let gradient = (x as u32 * 3 + y as u32 * 2 + phase) % 200;
                    let edge = if (x / 13 + y / 7) % 3 == 0 { 40 } else { 0 };
                    let value = (gradient + edge + noise()) << (bit_depth - 8);
                    samples[y * w + x] = value.min(max) as u16;
                }
            }
            samples
        };
        [
            plane(cw, ch, 0),
            plane(cw / 2, ch / 2, 60),
            plane(cw / 2, ch / 2, 120),
        ]
    }

    fn round_trip(width: usize, height: usize, bit_depth: u32, qp: i32, deblocking: bool) {
        for ctb_log2 in [4, 5] {
            for group_depth in [0, 1] {
                round_trip_with(
                    width,
                    height,
                    bit_depth,
                    qp,
                    deblocking,
                    (ctb_log2, group_depth),
                );
            }
        }
    }

    fn round_trip_with(
        width: usize,
        height: usize,
        bit_depth: u32,
        qp: i32,
        deblocking: bool,
        (ctb_log2, group_depth): (u32, u32),
    ) {
        let mut settings = Settings::new(width, height, bit_depth, qp);
        settings.ctb_log2 = ctb_log2;
        settings.deblocking = deblocking;
        settings.qp_delta_depth = Some(group_depth);
        settings.aq_strength = 2.0;
        let encoded = encode_keeping(&settings, picture(width, height, bit_depth));
        let units = [
            &encoded.vps[..],
            &encoded.sps[..],
            &encoded.pps[..],
            &encoded.slice[..],
        ];
        let decoded = super::super::decode_picture(units).expect("decodes");
        assert_eq!(
            (decoded.width, decoded.height),
            (settings.coded_width, settings.coded_height)
        );
        if deblocking {
            return;
        }
        for component in 0..3 {
            let plane = &decoded.planes[component];
            let recon = &encoded.recon[component];
            assert_eq!(plane.len(), recon.len());
            for (index, &value) in recon.iter().enumerate() {
                assert_eq!(
                    plane.get(index),
                    value,
                    "{width}x{height} {bit_depth}-bit qp {qp}: component {component} at {index}"
                );
            }
        }
    }

    #[test]
    fn decodes_to_the_encoders_reconstruction() {
        for (width, height, bit_depth, qp) in [
            (64, 64, 8, 30),
            (100, 76, 8, 22),
            (37, 29, 8, 40),
            (96, 40, 10, 30),
            (200, 136, 8, 12),
        ] {
            round_trip(width, height, bit_depth, qp, false);
            round_trip(width, height, bit_depth, qp, true);
        }
    }

    #[test]
    fn the_same_picture_codes_the_same_under_contention() {
        // Encoders racing for the cores: the rows' workers interleave
        // differently each time, and the stream must not change.
        let settings = Settings::new(160, 256, 8, 30);
        let first = encode(&settings, picture(160, 256, 8)).slice;
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..6 {
                        let again = encode(&settings, picture(160, 256, 8)).slice;
                        assert!(again == first, "a different stream");
                    }
                });
            }
        });
    }

    #[test]
    fn small_pictures_code_with_smaller_ctbs() {
        for (width, height, ctb_log2) in [
            (768, 512, 4),
            (512, 768, 4),
            (1, 1, 4),
            (1024, 768, 5),
            (4032, 3024, 5),
            (12000, 40, 4),
        ] {
            let settings = Settings::new(width, height, 8, 30);
            assert_eq!(settings.ctb_log2, ctb_log2, "{width}x{height}");
        }
    }
}
