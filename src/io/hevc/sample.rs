//! A plane's samples: bytes up to 8 bits, which halves a picture's
//! memory, and 16-bit words above; and the `Sample` trait the decoder's
//! sample loops are written over, so each runs on either without a
//! branch per sample.

/// A sample of either width, as the decoder computes with it.
pub trait Sample: Copy + Default + Send + Sync + 'static {
    fn value(self) -> i32;
    /// From a value already within the bit depth's range.
    fn of(value: i32) -> Self;
}

impl Sample for u8 {
    #[inline(always)]
    fn value(self) -> i32 {
        i32::from(self)
    }

    #[inline(always)]
    fn of(value: i32) -> u8 {
        value as u8
    }
}

impl Sample for u16 {
    #[inline(always)]
    fn value(self) -> i32 {
        i32::from(self)
    }

    #[inline(always)]
    fn of(value: i32) -> u16 {
        value as u16
    }
}

/// Runs `$body` with `$plane` bound to the samples of either width.
macro_rules! with_samples {
    ($samples:expr, $plane:ident => $body:expr) => {
        match $samples {
            $crate::io::hevc::Samples::Eight($plane) => $body,
            $crate::io::hevc::Samples::Deep($plane) => $body,
        }
    };
}
pub(crate) use with_samples;

/// A plane's samples: bytes at 8 bits, which halves a large photo's
/// memory, and 16-bit words above.
pub enum Samples {
    Eight(Vec<u8>),
    Deep(Vec<u16>),
}

impl Samples {
    pub fn new(depth: u32, count: usize) -> Samples {
        if depth <= 8 {
            Samples::Eight(vec![0; count])
        } else {
            Samples::Deep(vec![0; count])
        }
    }

    /// These samples' storage for `count` samples at `depth`, when it is
    /// of that width; else new storage. The values are left as they were.
    pub fn reused(self, depth: u32, count: usize) -> Samples {
        match (self, depth <= 8) {
            (Samples::Eight(mut samples), true) => {
                samples.resize(count, 0);
                Samples::Eight(samples)
            }
            (Samples::Deep(mut samples), false) => {
                samples.resize(count, 0);
                Samples::Deep(samples)
            }
            _ => Samples::new(depth, count),
        }
    }

    /// The samples as bytes, when they are 8-bit.
    pub fn bytes(&self) -> Option<&[u8]> {
        match self {
            Samples::Eight(samples) => Some(samples),
            Samples::Deep(_) => None,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Samples::Eight(samples) => samples.len(),
            Samples::Deep(samples) => samples.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> u16 {
        match self {
            Samples::Eight(samples) => u16::from(samples[index]),
            Samples::Deep(samples) => samples[index],
        }
    }

    /// `count` samples from `start`, widened into `out`.
    pub fn widen(&self, start: usize, count: usize, out: &mut [i64]) {
        match self {
            Samples::Eight(samples) => {
                for (value, &sample) in out.iter_mut().zip(&samples[start..start + count]) {
                    *value = i64::from(sample);
                }
            }
            Samples::Deep(samples) => {
                for (value, &sample) in out.iter_mut().zip(&samples[start..start + count]) {
                    *value = i64::from(sample);
                }
            }
        }
    }

    /// Copies `count` samples from `from` at `source` to `at`.
    pub fn copy_from(&mut self, at: usize, from: &Samples, source: usize, count: usize) {
        match (self, from) {
            (Samples::Eight(to), Samples::Eight(from)) => {
                to[at..at + count].copy_from_slice(&from[source..source + count]);
            }
            (Samples::Deep(to), Samples::Deep(from)) => {
                to[at..at + count].copy_from_slice(&from[source..source + count]);
            }
            (to, from) => {
                for offset in 0..count {
                    let sample = from.get(source + offset);
                    match to {
                        Samples::Eight(to) => to[at + offset] = sample as u8,
                        Samples::Deep(to) => to[at + offset] = sample,
                    }
                }
            }
        }
    }
}
