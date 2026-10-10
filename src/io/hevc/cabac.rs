//! CABAC (H.265 9.3): the arithmetic decoding engine and the context
//! variables of intra slices. The engine is the branch-free form H.264
//! decoders use: the offset held with eight bits below it and a marker bit
//! that says when to read the next byte, each decision's LPS case taken by
//! masks rather than a branch, and a context's state and MPS in one byte
//! whose next value, for either outcome, comes from one table.

/// A context variable: its probability state times two plus its most
/// probable symbol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    pub(super) packed: u8,
}

impl Context {
    /// Initialized from its `initValue` at the slice's QP (9.3.2.2).
    fn new(init: u8, qp: i32) -> Context {
        let slope = i32::from(init >> 4) * 5 - 45;
        let offset = (i32::from(init & 15) << 3) - 16;
        let state = (((slope * qp.clamp(0, 51)) >> 4) + offset).clamp(1, 126);
        if state <= 63 {
            Context::of((63 - state) as u8, 0)
        } else {
            Context::of((state - 64) as u8, 1)
        }
    }

    fn of(state: u8, mps: u8) -> Context {
        Context {
            packed: (state << 1) | mps,
        }
    }
}

// Context layout: the first context of each syntax element.
pub const SAO_MERGE: usize = 0;
pub const SAO_TYPE: usize = 1;
pub const SPLIT_CU: usize = 2;
pub const TRANSQUANT_BYPASS: usize = 5;
pub const PART_MODE: usize = 6;
pub const PREV_INTRA_LUMA: usize = 7;
pub const INTRA_CHROMA: usize = 8;
pub const SPLIT_TRANSFORM: usize = 9;
pub const CBF_LUMA: usize = 12;
pub const CBF_CHROMA: usize = 14;
pub const CU_QP_DELTA: usize = 19;
pub const CHROMA_QP_OFFSET_FLAG: usize = 21;
pub const CHROMA_QP_OFFSET_IDX: usize = 22;
pub const TRANSFORM_SKIP: usize = 23;
pub const LAST_X: usize = 25;
pub const LAST_Y: usize = 43;
pub const CODED_SUB_BLOCK: usize = 61;
pub const SIG_COEFF: usize = 65;
pub const GREATER1: usize = 109;
pub const GREATER2: usize = 133;
pub const LOG2_RES_SCALE: usize = 139;
pub const RES_SCALE_SIGN: usize = 147;
pub const CONTEXTS: usize = 149;

/// Every context's `initValue` for an intra slice (initType 0), in the
/// layout above (Tables 9-5 to 9-37).
const INIT_VALUES: [u8; CONTEXTS] = {
    let mut values = [154u8; CONTEXTS];
    values[SAO_MERGE] = 153;
    values[SAO_TYPE] = 200;
    values[SPLIT_CU] = 139;
    values[SPLIT_CU + 1] = 141;
    values[SPLIT_CU + 2] = 157;
    values[TRANSQUANT_BYPASS] = 154;
    values[PART_MODE] = 184;
    values[PREV_INTRA_LUMA] = 184;
    values[INTRA_CHROMA] = 63;
    values[SPLIT_TRANSFORM] = 153;
    values[SPLIT_TRANSFORM + 1] = 138;
    values[SPLIT_TRANSFORM + 2] = 138;
    values[CBF_LUMA] = 111;
    values[CBF_LUMA + 1] = 141;
    let chroma = [94, 138, 182, 154, 154];
    let mut index = 0;
    while index < 5 {
        values[CBF_CHROMA + index] = chroma[index];
        index += 1;
    }
    values[TRANSFORM_SKIP] = 139;
    values[TRANSFORM_SKIP + 1] = 139;
    let last = [
        110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63,
    ];
    index = 0;
    while index < 18 {
        values[LAST_X + index] = last[index];
        values[LAST_Y + index] = last[index];
        index += 1;
    }
    let coded = [91, 171, 134, 141];
    index = 0;
    while index < 4 {
        values[CODED_SUB_BLOCK + index] = coded[index];
        index += 1;
    }
    let significant = [
        111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141, 179, 153, 125, 107, 125, 141,
        179, 153, 125, 107, 125, 141, 179, 153, 125, 140, 139, 182, 182, 152, 136, 152, 136, 153,
        136, 139, 111, 136, 139, 111, 141, 111,
    ];
    index = 0;
    while index < 44 {
        values[SIG_COEFF + index] = significant[index];
        index += 1;
    }
    let greater1 = [
        140, 92, 137, 138, 140, 152, 138, 139, 153, 74, 149, 92, 139, 107, 122, 152, 140, 179, 166,
        182, 140, 227, 122, 197,
    ];
    index = 0;
    while index < 24 {
        values[GREATER1 + index] = greater1[index];
        index += 1;
    }
    let greater2 = [138, 153, 136, 167, 152, 152];
    index = 0;
    while index < 6 {
        values[GREATER2 + index] = greater2[index];
        index += 1;
    }
    values
};

