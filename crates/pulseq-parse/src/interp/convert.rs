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
            block.id,
        )?;

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
/// the result matches per-axis scaling. With a non-identity rotation every
/// present input must share the same `interp::Shape` Arc (after `ShapeLib`
/// lookup) and the same `delay`; otherwise we error with `UnsupportedRotation`.
/// `None` axes can become `Some` if rotation projects onto them.
fn transform_grad(
    gx: Option<&model::Gradient>,
    gy: Option<&model::Gradient>,
    gz: Option<&model::Gradient>,
    transform: Transform,
    grad_raster: f64,
    shapes: &mut ShapeLib,
    block_id: u32,
) -> Result<GradAxes, InterpreterError> {
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
        return Ok((emit(lx), emit(ly), emit(lz)));
    }

    // Pick the first present axis as the reference. If nothing is present
    // there's no gradient — rotation has nothing to project from.
    let Some((_, ref_delay, ref_shape)) = lookups.iter().find_map(|opt| opt.as_ref()).cloned()
    else {
        return Ok((None, None, None));
    };

    // Every other present axis must agree on shape and delay. Arc::ptr_eq
    // catches Trap+Free mixes, mismatched trap timings, and different free
    // shapes uniformly because ShapeLib gives the same Arc only for inputs
    // that share both kind and parameters.
    for opt in lookups.iter().flatten() {
        let (_, delay, shape) = opt;
        if *delay != ref_delay || !Arc::ptr_eq(shape, &ref_shape) {
            return Err(InterpreterError::UnsupportedRotation { block_id });
        }
    }

    let amps = [
        lookups[0].as_ref().map_or(0.0, |(a, _, _)| *a),
        lookups[1].as_ref().map_or(0.0, |(a, _, _)| *a),
        lookups[2].as_ref().map_or(0.0, |(a, _, _)| *a),
    ];
    let m = transform.to_grad_transform();
    let out = [
        m[0][0] * amps[0] + m[0][1] * amps[1] + m[0][2] * amps[2],
        m[1][0] * amps[0] + m[1][1] * amps[1] + m[1][2] * amps[2],
        m[2][0] * amps[0] + m[2][1] * amps[1] + m[2][2] * amps[2],
    ];

    let emit = |amp: f64| {
        if amp == 0.0 {
            None
        } else {
            Some(super::Gradient {
                amp,
                delay: ref_delay,
                shape: ref_shape.clone(),
            })
        }
    };
    Ok((emit(out[0]), emit(out[1]), emit(out[2])))
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
