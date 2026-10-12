//! CABAC for encoding (H.265 9.3.4.4 in reverse): the arithmetic encoder
//! that writes a substream, and an estimator that counts what the same
//! bins would cost, so a decision is weighed by the very syntax that
//! would be written. Both update the context variables the same way.

use super::super::cabac::{Context, LPS_RANGE, NEXT_STATE};

/// What syntax is written through: the encoder, or the estimator.
pub trait Coder {
    /// A context-coded bin.
    fn decision(&mut self, context: &mut Context, bin: u32);
    /// `count` bypass bins of `value`, most significant first.
    fn bypass(&mut self, value: u32, count: u32);
    /// A terminating bin.
    fn terminate(&mut self, bin: u32);
}

/// The fractional bits of a bin, in 1/32768ths: by packed state (state
/// times two plus the MPS) and the bin's value.
#[inline(always)]
pub fn bin_cost(packed: u8, bin: u32) -> u32 {
    COSTS[usize::from(packed)][bin as usize]
}

/// Each packed state's costs of a 0 and a 1: minus the log2 of the bin's
/// probability (the LPS's 0.5 * (0.01875 / 0.5)^(state / 63), 9.3.4.2's
/// derivation), times 32768, rounded (the test checks the formula).
pub(super) const COSTS: [[u32; 2]; 128] = [
    [32768, 32768],
    [32768, 32768],
    [30426, 35232],
    [35232, 30426],
    [28306, 37696],
    [37696, 28306],
    [26377, 40159],
    [40159, 26377],
    [24617, 42623],
    [42623, 24617],
    [23005, 45087],
    [45087, 23005],
    [21523, 47551],
    [47551, 21523],
    [20159, 50015],
    [50015, 20159],
    [18899, 52479],
    [52479, 18899],
    [17734, 54942],
    [54942, 17734],
    [16653, 57406],
    [57406, 16653],
    [15650, 59870],
    [59870, 15650],
    [14717, 62334],
    [62334, 14717],
    [13849, 64798],
    [64798, 13849],
    [13038, 67262],
    [67262, 13038],
    [12282, 69725],
    [69725, 12282],
    [11575, 72189],
    [72189, 11575],
    [10914, 74653],
    [74653, 10914],
    [10294, 77117],
    [77117, 10294],
    [9714, 79581],
    [79581, 9714],
    [9169, 82044],
    [82044, 9169],
    [8658, 84508],
    [84508, 8658],
    [8178, 86972],
    [86972, 8178],
    [7727, 89436],
    [89436, 7727],
    [7303, 91900],
    [91900, 7303],
    [6903, 94364],
    [94364, 6903],
    [6527, 96827],
    [96827, 6527],
    [6173, 99291],
    [99291, 6173],
    [5840, 101755],
    [101755, 5840],
    [5525, 104219],
    [104219, 5525],
    [5228, 106683],
    [106683, 5228],
    [4948, 109147],
    [109147, 4948],
    [4684, 111610],
    [111610, 4684],
    [4435, 114074],
    [114074, 4435],
    [4199, 116538],
    [116538, 4199],
    [3977, 119002],
    [119002, 3977],
    [3767, 121466],
    [121466, 3767],
    [3568, 123929],
    [123929, 3568],
    [3380, 126393],
    [126393, 3380],
    [3202, 128857],
    [128857, 3202],
    [3034, 131321],
    [131321, 3034],
    [2876, 133785],
    [133785, 2876],
    [2725, 136249],
    [136249, 2725],
    [2583, 138712],
    [138712, 2583],
    [2448, 141176],
    [141176, 2448],
    [2321, 143640],
    [143640, 2321],
    [2200, 146104],
    [146104, 2200],
    [2086, 148568],
    [148568, 2086],
    [1978, 151032],
    [151032, 1978],
    [1875, 153495],
    [153495, 1875],
    [1778, 155959],
    [155959, 1778],
    [1686, 158423],
    [158423, 1686],
    [1599, 160887],
    [160887, 1599],
    [1517, 163351],
    [163351, 1517],
    [1439, 165814],
    [165814, 1439],
    [1364, 168278],
    [168278, 1364],
    [1294, 170742],
    [170742, 1294],
    [1228, 173206],
    [173206, 1228],
    [1164, 175670],
    [175670, 1164],
    [1105, 178134],
    [178134, 1105],
    [1048, 180597],
    [180597, 1048],
    [994, 183061],
    [183061, 994],
    [943, 185525],
    [185525, 943],
    [895, 187989],
    [187989, 895],
];

