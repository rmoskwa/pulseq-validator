use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use super::Sequence;
use crate::{error::ConversionError, model, raw};

mod definitions;
mod sections;
mod shape_lib;

use definitions::Defs;
use sections::{SectionData, get_section_data};
use shape_lib::ShapeLib;

pub fn from_raw(mut sections: Vec<raw::Section>) -> Result<Sequence, ConversionError> {
    // Validate exactly one [VERSION] section (parse_file already gated on 1.5).
    let [_version]: [raw::Version; 1] = get_section_data(&mut sections)
        .try_into()
        .map_err(|v: Vec<raw::Version>| ConversionError::VersionSectionCount(v.len()))?;

    let defs = Defs::from_raw(get_section_data(&mut sections))?;
    check_ext_support(&defs.required_exts)?;

    let raw_shapes: HashMap<u32, Arc<Vec<f64>>> =
        map_section_data(&mut sections, |shape: raw::Shape| Ok((shape.id, Arc::new(shape.samples))))?;
    let mut shapes = ShapeLib::new(raw_shapes)?;

    let adcs = map_section_data(&mut sections, |adc: raw::Adc| {
        // SPEC NOTE: pulseq has no `time_id` for ADC phase shapes; the array
        // is sampled per ADC sample at `dwell`. We pass `time_id = 0` so the
        // shape gets a synthesised `time = (1..=n)` matching every other
        // uniform shape, letting downstream code treat shapes uniformly.
        let phase_shape = if adc.phase_shape_id == 0 {
            None
        } else {
            Some(shapes.get(adc.phase_shape_id, 0)?)
        };
        Ok((
            adc.id,
            Arc::new(model::Adc {
                num: adc.num,
                dwell: adc.dwell,
                delay: adc.delay,
                freq: (adc.freq_rel, adc.freq_off),
                phase: (adc.phase_rel, adc.phase_off),
                phase_shape,
            }),
        ))
    })?;

    let rfs = map_section_data(&mut sections, |rf: raw::Rf| {
        let shape = shapes.get_complex(rf.mag_id, rf.phase_id, rf.time_id)?;
        let shim_shape = match rf.shim_id {
            Some((mag_id, phase_id)) => Some(shapes.get_complex(mag_id, phase_id, 0)?),
            None => None,
        };
        let center = rf
            .center
            .unwrap_or_else(|| rf.delay + defs.time_raster.rf * shape.calc_center() as f64);

        Ok((
            rf.id,
            Arc::new(model::Rf {
                amp: rf.amp,
                phase: (rf.phase_rel, rf.phase_off),
                shape,
                delay: rf.delay,
                center,
                freq: (rf.freq_rel, rf.freq_off),
                shim_shape,
                rf_use: rf.rf_use,
            }),
        ))
    })?;

    // 1.5 gradients carry explicit boundary amplitudes (first/last), so each
    // converts independently — no file-order back-fill of `prev_last` needed.
    let mut gradients: HashMap<u32, Arc<model::Gradient>> =
        map_section_data(&mut sections, |grad: raw::Gradient| {
            let (first_norm, last_norm) = if grad.amp != 0.0 {
                (grad.first / grad.amp, grad.last / grad.amp)
            } else {
                (0.0, 0.0)
            };
            let shape =
                shapes.get_with_boundaries(grad.shape_id, grad.time_id, first_norm, last_norm)?;
            Ok((
                grad.id,
                Arc::new(model::Gradient::Free {
                    amp: grad.amp,
                    shape,
                    delay: grad.delay,
                }),
            ))
        })?;

    let traps: HashMap<u32, Arc<model::Gradient>> = map_section_data(&mut sections, |trap: raw::Trap| {
        Ok((
            trap.id,
            Arc::new(model::Gradient::Trap {
                amp: trap.amp,
                rise: trap.rise,
                flat: trap.flat,
                fall: trap.fall,
                delay: trap.delay,
            }),
        ))
    })?;

    // Gradients and Traps share an id namespace.
    let count = gradients.len() + traps.len();
    gradients.extend(traps);
    if gradients.len() < count {
        return Err(ConversionError::GradTrapIdReuse);
    }

    let ext_refs: Vec<raw::ExtensionRef> = get_section_data(&mut sections);
    let ext_specs: Vec<raw::ExtensionSpec> = get_section_data(&mut sections);
    let (exts, soft_delay_hints) = convert_exts(ext_refs, ext_specs)?;

    // We do not use map_section_data here since we do not care about block ids
    let raw_blocks: Vec<raw::Block> = get_section_data(&mut sections);
    let blocks = raw_blocks
        .into_iter()
        .map(|block: raw::Block| {
            convert_block(block, &rfs, &gradients, &adcs, &defs.time_raster, &exts)
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Sequence {
        name: defs.name,
        fov: defs.fov,
        definitions: defs.defs,
        time_raster: defs.time_raster,
        blocks,
        soft_delay_hints,
    })
}

fn check_ext_support(required: &[String]) -> Result<(), ConversionError> {
    for ext in required {
        match ext.as_str() {
            "label" | "labelset" | "labelinc" | "triggers" | "delays" | "rotations"
            | "rf_shims" => (),
            _ => return Err(ConversionError::UnsupportedExtension(ext.to_owned())),
        }
    }
    Ok(())
}

/// Wrapper around get_section_data that applies a mapping func and hashes by id.
fn map_section_data<T, Val, F>(
    sections: &mut Vec<raw::Section>,
    f: F,
) -> Result<HashMap<u32, Val>, ConversionError>
where
    T: SectionData,
    F: FnMut(T) -> Result<(u32, Val), ConversionError>,
{
    let data: Vec<T> = get_section_data(sections);
    let raw_count = data.len();
    let data = data
        .into_iter()
        .map(f)
        .collect::<Result<HashMap<u32, Val>, ConversionError>>()?;

    if data.len() < raw_count {
        Err(ConversionError::EventIdReuse(T::SECTION_TYPE))
    } else {
        Ok(data)
    }
}

/// Output of `convert_exts`: parsed extensions keyed by ext-ref id, plus the
/// soft-delay `id → hint` table collected from `delays` specs.
type ConvertedExts = (HashMap<u32, Vec<model::Extension>>, HashMap<u32, String>);

fn convert_exts(
    ext_refs: Vec<raw::ExtensionRef>,
    ext_specs: Vec<raw::ExtensionSpec>,
) -> Result<ConvertedExts, ConversionError> {
    // Walk every (spec, obj) pair, parsing the extension and - for `delays`
    // specs - capturing the hint into a single sequence-level table. Hints
    // are not stored on `Extension::Delay` itself (see seq/extensions.rs).
    let mut parsed_specs: HashMap<(u32, u32), model::Extension> = HashMap::new();
    let mut soft_delay_hints: HashMap<u32, String> = HashMap::new();
    for spec in &ext_specs {
        for obj in &spec.instances {
            let ext = model::Extension::parse(&spec.name, &obj.data)?;

            if let model::Extension::Delay { id, .. } = &ext {
                // parse_delay validated 4 whitespace-separated fields, so
                // parts[3] (the hint) is guaranteed to exist.
                let hint = obj
                    .data
                    .split_whitespace()
                    .nth(3)
                    .unwrap_or_default()
                    .to_owned();
                match soft_delay_hints.entry(*id) {
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(hint);
                    }
                    std::collections::hash_map::Entry::Occupied(slot) => {
                        if slot.get() != &hint {
                            return Err(crate::error::SoftDelayHintConflict {
                                id: *id,
                                hint_a: slot.get().clone(),
                                hint_b: hint,
                            }
                            .into());
                        }
                    }
                }
            }

            parsed_specs.insert((spec.id, obj.id), ext);
        }
    }
    let ext_specs = parsed_specs;

    // Transform flat list of extension refs into a HashMap indexed by their id
    let ext_ref_count = ext_refs.len();
    let ext_refs: HashMap<u32, _> = ext_refs.iter().map(|ext| (ext.id, *ext)).collect();
    if ext_refs.len() < ext_ref_count {
        return Err(ConversionError::EventIdReuse(
            raw::ExtensionRef::SECTION_TYPE,
        ));
    }

    // Each ref starts a linked list of extensions, terminated by `next == 0`.
    // Walk every chain, until the end is reached or a cycle detected.
    // Build a HashMap with key=ref_id, value=Vec<Extension>
    let mut parsed = HashMap::with_capacity(ext_refs.len());

    for root_ref in ext_refs.values() {
        // Keep a list of visited references - if we see one again we have a cycle
        let mut visited: HashSet<u32> = HashSet::new();
        // Convert the linked list of references into a Vec<Extensions>
        let mut ext_list = Vec::new();

        let mut ext_ref = root_ref;
        loop {
            // Return error on cycle
            if !visited.insert(ext_ref.id) {
                return Err(ConversionError::ExtensionRefCycle {
                    start_id: root_ref.id,
                });
            }

            // Convert the reference into a parsed extension and push to Vec
            let ext = ext_specs
                .get(&(ext_ref.spec_id, ext_ref.obj_id))
                .ok_or(ConversionError::InvalidExtensionRef { id: ext_ref.id })?;
            ext_list.push(ext.clone());

            // Break on end-of-linked-list
            if ext_ref.next == 0 {
                break;
            }
            // Otherwise move on to next element
            ext_ref = ext_refs
                .get(&ext_ref.next)
                .ok_or(ConversionError::InvalidExtensionRef { id: ext_ref.next })?;
        }
        parsed.insert(root_ref.id, ext_list);
    }
    Ok((parsed, soft_delay_hints))
}

