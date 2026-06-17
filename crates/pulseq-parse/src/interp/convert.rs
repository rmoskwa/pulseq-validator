use std::collections::HashMap;
use std::sync::Arc;

use num_complex::Complex64;

use crate::error::{InterpreterError, InterpreterWarning};
use crate::interp::{Quaternion, Transform};
use crate::model;

/// Lowers a seq sequence into the int form, folding the relative
/// (`rel × larmor`) and absolute components of RF/ADC frequency and phase,
/// resolving RF shims from either the `rf_shims` extension or the pTx shim
/// shape on the seq RF, and applying the per-axis scale of `fov` to gradient
/// amplitudes.
pub fn convert(
    seq: &model::Sequence,
    fov: super::Transform,
    larmor: f64,
    soft_delays: HashMap<String, f64>,
    warnings: &mut Vec<InterpreterWarning>,
) -> Result<super::Sequence, InterpreterError> {
    // Every soft-delay referenced anywhere in the sequence must have a value
    // in the input map. We check up front so the per-block loop can assume
    // all lookups succeed.
    for (id, hint) in &seq.soft_delay_hints {
        if !soft_delays.contains_key(hint) {
            return Err(InterpreterError::MissingSoftDelay {
                id: *id,
                hint: hint.clone(),
            });
        }
    }

    // Update the (purely informative) sequence FOV to account for scaling:
    let out_fov = {
        let s = fov.scale;
        seq.fov.map_or([s; 3], |(x, y, z)| [s * x, s * y, s * z])
    };

    // Track the channel count established by the first explicit shim so we
    // can warn (not error) if later RFs disagree.
    let mut expected_shim_channels: Option<usize> = None;
    // Sticky label state - all counters and flags start at 0 / false. Each
    // block's LABELSETs are applied before its LABELINCs (per spec). State
    // is then snapshotted: ADC-relevant fields go onto `Adc::labels`, the
    // block-level subset (trid, once, pmc, no_*) onto `Block`.
    let mut label_state = LabelState::default();
    // Memoizes seq → int shape conversions so blocks that share an
    // `Arc<model::Shape>` end up sharing a single `Arc<interp::Shape>` too.
    let mut shapes = ShapeLib::default();
    let mut blocks = Vec::with_capacity(seq.blocks.len());

    for block in &seq.blocks {
        let rf = block
            .rf
            .as_ref()
            .map(|rf| -> Result<super::Rf, InterpreterError> {
                let shims = resolve_shims(block.id, rf, &block.ext)?;
                if shims.len() > 1 {
                    match expected_shim_channels {
                        None => expected_shim_channels = Some(shims.len()),
                        Some(expected) if expected != shims.len() => {
                            warnings.push(InterpreterWarning::InconsistentShimChannelCount {
                                block_id: block.id,
                                expected,
                                got: shims.len(),
                            });
                        }
                        _ => {}
                    }
                }
                Ok(convert_rf(
                    rf,
                    larmor,
                    seq.time_raster.rf,
                    shims,
                    &mut shapes,
                ))
            })
            .transpose()?;

        // Apply any `Delay` extensions on this block. Each one computes a
        // candidate duration `t_factor * x + t_offset` where `x` is the
        // user-supplied value for that hint. Updates only when the candidate
        // is at least the current duration; otherwise warns and skips.
        let delay_count = block
            .ext
            .iter()
            .filter(|e| matches!(e, model::Extension::Delay { .. }))
            .count();
        if delay_count > 1 {
            warnings.push(InterpreterWarning::MultipleSoftDelays { block_id: block.id });
        }
        let mut duration = block.duration;
        for ext in &block.ext {
            if let model::Extension::Delay {
                id,
                t_offset,
                t_factor,
            } = ext
            {
                // Both lookups are guaranteed by the validation above.
                #[allow(clippy::indexing_slicing)]
                let x = soft_delays[&seq.soft_delay_hints[id]];
                let computed = t_factor * x + t_offset;
                if computed >= block.duration {
                    duration = computed;
                } else {
                    warnings.push(InterpreterWarning::SoftDelayShortensBlock {
                        block_id: block.id,
                        computed,
                        block: block.duration,
                    });
                }
            }
        }

        // First pass: apply all LABELSETs.
        for ext in &block.ext {
            if let model::Extension::LabelSet { flag, value } = ext {
                label_state.apply_set(flag, *value, block.id)?;
            }
        }
        // Second pass: apply all LABELINCs.
        for ext in &block.ext {
            if let model::Extension::LabelInc { counter, value } = ext {
                label_state.apply_inc(counter, *value);
            }
        }

        // Compute gradient transform for current block, starting with rot ext.
        // Default quaternion `[1, 0, 0, 0]` is the identity rotation; any
        // `rotations` extension on the block overrides it. Two or more
        // instances on the same block are ambiguous and rejected.
        let mut rot_iter = block.ext.iter().filter_map(|e| match e {
            model::Extension::Rotation { quat } => Some(Quaternion(*quat)),
            _ => None,
        });
        let rot_extension = rot_iter.next().unwrap_or_default();
        if rot_iter.next().is_some() {
            return Err(InterpreterError::MultipleRotationExtensions { block_id: block.id });
        }

        // This is the full scanner transform of the sequence (PMC missing)
        let mut transform = fov;
        transform.rotation = transform.rotation * rot_extension;
        // Narrow it down if the seq flags were set
        if label_state.no_rot {
            transform.rotation = rot_extension;
        }
        if label_state.no_scl {
            transform.scale = 1.0;
        }
        if label_state.no_pos {
            transform.position = [0.0; 3];
        }

        let (gx, gy, gz) = transform_grad(
            block.gx.as_deref(),
            block.gy.as_deref(),
            block.gz.as_deref(),
            transform,
            seq.time_raster.grad,
            &mut shapes,
        );

        blocks.push(super::Block {
            duration,
            rf,
            gx,
            gy,
            gz,
            adc: block
                .adc
                .as_ref()
                .map(|adc| convert_adc(adc, larmor, label_state.adc_labels, &mut shapes)),
            triggers: block
                .ext
                .iter()
                .filter_map(|ext| match ext {
                    model::Extension::Trigger {
                        typ,
                        channel,
                        delay,
                        duration,
                    } => Some(super::Trigger {
                        typ: *typ,
                        channel: *channel,
                        delay: *delay,
                        duration: *duration,
                    }),
                    _ => None,
                })
                .collect(),
            labels: label_state.block_labels,
        });
    }

    Ok(super::Sequence {
        name: seq.name.clone(),
        fov: out_fov,
        blocks,
    })
}