/// The cost of one bypass bin, in 1/32768ths.
pub const BYPASS_COST: u32 = 32768;

/// Counts the cost of bins without writing them.
#[derive(Default, Clone, Copy)]
pub struct Estimator {
    pub bits: u64,
}

impl Coder for Estimator {
    #[inline(always)]
    fn decision(&mut self, context: &mut Context, bin: u32) {
        self.bits += u64::from(bin_cost(context.packed, bin));
        context.packed = next_state(context.packed, bin);
    }

    #[inline(always)]
    fn bypass(&mut self, _value: u32, count: u32) {
        self.bits += u64::from(count) * u64::from(BYPASS_COST);
    }

    fn terminate(&mut self, bin: u32) {
        // Nearly free when 0, a few bits to end a substream.
        self.bits += if bin == 0 { 0 } else { 7 * 32768 };
    }
}

/// A context's packed state after coding `bin`.
#[inline(always)]
pub fn next_state(packed: u8, bin: u32) -> u8 {
    let mps = u32::from(packed & 1);
    if bin == mps {
        NEXT_STATE[128 + usize::from(packed)]
    } else {
        NEXT_STATE[127 - usize::from(packed)]
    }
}

/// The arithmetic encoder of a substream (the form of the HM reference
/// encoder: `low` holds the bits not yet settled, a run of 0xFF bytes
/// waits for a possible carry).
pub struct Encoder {
    low: u32,
    range: u32,
    bits_left: i32,
    buffered: u32,
    buffered_count: u32,
    out: BitSink,
}

/// Bits written MSB first.
#[derive(Default)]
pub struct BitSink {
    pub bytes: Vec<u8>,
    current: u32,
    count: u32,
}

impl BitSink {
    pub fn put(&mut self, value: u32, count: u32) {
        for index in (0..count).rev() {
            self.current = (self.current << 1) | ((value >> index) & 1);
            self.count += 1;
            if self.count == 8 {
                self.bytes.push(self.current as u8);
                self.current = 0;
                self.count = 0;
            }
        }
    }

    #[inline(always)]
    fn byte(&mut self, value: u32) {
        if self.count == 0 {
            self.bytes.push(value as u8);
        } else {
            self.put(value & 0xFF, 8);
        }
    }

    /// `byte_alignment()`: a one, then zeros to the byte.
    pub fn align(&mut self) {
        self.put(1, 1);
        while self.count != 0 {
            self.put(0, 1);
        }
    }
}

impl Encoder {
    pub fn new(out: Vec<u8>) -> Encoder {
        let mut out = BitSink {
            bytes: out,
            current: 0,
            count: 0,
        };
        out.bytes.clear();
        Encoder {
            low: 0,
            range: 510,
            bits_left: 23,
            buffered: 0xFF,
            buffered_count: 0,
            out,
        }
    }

    #[inline(always)]
    fn settle(&mut self) {
        if self.bits_left < 12 {
            self.write_out();
        }
    }

    fn write_out(&mut self) {
        let lead = self.low >> (24 - self.bits_left);
        self.bits_left += 8;
        self.low &= 0xFFFF_FFFF >> self.bits_left;
        if lead == 0xFF {
            self.buffered_count += 1;
        } else if self.buffered_count > 0 {
            let carry = lead >> 8;
            let byte = self.buffered + carry;
            self.buffered = lead & 0xFF;
            self.out.byte(byte);
            let filler = (0xFF + carry) & 0xFF;
            while self.buffered_count > 1 {
                self.out.byte(filler);
                self.buffered_count -= 1;
            }
        } else {
            self.buffered_count = 1;
            self.buffered = lead;
        }
    }

    /// Ends the substream after its terminating bin of 1, with
    /// `byte_alignment()`, and gives its bytes.
    pub fn finish(mut self) -> Vec<u8> {
        if (self.low >> (32 - self.bits_left)) != 0 {
            self.out.byte(self.buffered + 1);
            while self.buffered_count > 1 {
                self.out.byte(0x00);
                self.buffered_count -= 1;
            }
            self.low -= 1 << (32 - self.bits_left);
        } else {
            if self.buffered_count > 0 {
                self.out.byte(self.buffered);
            }
            while self.buffered_count > 1 {
                self.out.byte(0xFF);
                self.buffered_count -= 1;
            }
        }
        self.out.put(self.low >> 8, (24 - self.bits_left) as u32);
        self.out.align();
        self.out.bytes
    }
}