/// The context variables of a slice, with the Rice statistics of
/// persistent Rice adaptation (which are stored and restored with them).
#[derive(Clone)]
pub struct Contexts {
    pub contexts: [Context; CONTEXTS],
    pub stat_coeff: [u8; 4],
}

impl Contexts {
    pub fn new(qp: i32) -> Contexts {
        Contexts {
            contexts: std::array::from_fn(|index| Context::new(INIT_VALUES[index], qp)),
            stat_coeff: [0; 4],
        }
    }
}

pub(super) const LPS_RANGE: [[u8; 4]; 64] = [
    [128, 176, 208, 240],
    [128, 167, 197, 227],
    [128, 158, 187, 216],
    [123, 150, 178, 205],
    [116, 142, 169, 195],
    [111, 135, 160, 185],
    [105, 128, 152, 175],
    [100, 122, 144, 166],
    [95, 116, 137, 158],
    [90, 110, 130, 150],
    [85, 104, 123, 142],
    [81, 99, 117, 135],
    [77, 94, 111, 128],
    [73, 89, 105, 122],
    [69, 85, 100, 116],
    [66, 80, 95, 110],
    [62, 76, 90, 104],
    [59, 72, 86, 99],
    [56, 69, 81, 94],
    [53, 65, 77, 89],
    [51, 62, 73, 85],
    [48, 59, 69, 80],
    [46, 56, 66, 76],
    [43, 53, 63, 72],
    [41, 50, 59, 69],
    [39, 48, 56, 65],
    [37, 45, 54, 62],
    [35, 43, 51, 59],
    [33, 41, 48, 56],
    [32, 39, 46, 53],
    [30, 37, 43, 50],
    [29, 35, 41, 48],
    [27, 33, 39, 45],
    [26, 31, 37, 43],
    [24, 30, 35, 41],
    [23, 28, 33, 39],
    [22, 27, 32, 37],
    [21, 26, 30, 35],
    [20, 24, 29, 33],
    [19, 23, 27, 31],
    [18, 22, 26, 30],
    [17, 21, 25, 28],
    [16, 20, 23, 27],
    [15, 19, 22, 25],
    [14, 18, 21, 24],
    [14, 17, 20, 23],
    [13, 16, 19, 22],
    [12, 15, 18, 21],
    [12, 14, 17, 20],
    [11, 14, 16, 19],
    [11, 13, 15, 18],
    [10, 12, 15, 17],
    [10, 12, 14, 16],
    [9, 11, 13, 15],
    [9, 11, 12, 14],
    [8, 10, 12, 14],
    [8, 9, 11, 13],
    [7, 9, 11, 12],
    [7, 9, 10, 12],
    [7, 8, 10, 11],
    [6, 8, 9, 11],
    [6, 7, 9, 10],
    [6, 7, 8, 9],
    [2, 2, 2, 2],
];

const NEXT_STATE_LPS: [u8; 64] = [
    0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12, 13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21,
    21, 22, 22, 23, 24, 24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33, 33, 33, 34,
    34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];

/// A decision's LPS range by `2 * (range & 0xC0) + packed state`.
const LPS_TABLE: [u8; 512] = {
    let mut table = [0u8; 512];
    let mut quarter = 0;
    while quarter < 4 {
        let mut packed = 0;
        while packed < 128 {
            table[quarter * 128 + packed] = LPS_RANGE[packed >> 1][quarter];
            packed += 1;
        }
        quarter += 1;
    }
    table
};

/// The shift that brings a value below 512 to at least 256 (9 for zero).
const NORM_SHIFT: [u8; 512] = {
    let mut table = [0u8; 512];
    let mut value = 0;
    while value < 512 {
        let mut length = 0;
        while length < 10 && (value >> length) != 0 {
            length += 1;
        }
        table[value] = (9 - length) as u8;
        value += 1;
    }
    table
};

/// A packed state's next value, at `128 + packed` after its MPS and at
/// `127 - packed` after its LPS (the packed state's complement).
pub(super) const NEXT_STATE: [u8; 256] = {
    let mut table = [0u8; 256];
    let mut packed = 0;
    while packed < 128 {
        let (state, mps) = (packed >> 1, (packed & 1) as u8);
        let after_mps = if state < 62 { state + 1 } else { state };
        table[128 + packed] = ((after_mps as u8) << 1) | mps;
        table[127 - packed] = if state == 0 {
            (NEXT_STATE_LPS[0] << 1) | (1 - mps)
        } else {
            (NEXT_STATE_LPS[state] << 1) | mps
        };
        packed += 1;
    }
    table
};

/// Bits below the offset's integer part: the next byte goes in when they
/// run out.
const LOW_BITS: i32 = 8;
const LOW_MASK: i32 = (1 << LOW_BITS) - 1;

/// The arithmetic decoder over a slice segment's data.
pub struct Cabac<'a> {
    data: &'a [u8],
    /// The next byte to read into `low`.
    position: usize,
    range: i32,
    /// The offset times 2^9, with the bits read ahead below it and a
    /// marker bit after them.
    low: i32,
}