/// Resolves the shim for one RF. Errors on multiple `Shimming` extensions,
/// conflicting sources, or an empty explicit shim.
fn resolve_shims(
    block_id: u32,
    rf: &model::Rf,
    extensions: &[model::Extension],
) -> Result<Vec<Complex64>, InterpreterError> {
    let mut ext_iter = extensions.iter().filter_map(|e| match e {
        model::Extension::Shimming { shim } => Some(shim),
        _ => None,
    });
    let ext_shim = ext_iter.next();
    if ext_iter.next().is_some() {
        return Err(InterpreterError::MultipleShimmingExtensions { block_id });
    }

    match (ext_shim, rf.shim_shape.as_ref()) {
        (Some(_), Some(_)) => Err(InterpreterError::ConflictingShimSources { block_id }),
        (Some(ext), None) => {
            if ext.is_empty() {
                return Err(InterpreterError::EmptyShim { block_id });
            }
            let shim = ext
                .iter()
                .map(|[a, p]| Complex64::from_polar(*a, p * std::f64::consts::TAU))
                .collect();
            Ok(shim)
        }
        (None, Some(ptx)) => {
            if ptx.amp.is_empty() {
                return Err(InterpreterError::EmptyShim { block_id });
            }
            Ok(ptx.amp.clone())
        }
        (None, None) => Ok(vec![Complex64::new(1.0, 0.0)]),
    }
}