impl Coder for Encoder {
    #[inline(always)]
    fn decision(&mut self, context: &mut Context, bin: u32) {
        let packed = context.packed;
        let (state, mps) = (usize::from(packed >> 1), u32::from(packed & 1));
        let lps = u32::from(LPS_RANGE[state][((self.range >> 6) & 3) as usize]);
        self.range -= lps;
        if bin != mps {
            // Renormalized so the range is at least 256 again (an LPS
            // range is under 256).
            let shift = lps.leading_zeros() as i32 - 23;
            self.low = (self.low + self.range) << shift;
            self.range = lps << shift;
            self.bits_left -= shift;
        } else {
            if self.range >= 256 {
                context.packed = next_state(packed, bin);
                return;
            }
            self.low <<= 1;
            self.range <<= 1;
            self.bits_left -= 1;
        }
        context.packed = next_state(packed, bin);
        self.settle();
    }

    fn bypass(&mut self, value: u32, count: u32) {
        let mut value = value;
        let mut count = count;
        while count > 8 {
            count -= 8;
            let pattern = value >> count;
            self.low <<= 8;
            self.low += self.range * pattern;
            value -= pattern << count;
            self.bits_left -= 8;
            self.settle();
        }
        self.low <<= count;
        self.low += self.range * value;
        self.bits_left -= count as i32;
        self.settle();
    }

    fn terminate(&mut self, bin: u32) {
        self.range -= 2;
        if bin != 0 {
            self.low += self.range;
            self.low <<= 7;
            self.range = 2 << 7;
            self.bits_left -= 7;
        } else if self.range >= 256 {
            return;
        } else {
            self.low <<= 1;
            self.range <<= 1;
            self.bits_left -= 1;
        }
        self.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::cabac::{Cabac, Context};
    use super::*;

    #[test]
    fn bin_costs_follow_the_probabilities() {
        for (packed, costs) in COSTS.iter().enumerate() {
            let (state, mps) = (packed >> 1, packed & 1);
            let lps = 0.5 * (0.01875f64 / 0.5).powf(state as f64 / 63.0);
            let bits = |probability: f64| (-probability.log2() * 32768.0).round() as u32;
            assert_eq!(costs[mps], bits(1.0 - lps), "{packed}");
            assert_eq!(costs[1 - mps], bits(lps), "{packed}");
        }
    }

    /// Bins written by the encoder read back the same through the decoder.
    #[test]
    fn written_bins_decode_back() {
        let mut seed = 0x1234_5678u32;
        let mut random = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        let mut steps = Vec::new();
        for _ in 0..20_000 {
            let kind = random() % 10;
            let skewed = u32::from(random() % 8 != 0);
            steps.push(match kind {
                0..=6 => (0, (random() % 4) as usize, skewed),
                7 | 8 => (1, (random() % 12) as usize + 1, random() & 0xFFFF),
                _ => (2, 0, 0),
            });
        }
        let mut contexts = [Context::default(); 4];
        let mut encoder = Encoder::new(Vec::new());
        for &(kind, index, value) in &steps {
            match kind {
                0 => encoder.decision(&mut contexts[index], value),
                1 => encoder.bypass(value & ((1 << index) - 1), index as u32),
                _ => encoder.terminate(0),
            }
        }
        encoder.terminate(1);
        let bytes = encoder.finish();
        let mut contexts = [Context::default(); 4];
        let mut decoder = Cabac::new(&bytes, 0);
        for (step, &(kind, index, value)) in steps.iter().enumerate() {
            match kind {
                0 => assert_eq!(decoder.decision(&mut contexts[index]), value, "step {step}"),
                1 => assert_eq!(
                    decoder.bypass_bits(index as u32),
                    value & ((1 << index) - 1),
                    "step {step}"
                ),
                _ => assert_eq!(decoder.terminate(), 0, "step {step}"),
            }
        }
        assert_eq!(decoder.terminate(), 1);
    }
}
