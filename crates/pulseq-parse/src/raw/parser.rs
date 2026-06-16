//! Parser for the Pulseq 1.5 file grammar.
//!
//! This validator targets Pulseq 1.5+ only, so a single grammar lives here.
//! (Earlier 1.2–1.4 dialects — Delay events, implicit gradient boundaries,
//! optional rasters, the pTx `shim_id` columns — were dropped; see NOTICE.)
//! `raw::parse_file` rejects any other version with `UnsupportedVersion`.

use winnow::ascii::{alphanumeric1, till_line_ending};
use winnow::combinator::{alt, cut_err, delimited, empty, opt, preceded, repeat, seq, terminated};
use winnow::error::StrContext;
use winnow::prelude::*;
use winnow::token::one_of;

use super::{helpers::*, *};

pub fn file(input: &mut &str) -> ModalResult<Vec<Section>> {
    repeat(
        0..,
        preceded(
            opt(nl),
            alt((
                version.map(Section::Version),
                definitions.map(Section::Definitions),
                blocks.map(Section::Blocks),
                rfs.map(Section::Rfs),
                gradients.map(Section::Gradients),
                traps.map(Section::Traps),
                adcs.map(Section::Adcs),
                alt((
                    extension_refs.map(Section::ExtensionRefs),
                    extension_specs.map(Section::ExtensionSpecs),
                    shapes.map(Section::Shapes),
                    signature.map(Section::Signature),
                )),
            )),
        ),
    )
    .parse_next(input)
}

pub fn version(input: &mut &str) -> ModalResult<Version> {
    seq! { Version {
        _: tag_nl("[VERSION]"),
        major: cut_err(delimited(tag_ws("major"), int, nl)),
        minor: cut_err(delimited(tag_ws("minor"), int, nl)),
        revision: cut_err(preceded(tag_ws("revision"), int)),
        rev_suppl: cut_err(terminated(opt(ident), nl))
    }}
    .context(StrContext::Label("[VERSION] section"))
    .parse_next(input)
}

pub fn definitions(input: &mut &str) -> ModalResult<Vec<(String, String)>> {
    let def = (
        ident,
        ws,
        till_line_ending.map(|s: &str| s.trim().to_owned()),
        nl,
    )
        .map(|(key, _, value, _)| (key, value));

    preceded(tag_nl("[DEFINITIONS]"), repeat(0.., def))
        .context(StrContext::Label("[DEFINITIONS] section"))
        .parse_next(input)
}

pub fn signature(input: &mut &str) -> ModalResult<Signature> {
    let mut typ = cut_err(delimited(
        tag_ws("Type"),
        alphanumeric1.map(|s: &str| s.to_owned()),
        nl,
    ));
    let mut hash = cut_err(delimited(
        tag_ws("Hash"),
        till_line_ending.map(|s: &str| s.trim().to_owned()),
        nl,
    ));

    seq! { Signature {
        _: tag_nl("[SIGNATURE]"),
        typ: typ,
        hash: hash,
    }}
    .context(StrContext::Label("[SIGNATURE] section"))
    .parse_next(input)
}

pub fn blocks(input: &mut &str) -> ModalResult<Vec<Block>> {
    let block = seq! { Block {
        id: int,
        // Block duration in `BlockDurationRaster` ticks (1.4+ semantics).
        dur: cut_err(int),
        rf: cut_err(int),
        gx: cut_err(int),
        gy: cut_err(int),
        gz: cut_err(int),
        adc: cut_err(int),
        ext: cut_err(int),
        _: cut_err(nl),
    }}
    .context(StrContext::Label("block record"));

    preceded(tag_nl("[BLOCKS]"), repeat(0.., block))
        .context(StrContext::Label("[BLOCKS] section"))
        .parse_next(input)
}

pub fn rfs(input: &mut &str) -> ModalResult<Vec<Rf>> {
    let rf = seq! {Rf {
        id: int,
        amp: cut_err(float),
        mag_id: cut_err(int),
        phase_id: cut_err(int),
        time_id: cut_err(signed_int),
        center: cut_err(float).map(|x| Some(x * 1e-6)),
        delay: cut_err(int).map(|x: u32| x as f64 * 1e-6),
        freq_rel: cut_err(float).map(|x| x * 1e-6),
        phase_rel: cut_err(float).map(|x| x * 1e-6),
        freq_off: cut_err(float),
        phase_off: cut_err(float),
        // 1.5 carries pTx shims via the `rf_shims` extension, not RF columns.
        shim_id: empty.value(None),
        rf_use: cut_err(preceded(ws, one_of(['e', 'r', 'i', 's', 'p', 'o', 'u']))).map(
            |c| match c {
                'e' => RfUse::Excitation,
                'r' => RfUse::Refocusing,
                'i' => RfUse::Inversion,
                's' => RfUse::Saturation,
                'p' => RfUse::Preparation,
                'o' => RfUse::Other,
                'u' => RfUse::Undefined,
                _ => unreachable!("one_of restricts to listed chars"),
            },
        ),
        _: cut_err(nl),
    }}
    .context(StrContext::Label("rf record"));

    preceded(tag_nl("[RF]"), repeat(0.., rf))
        .context(StrContext::Label("[RF] section"))
        .parse_next(input)
}

