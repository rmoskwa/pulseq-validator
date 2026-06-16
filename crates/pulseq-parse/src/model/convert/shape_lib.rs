use std::{collections::HashMap, sync::Arc};

use num_complex::Complex64;

use crate::{error::ConversionError, model::Shape};

pub struct ShapeLib {
    raw: HashMap<u32, Arc<Vec<f64>>>,
    memo: HashMap<(u32, i32), Arc<Shape<f64>>>,
    complex_memo: HashMap<(u32, u32, i32), Arc<Shape<Complex64>>>,
}

impl ShapeLib {
    pub fn new(raw: HashMap<u32, Arc<Vec<f64>>>) -> Result<Self, ConversionError> {
        // Checking these guarantees once makes later code easier.
        if raw.contains_key(&0) {
            return Err(ConversionError::ShapeIndexZero);
        }
        if raw.values().any(|s| s.is_empty()) {
            return Err(ConversionError::EmptyShape);
        }
        Ok(Self {
            raw,
            memo: HashMap::default(),
            complex_memo: HashMap::default(),
        })
    }

    pub fn get(&mut self, shape_id: u32, time_id: i32) -> Result<Arc<Shape<f64>>, ConversionError> {
        let key = (shape_id, time_id);
        if let Some(cached) = self.memo.get(&key) {
            return Ok(cached.clone());
        }

        let amp_arc = self
            .raw
            .get(&shape_id)
            .ok_or(ConversionError::ShapeNotFound(shape_id))?
            .clone();
        let amp = (*amp_arc).clone();
        let m = amp.len();

        let (time, duration) = match time_id {
            // time_id = 0: uniform centers [0.5, 1.5, ..., M-0.5], duration M.
            0 => {
                let time: Vec<f64> = (0..m).map(|i| i as f64 + 0.5).collect();
                (time, m as u32)
            }
            // time_id = -1 (pulseq 1.5+): half-tick grid [0.5, 1.0, ..., M*0.5].
            // M = 2N-1, so the shape's sample count must be odd.
            -1 => {
                if m % 2 == 0 {
                    return Err(ConversionError::HalfTickShapeEvenSampleCount(m));
                }
                let time: Vec<f64> = (0..m).map(|i| (i + 1) as f64 * 0.5).collect();
                let duration = m.div_ceil(2) as u32;
                (time, duration)
            }
            // Custom time shape - look it up and validate.
            x if x > 0 => {
                let time_shape_id = x as u32;
                let raw_time = self
                    .raw
                    .get(&time_shape_id)
                    .ok_or(ConversionError::ShapeNotFound(time_shape_id))?;

                if raw_time.len() != m {
                    return Err(ConversionError::TimeShapeMismatch {
                        shape_len: m,
                        time_len: raw_time.len(),
                    });
                }
                if raw_time.iter().any(|x| x.fract() != 0.0) {
                    return Err(ConversionError::TimeShapeNonInteger);
                }
                if raw_time.iter().any(|x| *x < 0.0) {
                    return Err(ConversionError::TimeShapeNegative);
                }
                let time: Vec<f64> = raw_time.iter().copied().collect();
                let duration = *raw_time.last().ok_or(ConversionError::EmptyShape)? as u32;
                (time, duration)
            }
            other => return Err(ConversionError::UnknownTimeId(other)),
        };

        let shape = Arc::new(Shape::new(time, amp, duration)?);
        self.memo.insert(key, shape.clone());
        Ok(shape)
    }

    /// Like [`get`], but for a Free gradient where the boundary samples at
    /// `t = 0` and `t = shape_dur` must be explicit in the shape.
    ///
    /// `first` and `last` are in the same normalized units as the raw shape
    /// samples (i.e. divide the gradient's absolute Hz/m by `Gradient.amp`
    /// before calling).
    ///
    /// For `time_id != 0` the existing samples already sit at the boundaries,
    /// so this delegates to [`get`]. For `time_id == 0` (uniform centers
    /// `[0.5, 1.5, …, M-0.5]`) the result has `M + 2` samples on a non-uniform
    /// grid `[0.0, 0.5, 1.5, …, M-0.5, M]` with `first` and `last` bracketing
    /// the original samples. Not memoized — pre-1.5 conversion calls this
    /// at most once per gradient id.
    pub fn get_with_boundaries(
        &mut self,
        shape_id: u32,
        time_id: i32,
        first: f64,
        last: f64,
    ) -> Result<Arc<crate::model::Shape<f64>>, ConversionError> {
        if time_id != 0 {
            return self.get(shape_id, time_id);
        }
        let raw = self
            .raw
            .get(&shape_id)
            .ok_or(ConversionError::ShapeNotFound(shape_id))?
            .clone();
        let m = raw.len();
        let mut time = Vec::with_capacity(m + 2);
        let mut amp = Vec::with_capacity(m + 2);
        time.push(0.0);
        amp.push(first);
        for (i, &s) in raw.iter().enumerate() {
            time.push(i as f64 + 0.5);
            amp.push(s);
        }
        time.push(m as f64);
        amp.push(last);
        Ok(Arc::new(crate::model::Shape::new(time, amp, m as u32)?))
    }

    pub fn get_complex(
        &mut self,
        mag_id: u32,
        phase_id: u32,
        time_id: i32,
    ) -> Result<Arc<Shape<Complex64>>, ConversionError> {
        let key = (mag_id, phase_id, time_id);
        if let Some(cached) = self.complex_memo.get(&key) {
            return Ok(cached.clone());
        }

        let mag = self.get(mag_id, time_id)?;
        let phase = self.get(phase_id, time_id)?;

        // mag and phase share `time` when `time_id != 0`. For `time_id == 0`
        // both are synthesised from their own length, so a length mismatch
        // here means the two raw shapes disagreed.
        if mag.amp.len() != phase.amp.len() {
            return Err(ConversionError::TimeShapeMismatch {
                shape_len: mag.amp.len(),
                time_len: phase.amp.len(),
            });
        }

        let amp: Vec<Complex64> = mag
            .amp
            .iter()
            .zip(phase.amp.iter())
            .map(|(&a, &p)| Complex64::from_polar(a, p * std::f64::consts::TAU))
            .collect();

        let shape = Arc::new(Shape::new(mag.time.clone(), amp, mag.duration)?);
        self.complex_memo.insert(key, shape.clone());
        Ok(shape)
    }
}
