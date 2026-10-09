//! A plane's samples: bytes up to 8 bits, which halves a picture's
//! memory, and 16-bit words above; and the `Sample` trait the decoder's
//! sample loops are written over, so each runs on either without a
//! branch per sample.

/// A sample of either width, as the decoder computes with it.
pub trait Sample: Copy + Default + Send + Sync + 'static {
    fn value(self) -> i32;
    /// From a value already within the bit depth's range.
    fn of(value: i32) -> Self;
    /// The samples, when they are of this width; else none.
    fn slice(samples: &Samples) -> &[Self];
    /// Runs `work` on this thread's scratch of `count` samples, all zero:
    /// the buffer is kept for the thread's next call, not allocated anew.
    fn with_scratch<R>(count: usize, work: impl FnOnce(&mut [Self]) -> R) -> R;
}

// Each thread's scratch samples of each width.
std::thread_local! {
    static SCRATCH_EIGHT: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    static SCRATCH_DEEP: std::cell::RefCell<Vec<u16>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// `work` on `scratch` cleared to `count` zeros; a nested call (none
/// today) gets a buffer of its own.
fn with_cleared<T: Copy + Default, R>(
    scratch: &std::cell::RefCell<Vec<T>>,
    count: usize,
    work: impl FnOnce(&mut [T]) -> R,
) -> R {
    let Ok(mut buffer) = scratch.try_borrow_mut() else {
        return work(&mut vec![T::default(); count]);
    };
    buffer.clear();
    buffer.resize(count, T::default());
    work(&mut buffer)
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

    fn slice(samples: &Samples) -> &[u8] {
        match samples {
            Samples::Eight(samples) => samples,
            Samples::Deep(_) => &[],
        }
    }

    fn with_scratch<R>(count: usize, work: impl FnOnce(&mut [u8]) -> R) -> R {
        SCRATCH_EIGHT.with(|scratch| with_cleared(scratch, count, work))
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

    fn slice(samples: &Samples) -> &[u16] {
        match samples {
            Samples::Deep(samples) => samples,
            Samples::Eight(_) => &[],
        }
    }

    fn with_scratch<R>(count: usize, work: impl FnOnce(&mut [u16]) -> R) -> R {
        SCRATCH_DEEP.with(|scratch| with_cleared(scratch, count, work))
    }
}

/// A plane's samples: bytes at 8 bits, which halves a large photo's
/// memory, and 16-bit words above.
#[derive(Clone)]
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

    /// Keeps the storage, `count` samples long.
    pub fn resize(&mut self, count: usize) {
        match self {
            Samples::Eight(samples) => samples.resize(count, 0),
            Samples::Deep(samples) => samples.resize(count, 0),
        }
    }

    /// Moves `count` samples from `start` to the front.
    pub fn raise(&mut self, start: usize, count: usize) {
        match self {
            Samples::Eight(samples) => samples.copy_within(start..start + count, 0),
            Samples::Deep(samples) => samples.copy_within(start..start + count, 0),
        }
    }

    /// The samples as a band of rows to write.
    pub(super) fn band(&mut self) -> super::decode::Band<'_> {
        match self {
            Samples::Eight(samples) => super::decode::Band::Eight(samples),
            Samples::Deep(samples) => super::decode::Band::Deep(samples),
        }
    }

    /// Appends `other`'s samples (of the same width).
    pub fn extend(&mut self, other: &Samples) {
        match (self, other) {
            (Samples::Eight(to), Samples::Eight(from)) => to.extend_from_slice(from),
            (Samples::Deep(to), Samples::Deep(from)) => to.extend_from_slice(from),
            _ => {}
        }
    }

    /// `count` samples from `start`, copied.
    pub fn slice_of(&self, start: usize, count: usize) -> Samples {
        match self {
            Samples::Eight(samples) => Samples::Eight(samples[start..start + count].to_vec()),
            Samples::Deep(samples) => Samples::Deep(samples[start..start + count].to_vec()),
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