impl<'a> Cabac<'a> {
    /// Starts decoding at `position` (9.3.2.5).
    pub fn new(data: &'a [u8], position: usize) -> Cabac<'a> {
        let mut cabac = Cabac {
            data,
            position,
            range: 0x1FE,
            low: 0,
        };
        cabac.start();
        cabac
    }

    #[inline(always)]
    fn byte(&self, at: usize) -> i32 {
        i32::from(self.data.get(at).copied().unwrap_or(0))
    }

    fn start(&mut self) {
        self.range = 0x1FE;
        self.low = (self.byte(self.position) << 10) + (self.byte(self.position + 1) << 2) + 2;
        self.position += 2;
    }

    /// Restarts at the next byte, after a terminating bin of 1: a new
    /// substream, or the bins after PCM samples.
    pub fn restart(&mut self) {
        self.position = self.byte_position();
        self.start();
    }

    /// The slice segment's data.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The next byte to read after a terminating bin of 1: where PCM
    /// samples and the next substream begin.
    pub fn byte_position(&self) -> usize {
        // A byte read ahead into `low` and not yet used is given back.
        if self.low & 1 != 0 {
            self.position - 1
        } else {
            self.position
        }
    }

    /// Restarts at `position` (after PCM samples).
    pub fn restart_at(&mut self, position: usize) {
        self.position = position;
        self.start();
    }

    /// The next byte, when the bits below the offset run out.
    #[inline(always)]
    fn refill(&mut self) {
        self.low += (self.byte(self.position) << 1) - LOW_MASK;
        self.position += 1;
    }

    /// The next byte after a shift of several bits: placed under the
    /// bits still read ahead.
    #[inline(always)]
    fn refill_after_shift(&mut self) {
        let x = self.low ^ (self.low - 1);
        let shift = 7 - i32::from(NORM_SHIFT[(x >> (LOW_BITS - 1)) as usize]);
        let byte = (self.byte(self.position) << 1) - LOW_MASK;
        self.low += byte << shift;
        self.position += 1;
    }

    /// A context-coded bin (9.3.4.3.2), without a branch on its outcome.
    #[inline(always)]
    pub fn decision(&mut self, context: &mut Context) -> u32 {
        let packed = i32::from(context.packed);
        let lps = i32::from(LPS_TABLE[(2 * (self.range & 0xC0) + packed) as usize]);
        self.range -= lps;
        // All ones when the offset is past the MPS range: the LPS.
        let mask = ((self.range << (LOW_BITS + 1)) - self.low) >> 31;
        self.low -= (self.range << (LOW_BITS + 1)) & mask;
        self.range += (lps - self.range) & mask;
        let outcome = packed ^ mask;
        context.packed = NEXT_STATE[(128 + outcome) as usize];
        let shift = i32::from(NORM_SHIFT[self.range as usize]);
        self.range <<= shift;
        self.low <<= shift;
        if self.low & LOW_MASK == 0 {
            self.refill_after_shift();
        }
        (outcome & 1) as u32
    }

    /// A bypass bin (9.3.4.3.4).
    #[inline(always)]
    pub fn bypass(&mut self) -> u32 {
        // Wrapping: a damaged stream's offset can start past its range.
        self.low = self.low.wrapping_add(self.low);
        if self.low & LOW_MASK == 0 {
            self.refill();
        }
        let scaled = self.range << (LOW_BITS + 1);
        if self.low < scaled {
            0
        } else {
            self.low -= scaled;
            1
        }
    }

    /// `count` bypass bins as a number, first bin most significant.
    pub fn bypass_bits(&mut self, count: u32) -> u32 {
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1) | self.bypass();
        }
        value
    }

    /// A terminating bin (9.3.4.3.5).
    pub fn terminate(&mut self) -> u32 {
        self.range -= 2;
        if self.low < self.range << (LOW_BITS + 1) {
            if self.range < 0x100 {
                self.range <<= 1;
                self.low <<= 1;
                if self.low & LOW_MASK == 0 {
                    self.refill();
                }
            }
            0
        } else {
            1
        }
    }

    /// The alignment before aligned bypass bins (9.3.4.3.6).
    pub fn align_bypass(&mut self) {
        self.range = 256;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contexts_start_from_their_init_values() {
        // 139 at QP 26: m = -5, n = 72: preCtxState 63, MPS 0, state 0.
        assert_eq!(Context::new(139, 26), Context::of(0, 0));
        // 154 at QP 26: m = 0, n = 64: preCtxState 64, MPS 1, state 0.
        assert_eq!(Context::new(154, 26), Context::of(0, 1));
    }

    #[test]
    fn renormalization_after_an_lps_matches_the_table() {
        // The shift brings an LPS range back to at least 256.
        for lps in [2u32, 6, 7, 8, 15, 16, 31, 32, 63, 64, 127, 128, 240] {
            let shift = u32::from(NORM_SHIFT[lps as usize]);
            assert!(
                (lps << shift) >= 256 && (lps << shift) < 512,
                "{lps} {shift}"
            );
        }
    }
}