fn convert_rf(
    rf: &model::Rf,
    larmor: f64,
    rf_raster: f64,
    shims: Vec<Complex64>,
    shapes: &mut ShapeLib,
) -> super::Rf {
    super::Rf {
        amp: rf.amp,
        phase: rf.phase.0 * larmor + rf.phase.1,
        delay: rf.delay,
        center: rf.center,
        freq: rf.freq.0 * larmor + rf.freq.1,
        shape: shapes.get_complex(&rf.shape, rf_raster),
        shims,
        rf_use: rf.rf_use,
    }
}

/// Resolves a single seq gradient to its `(amp, delay, int_shape)` triple.
/// Trap and Free both end up with a memoized `Arc<interp::Shape<f64>>`; for
/// Trap that's the synthesised `[0, 1, 1, 0]` envelope, for Free it's the
/// per-sample shape from the raw file.
fn lookup_grad(g: &model::Gradient, grad_raster: f64, shapes: &mut ShapeLib) -> GradLookup {
    match g {
        model::Gradient::Free { amp, delay, shape } => {
            (*amp, *delay, shapes.get(shape, grad_raster))
        }
        model::Gradient::Trap {
            amp,
            rise,
            flat,
            fall,
            delay,
        } => (*amp, *delay, shapes.get_trap(*rise, *flat, *fall)),
    }
}

/// One resolved gradient axis: `(amp, delay, int_shape)`, as returned by
/// `lookup_grad`.
type GradLookup = (f64, f64, Arc<super::Shape<f64>>);

/// The three (optional) output gradient axes of `transform_grad`.
type GradAxes = (
    Option<super::Gradient>,
    Option<super::Gradient>,
    Option<super::Gradient>,
);

/// Applies the FOV transform (scale + rotation) across all three gradient
/// axes at once. With identity rotation each output axis is independent and
/// the result matches per-axis scaling. With a non-identity rotation there are
/// two cases: if every present input shares the same `interp::Shape` Arc (after
/// `ShapeLib` lookup) and the same `delay`, the rotation reduces to a matrix on
/// the scalar amplitudes against that one shared shape (the common PROPELLER /
/// rotated-spoke case, which keeps the Arc shared); otherwise the axes are
/// resampled onto a common breakpoint grid and mixed per sample (a rotated
/// readout combined with a differently-shaped blip on another axis — e.g.
/// stack-of-stars). `None` axes can become `Some` if rotation projects onto them.
fn transform_grad(
    gx: Option<&model::Gradient>,
    gy: Option<&model::Gradient>,
    gz: Option<&model::Gradient>,
    transform: Transform,
    grad_raster: f64,
    shapes: &mut ShapeLib,
) -> GradAxes {
    let lookups: [Option<GradLookup>; 3] = [
        gx.map(|g| lookup_grad(g, grad_raster, shapes)),
        gy.map(|g| lookup_grad(g, grad_raster, shapes)),
        gz.map(|g| lookup_grad(g, grad_raster, shapes)),
    ];

    // Identity rotation: each axis is independent, so we can sidestep the
    // shared-shape/delay requirement that the rotated path needs. Just apply
    // the inverse scale (`to_grad_transform` would do the same, but only
    // along the diagonal).
    if transform.rotation.is_identity() {
        let inv_scale = 1.0 / transform.scale;
        let emit = |opt: Option<GradLookup>| {
            opt.map(|(amp, delay, shape)| super::Gradient {
                amp: amp * inv_scale,
                delay,
                shape,
            })
        };
        let [lx, ly, lz] = lookups;
        return (emit(lx), emit(ly), emit(lz));
    }

    // Pick the first present axis as the reference. If nothing is present
    // there's no gradient — rotation has nothing to project from.
    let Some((_, ref_delay, ref_shape)) = lookups.iter().find_map(|opt| opt.as_ref()).cloned()
    else {
        return (None, None, None);
    };

    let m = transform.to_grad_transform();

    // Fast path: every present axis shares the reference shape and delay, so the
    // rotation is just a matrix on the scalar amplitudes against one shared shape
    // (a PROPELLER blade / radial spoke built from a single canonical waveform).
    // Arc::ptr_eq is exact because ShapeLib hands out the same Arc only for inputs
    // that match in both kind and parameters. Keeps the Arc shared — no resample.
    let shared = lookups
        .iter()
        .flatten()
        .all(|(_, delay, shape)| *delay == ref_delay && Arc::ptr_eq(shape, &ref_shape));
    if shared {
        let amps = [
            lookups[0].as_ref().map_or(0.0, |(a, _, _)| *a),
            lookups[1].as_ref().map_or(0.0, |(a, _, _)| *a),
            lookups[2].as_ref().map_or(0.0, |(a, _, _)| *a),
        ];
        let out = [
            m[0][0] * amps[0] + m[0][1] * amps[1] + m[0][2] * amps[2],
            m[1][0] * amps[0] + m[1][1] * amps[1] + m[1][2] * amps[2],
            m[2][0] * amps[0] + m[2][1] * amps[1] + m[2][2] * amps[2],
        ];
        let emit = |amp: f64| {
            (amp != 0.0).then(|| super::Gradient {
                amp,
                delay: ref_delay,
                shape: ref_shape.clone(),
            })
        };
        return (emit(out[0]), emit(out[1]), emit(out[2]));
    }

    // General path: the present axes differ in shape and/or delay (e.g. a rotated
    // readout combined with a differently-shaped partition/slice blip on another
    // axis — stack-of-stars). Resample every present axis onto the shared
    // breakpoint grid and mix per sample through the transform.
    rotate_resample(&lookups, m)
}