fn convert_block(
    block: raw::Block,
    rfs: &HashMap<u32, Arc<model::Rf>>,
    gradients: &HashMap<u32, Arc<model::Gradient>>,
    adcs: &HashMap<u32, Arc<model::Adc>>,
    time_raster: &model::TimeRaster,
    exts: &HashMap<u32, Vec<model::Extension>>,
) -> Result<model::Block, ConversionError> {
    let err = |ty, id| ConversionError::BrokenRef { ty, id };
    use super::EventType::*;

    let rf = (block.rf != 0)
        .then(|| rfs.get(&block.rf).cloned().ok_or(err(Rf, block.rf)))
        .transpose()?;
    let gx = (block.gx != 0)
        .then(|| gradients.get(&block.gx).cloned().ok_or(err(Gx, block.gx)))
        .transpose()?;
    let gy = (block.gy != 0)
        .then(|| gradients.get(&block.gy).cloned().ok_or(err(Gy, block.gy)))
        .transpose()?;
    let gz = (block.gz != 0)
        .then(|| gradients.get(&block.gz).cloned().ok_or(err(Gz, block.gz)))
        .transpose()?;
    let adc = (block.adc != 0)
        .then(|| adcs.get(&block.adc).cloned().ok_or(err(Adc, block.adc)))
        .transpose()?;

    // Since 1.4 the block stores its duration directly (in block-raster ticks).
    let duration = block.dur as f64 * time_raster.block;

    let ext = if block.ext != 0 {
        exts.get(&block.ext)
            .cloned()
            .ok_or(ConversionError::InvalidExtensionRef { id: block.ext })?
    } else {
        Vec::new()
    };

    Ok(model::Block {
        id: block.id,
        duration,
        rf,
        gx,
        gy,
        gz,
        adc,
        ext,
    })
}