pub fn gradients(input: &mut &str) -> ModalResult<Vec<Gradient>> {
    let grad = seq! {Gradient {
        id: int,
        amp: cut_err(float),
        // 1.5 always stores explicit boundary amplitudes (Hz/m).
        first: cut_err(float),
        last: cut_err(float),
        shape_id: cut_err(int),
        time_id: cut_err(signed_int),
        delay: cut_err(int).map(|d: u32| d as f64 * 1e-6),
        _: cut_err(nl),
    }}
    .context(StrContext::Label("gradient record"));

    preceded(tag_nl("[GRADIENTS]"), repeat(0.., grad))
        .context(StrContext::Label("[GRADIENTS] section"))
        .parse_next(input)
}

pub fn traps(input: &mut &str) -> ModalResult<Vec<Trap>> {
    let trap = seq! {Trap {
        id: int,
        amp: cut_err(float),
        rise: cut_err(int).map(|d: u32| d as f64 * 1e-6),
        flat: cut_err(int).map(|d: u32| d as f64 * 1e-6),
        fall: cut_err(int).map(|d: u32| d as f64 * 1e-6),
        delay: cut_err(int).map(|d: u32| d as f64 * 1e-6),
        _: cut_err(nl),
    }}
    .context(StrContext::Label("trap record"));

    preceded(tag_nl("[TRAP]"), repeat(0.., trap))
        .context(StrContext::Label("[TRAP] section"))
        .parse_next(input)
}

pub fn adcs(input: &mut &str) -> ModalResult<Vec<Adc>> {
    let adc = seq! {Adc {
        id: int,
        num: cut_err(int),
        dwell: cut_err(float).map(|d: f64| d * 1e-9),
        delay: cut_err(int).map(|d: u32| d as f64 * 1e-6),
        freq_rel: cut_err(float).map(|x| x * 1e-6),
        phase_rel: cut_err(float).map(|x| x * 1e-6),
        freq_off: cut_err(float),
        phase_off: cut_err(float),
        phase_shape_id: cut_err(int),
        _: cut_err(nl),
    }}
    .context(StrContext::Label("adc record"));

    preceded(tag_nl("[ADC]"), repeat(0.., adc))
        .context(StrContext::Label("[ADC] section"))
        .parse_next(input)
}

// [EXTENSIONS] section format:
//
// `ExtensionRef` defined by 4 numbers: "<id> <type> <ref> <next>"
// (introduced by the `[EXTENSIONS]` header).
//
// Followed by extension specifications. Each `ExtensionSpec` starts with
// `extension <STRING_ID> <type>` and is followed by a list of `ExtensionObject`
// instances - one line, `<id>` + extension specific data.
//
// In the spec all refs of one [EXTENSIONS] block come before all specs of that
// block, but we don't enforce that here: refs are parsed by `extension_refs`
// (anchored on the [EXTENSIONS] header), and each `extension <NAME> <ID>` block
// is parsed independently by `extension_specs`. The downstream conversion
// merges them via the SectionData pipeline, so interleaved or repeated blocks
// just concatenate.

pub fn extension_refs(input: &mut &str) -> ModalResult<Vec<ExtensionRef>> {
    let ext_ref = seq! { ExtensionRef {
        id: int,
        spec_id: cut_err(int),
        obj_id: cut_err(int),
        next: cut_err(int),
        _: cut_err(nl),
    }}
    .context(StrContext::Label("extension reference"));

    preceded(tag_nl("[EXTENSIONS]"), repeat(0.., ext_ref))
        .context(StrContext::Label("[EXTENSIONS] section"))
        .parse_next(input)
}

pub fn extension_specs(input: &mut &str) -> ModalResult<Vec<ExtensionSpec>> {
    let ext_obj = || {
        seq! { ExtensionObject {
            id: int,
            data: till_line_ending.map(|s: &str| s.trim().to_owned()),
            _: cut_err(nl),
        }}
        .context(StrContext::Label("extension object"))
    };

    let ext_spec = seq! { ExtensionSpec {
        _: tag_ws("extension"),
        name: cut_err(ident),
        id: cut_err(int),
        _: cut_err(nl),
        instances: cut_err(repeat(1.., ext_obj())),
    }}
    .context(StrContext::Label("extension specification"));

    repeat(1.., ext_spec)
        .context(StrContext::Label("extension specifications"))
        .parse_next(input)
}

pub fn raw_shape(input: &mut &str) -> ModalResult<(u32, (u32, Vec<f64>))> {
    // The spec and the exporter use different tags, we allow both.
    let shape_id = || delimited(alt((tag_ws("Shape_ID"), tag_ws("shape_id"))), int, nl);
    let num_samples = || {
        delimited(
            alt((tag_ws("Num_Uncompressed"), tag_ws("num_samples"))),
            int,
            nl,
        )
    };
    let samples = || repeat(0.., terminated(float, nl));

    seq!((shape_id(), cut_err((num_samples(), samples()))))
        .context(StrContext::Label("shape"))
        .parse_next(input)
}

pub fn shapes(input: &mut &str) -> ModalResult<Vec<Shape>> {
    let shape = raw_shape.try_map(|(id, (num_samples, samples))| {
        if samples.len() == num_samples as usize {
            Ok(Shape { id, samples })
        } else {
            decompress_shape(samples, num_samples).map(|samples| Shape { id, samples })
        }
    });

    preceded(tag_nl("[SHAPES]"), repeat(0.., shape))
        .context(StrContext::Label("[SHAPES] section"))
        .parse_next(input)
}