/// Physical value `[Hz/m]` of one resolved input axis at block-relative time
/// `t` `[s]`: `amp · shape(t − delay)` inside the gradient's active window,
/// `0` outside it. Used by [`rotate_resample`] to evaluate each axis on a shared
/// time grid before mixing.
fn axis_value(lookup: &Option<GradLookup>, t: f64) -> f64 {
    let Some((amp, delay, shape)) = lookup else {
        return 0.0;
    };
    let local = t - *delay;
    if local < 0.0 || local > shape.duration {
        0.0
    } else {
        *amp * shape.interpolate(local)
    }
}

/// Resamples the present input axes onto their shared breakpoint grid and applies
/// the gradient transform `m` per sample, returning one interpreted gradient per
/// output axis. The grid is the union of every present axis's sample times (each
/// offset by its own `delay`) plus the active-window boundaries, so each input is
/// piecewise-linear between adjacent grid points and the per-sample mix is exact.
/// Each output gradient folds the (already physical, `Hz/m`) waveform into its
/// `shape` with `amp = 1.0`; an output axis that is identically zero is `None`.
fn rotate_resample(lookups: &[Option<GradLookup>; 3], m: [[f64; 3]; 3]) -> GradAxes {
    let mut grid: Vec<f64> = Vec::new();
    for (_, delay, shape) in lookups.iter().flatten() {
        grid.push(*delay);
        for &t in &shape.time {
            grid.push(delay + t);
        }
        grid.push(delay + shape.duration);
    }
    grid.sort_by(f64::total_cmp);
    grid.dedup_by(|a, b| (*a - *b).abs() <= 1e-12);

    // The caller guarantees at least one present axis, so the grid is non-empty.
    let t0 = grid.first().copied().unwrap_or(0.0);
    let duration = grid.last().copied().unwrap_or(0.0) - t0;
    let times: Vec<f64> = grid.iter().map(|&t| t - t0).collect();

    let mut amps: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for &gt in &grid {
        let vin = [
            axis_value(&lookups[0], gt),
            axis_value(&lookups[1], gt),
            axis_value(&lookups[2], gt),
        ];
        for (out, mrow) in amps.iter_mut().zip(m.iter()) {
            out.push(mrow[0] * vin[0] + mrow[1] * vin[1] + mrow[2] * vin[2]);
        }
    }

    let [ax, ay, az] = amps;
    let emit = |a: Vec<f64>| -> Option<super::Gradient> {
        if a.iter().all(|&v| v == 0.0) {
            return None;
        }
        Some(super::Gradient {
            amp: 1.0,
            delay: t0,
            shape: Arc::new(super::Shape {
                time: times.clone(),
                amp: a,
                duration,
            }),
        })
    };
    (emit(ax), emit(ay), emit(az))
}

fn convert_adc(
    adc: &model::Adc,
    larmor: f64,
    labels: super::Labels,
    shapes: &mut ShapeLib,
) -> super::Adc {
    super::Adc {
        num: adc.num,
        dwell: adc.dwell,
        delay: adc.delay,
        freq: adc.freq.0 * larmor + adc.freq.1,
        phase: adc.phase.0 * larmor + adc.phase.1,
        // ADC phase shapes are sampled per-ADC-sample at `dwell`, so we
        // multiply the seq tick-domain time by `dwell` to get seconds.
        phase_shape: adc.phase_shape.as_ref().map(|s| shapes.get(s, adc.dwell)),
        labels,
    }
}

// TODO: might be worth it to write a generic shape lib shared by raw->seq and
// seq->int conversion?
//
/// Memoizes seq → int shape conversions so repeated references to the same
/// `Arc<model::Shape>` produce a single shared `Arc<interp::Shape>`. Mirrors the
/// role of `model::convert::ShapeLib` at the previous stage.
///
/// Cache key is `(Arc::as_ptr as usize, raster.to_bits())`: pointer identity
/// of the input shape combined with the raster used to lift it into seconds.
/// The raster is part of the key because ADC phase shapes are converted with
/// `dwell`, which can differ per ADC even when the same seq shape is reused.
/// Pointer reuse can't happen during conversion because the seq `Sequence`
/// (which owns those shapes) outlives this function.
#[derive(Default)]
struct ShapeLib {
    real: HashMap<(usize, u64), Arc<super::Shape<f64>>>,
    complex: HashMap<(usize, u64), Arc<super::Shape<Complex64>>>,
    /// Synthesised trapezoid envelopes: `time = [0, rise, rise+flat,
    /// rise+flat+fall]`, `amp = [0, 1, 1, 0]`. Cached by `(rise, flat, fall)`
    /// — `delay` and `amp` belong to the gradient, not the shape, so
    /// dropping them from the key maximises sharing across blocks that
    /// reuse a trap timing.
    trap: HashMap<(u64, u64, u64), Arc<super::Shape<f64>>>,
}

impl ShapeLib {
    fn get(&mut self, shape: &Arc<model::Shape<f64>>, raster: f64) -> Arc<super::Shape<f64>> {
        let key = (Arc::as_ptr(shape) as usize, raster.to_bits());
        self.real
            .entry(key)
            .or_insert_with(|| {
                Arc::new(super::Shape {
                    time: shape.time.iter().map(|&t| t * raster).collect(),
                    amp: shape.amp.clone(),
                    duration: shape.duration as f64 * raster,
                })
            })
            .clone()
    }

    fn get_complex(
        &mut self,
        shape: &Arc<model::Shape<Complex64>>,
        raster: f64,
    ) -> Arc<super::Shape<Complex64>> {
        let key = (Arc::as_ptr(shape) as usize, raster.to_bits());
        self.complex
            .entry(key)
            .or_insert_with(|| {
                Arc::new(super::Shape {
                    time: shape.time.iter().map(|&t| t * raster).collect(),
                    amp: shape.amp.clone(),
                    duration: shape.duration as f64 * raster,
                })
            })
            .clone()
    }

    fn get_trap(&mut self, rise: f64, flat: f64, fall: f64) -> Arc<super::Shape<f64>> {
        let key = (rise.to_bits(), flat.to_bits(), fall.to_bits());
        self.trap
            .entry(key)
            .or_insert_with(|| {
                Arc::new(super::Shape {
                    time: vec![0.0, rise, rise + flat, rise + flat + fall],
                    amp: vec![0.0, 1.0, 1.0, 0.0],
                    duration: rise + flat + fall,
                })
            })
            .clone()
    }
}

#[derive(Default)]
struct LabelState {
    adc_labels: super::Labels,
    block_labels: super::BlockLabels,

    /// Disable FOV rotations for the current block
    no_rot: bool,
    /// Disable FOV positioning for the current block
    no_pos: bool,
    /// Disable FOV scaling for the current block
    no_scl: bool,
}

impl LabelState {
    fn apply_set(
        &mut self,
        flag: &model::extensions::ExtLabelFlag,
        value: i32,
        block_id: u32,
    ) -> Result<(), InterpreterError> {
        use model::extensions::ExtLabelFlag as F;
        // Counter and ONCE accept any i32; everything else must be 0 or 1.
        match flag {
            F::Counter(c) => {
                *self.counter_mut(c) = value;
                return Ok(());
            }
            F::Once => {
                self.block_labels.once = match value {
                    0 => super::Once::Always,
                    1 => super::Once::First,
                    _ => super::Once::Last,
                };
                return Ok(());
            }
            _ => {}
        }
        let on = match value {
            0 => false,
            1 => true,
            _ => {
                return Err(InterpreterError::FlagSetNonBoolean {
                    block_id,
                    flag: flag.to_string(),
                    value,
                });
            }
        };
        match flag {
            F::Nav => self.adc_labels.nav = on,
            F::Rev => self.adc_labels.rev = on,
            F::Sms => self.adc_labels.sms = on,
            F::Ref => self.adc_labels.ref_ = on,
            F::Ima => self.adc_labels.ima = on,
            F::Off => self.adc_labels.off = on,
            F::Noise => self.adc_labels.noise = on,
            F::Pmc => self.block_labels.pmc = on,
            F::NoRot => self.no_rot = on,
            F::NoPos => self.no_pos = on,
            F::NoScl => self.no_scl = on,
            // Counters / Once handled above by early-return.
            F::Counter(_) | F::Once => unreachable!(),
        }
        Ok(())
    }

    fn apply_inc(&mut self, counter: &model::extensions::ExtLabelCounter, value: i32) {
        let target = self.counter_mut(counter);
        *target = target.wrapping_add(value);
    }

    fn counter_mut(&mut self, counter: &model::extensions::ExtLabelCounter) -> &mut i32 {
        use model::extensions::ExtLabelCounter as C;
        match counter {
            C::Slc => &mut self.adc_labels.slc,
            C::Seg => &mut self.adc_labels.seg,
            C::Rep => &mut self.adc_labels.rep,
            C::Avg => &mut self.adc_labels.avg,
            C::Set => &mut self.adc_labels.set,
            C::Eco => &mut self.adc_labels.eco,
            C::Phs => &mut self.adc_labels.phs,
            C::Lin => &mut self.adc_labels.lin,
            C::Par => &mut self.adc_labels.par,
            C::Acq => &mut self.adc_labels.acq,
            C::Trid => &mut self.block_labels.trid,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]
    use super::*;

    fn shape(time: Vec<f64>, amp: Vec<f64>, duration: f64) -> Arc<crate::interp::Shape<f64>> {
        Arc::new(crate::interp::Shape {
            time,
            amp,
            duration,
        })
    }

    /// Zeroth moment of a resampled output gradient over its active extent, with
    /// the IR's hold-flat-before-first/after-last convention (mirrors
    /// `waveform::partial_integral`). Used to check that `rotate_resample`'s output
    /// integrates to the analytic mix.
    fn moment(g: &crate::interp::Gradient) -> f64 {
        let (t, a) = (&g.shape.time, &g.shape.amp);
        let n = t.len();
        let mut acc = a[0] * t[0];
        for i in 1..n {
            acc += (a[i - 1] + a[i]) * 0.5 * (t[i] - t[i - 1]);
        }
        acc += a[n - 1] * (g.shape.duration - t[n - 1]);
        g.amp * acc
    }

    /// `axis_value` zeros outside the active window and otherwise evaluates
    /// `amp · shape(t − delay)` with the IR hold convention: flat before the first
    /// sample, linear between, flat after the last, then a hard cut at the
    /// active-window end.
    #[test]
    fn axis_value_holds_inside_and_zeros_outside() {
        // amp 2, delay 0; shape held at 4 on [0,1], 4→8 on [1,2], held at 8 on [2,3].
        let lk = Some((2.0, 0.0, shape(vec![1.0, 2.0], vec![4.0, 8.0], 3.0)));
        assert_eq!(axis_value(&lk, -0.1), 0.0); // before the window
        assert_eq!(axis_value(&lk, 0.0), 8.0); // held leading (2·4)
        assert_eq!(axis_value(&lk, 1.5), 12.0); // linear interior (2·6)
        assert_eq!(axis_value(&lk, 2.5), 16.0); // held trailing (2·8)
        assert_eq!(axis_value(&lk, 3.0), 16.0); // at the active-window end
        assert_eq!(axis_value(&lk, 3.1), 0.0); // past the window
        let absent: Option<GradLookup> = None;
        assert_eq!(axis_value(&absent, 1.0), 0.0); // an absent axis
    }

    /// A 90°-about-z rotation mixing a readout-shaped x trapezoid with a
    /// differently-shaped, differently-delayed y triangle — the general path: the
    /// axes share no Arc, so they are resampled onto their union breakpoint grid
    /// and mixed per sample. Two analytic invariants: (1) the zeroth moment
    /// commutes with the transform, `∫(M·g) = M·∫g` (exact here because both
    /// inputs start and end at zero), and (2) the mixed waveform is reproduced
    /// pointwise.
    #[test]
    fn rotate_resample_mixes_differently_shaped_axes() {
        // R_z(90°) as a gradient transform: out_x = −in_y, out_y = in_x, out_z = in_z.
        let m = [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];

        // gx: unit trapezoid (area 2) × amp 2 ⇒ ∫ = 4; delay 0, window [0,3].
        let gx = (
            2.0,
            0.0,
            shape(vec![0.0, 1.0, 2.0, 3.0], vec![0.0, 1.0, 1.0, 0.0], 3.0),
        );
        // gy: triangle (area 1) × amp 3 ⇒ ∫ = 3; delay 0.5, window [0.5,2.5].
        let gy = (
            3.0,
            0.5,
            shape(vec![0.0, 1.0, 2.0], vec![0.0, 1.0, 0.0], 2.0),
        );
        let lookups = [Some(gx), Some(gy), None];

        let (ox, oy, oz) = rotate_resample(&lookups, m);
        let ox = ox.expect("out x is non-zero");
        let oy = oy.expect("out y is non-zero");
        assert!(oz.is_none(), "no z input projects to z under R_z(90°)");

        // (1) Moments: ∫g = [4, 3, 0] ⇒ M·∫g = [−3, 4, 0].
        assert!(
            (moment(&ox) + 3.0).abs() < 1e-12,
            "∫(out x) = {}",
            moment(&ox)
        );
        assert!(
            (moment(&oy) - 4.0).abs() < 1e-12,
            "∫(out y) = {}",
            moment(&oy)
        );

        // The output spans the union window and folds the physical waveform into
        // the shape (amp = 1).
        assert_eq!(ox.delay, 0.0);
        assert!((ox.shape.duration - 3.0).abs() < 1e-12);
        assert_eq!(ox.amp, 1.0);

        // (2) Pointwise at t = 1.0 (a grid breakpoint): in = [2·1, 3·0.5, 0] =
        //     [2, 1.5, 0] ⇒ out_x = −1.5, out_y = 2.0.
        assert!((ox.shape.interpolate(1.0) + 1.5).abs() < 1e-12);
        assert!((oy.shape.interpolate(1.0) - 2.0).abs() < 1e-12);
    }
}
