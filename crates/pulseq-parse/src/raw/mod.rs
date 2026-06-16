use std::fmt::Display;

use crate::error;

pub mod helpers;
mod parser;

// Pulseq 1.5 is parsed into the structs below. This validator targets Pulseq
// 1.5+ only; earlier dialects (1.2–1.4) are not parsed — see `parser` and
// NOTICE for what was dropped.

pub fn parse_file(source: &str) -> Result<Vec<Section>, error::ParseError> {
    // parse file twice, first time to only see version, second time below for full parse
    use winnow::combinator::{opt, preceded};
    use winnow::prelude::*;
    let mut tmp = source;
    let version = preceded(opt(helpers::nl), parser::version).parse_next(&mut tmp)?;

    match version {
        Version {
            major: 1, minor: 5, ..
        } => Ok(parser::file.parse(source)?),
        _ => Err(error::ParseError::UnsupportedVersion(version)),
    }
}

#[derive(Debug)]
pub enum Section {
    Version(Version),
    Signature(Signature),
    Definitions(Vec<(String, String)>),
    Blocks(Vec<Block>),
    Rfs(Vec<Rf>),
    Gradients(Vec<Gradient>),
    Traps(Vec<Trap>),
    Adcs(Vec<Adc>),
    ExtensionRefs(Vec<ExtensionRef>),
    ExtensionSpecs(Vec<ExtensionSpec>),
    Shapes(Vec<Shape>),
}

#[derive(Debug)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub revision: u32,
    pub rev_suppl: Option<String>,
}

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}{}",
            self.major,
            self.minor,
            self.revision,
            self.rev_suppl.as_deref().unwrap_or("")
        )
    }
}

#[derive(Debug)]
pub struct Signature {
    pub typ: String,
    pub hash: String,
}

#[derive(Debug)]
pub struct Block {
    pub id: u32,
    /// Block duration in `BlockDurationRaster` ticks.
    pub dur: u32,
    pub rf: u32,
    pub gx: u32,
    pub gy: u32,
    pub gz: u32,
    pub adc: u32,
    pub ext: u32,
}

#[derive(Debug)]
pub struct Rf {
    pub id: u32,
    /// `Hz`
    pub amp: f64,
    pub mag_id: u32,
    pub phase_id: u32,
    /// Sentinel values: `0` = uniform centers `[0.5, 1.5, …, N-0.5]`,
    /// `-1` (pulseq 1.5+) = half-tick grid `[0.5, 1.0, 1.5, …, N-0.5]` with
    /// `M = 2N-1` samples, positive = id of a custom time shape.
    pub time_id: i32,
    /// `s` (from pulseq: `us`)
    pub center: Option<f64>,
    /// `s` (from pulseq: `us`)
    pub delay: f64,
    /// relative to system frequency
    pub freq_rel: f64,
    /// offset to system frequency
    pub phase_rel: f64,
    /// `Hz` (offset to system frequency)
    pub freq_off: f64,
    /// `rad` (offset to system frequency)
    pub phase_off: f64,
    /// shim_mag_ID, shim_phase_ID
    pub shim_id: Option<(u32, u32)>,
    /// use - parsed from initial char of use identifier
    pub rf_use: RfUse,
}

#[derive(Debug, Clone, Copy)]
pub enum RfUse {
    Excitation,
    Refocusing,
    Inversion,
    Saturation,
    Preparation,
    Other,
    Undefined,
}

#[derive(Debug)]
pub struct Gradient {
    pub id: u32,
    /// `Hz/m`
    pub amp: f64,
    /// `Hz/m` - amplitude at the start of the gradient (explicit since 1.5)
    pub first: f64,
    /// `Hz/m` - amplitude at the end of the gradient (explicit since 1.5)
    pub last: f64,
    pub shape_id: u32,
    /// Sentinel values: `0` = uniform centers `[0.5, 1.5, …, N-0.5]`,
    /// `-1` (pulseq 1.5+) = half-tick grid `[0.5, 1.0, 1.5, …, N-0.5]` with
    /// `M = 2N-1` samples, positive = id of a custom time shape.
    pub time_id: i32,
    /// `s` (from pulseq: `us`)
    pub delay: f64,
}

#[derive(Debug)]
pub struct Trap {
    pub id: u32,
    /// `Hz/m`
    pub amp: f64,
    /// `s` (from pulseq: `us`)
    pub rise: f64,
    /// `s` (from pulseq: `us`)
    pub flat: f64,
    /// `s` (from pulseq: `us`)
    pub fall: f64,
    /// `s` (from pulseq: `us`)
    pub delay: f64,
}

#[derive(Debug)]
pub struct Adc {
    pub id: u32,
    pub num: u32,
    /// `s` (from pulseq: `ns`)
    pub dwell: f64,
    /// `s` (from pulseq: `us`)
    pub delay: f64,
    /// relative to system frequency
    pub freq_rel: f64,
    /// relative to system frequency
    pub phase_rel: f64,
    /// `Hz` (offset to system frequency)
    pub freq_off: f64,
    /// `rad` (offset to system frequency)
    pub phase_off: f64,
    /// optional per-sample adc phase - WIP: no examples found
    pub phase_shape_id: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct ExtensionRef {
    pub id: u32,
    pub spec_id: u32,
    pub obj_id: u32,
    pub next: u32,
}

#[derive(Debug)]
pub struct ExtensionSpec {
    pub id: u32,
    pub name: String,
    pub instances: Vec<ExtensionObject>,
}

#[derive(Debug)]
pub struct ExtensionObject {
    pub id: u32,
    pub data: String,
}

#[derive(Debug)]
pub struct Shape {
    pub id: u32,
    pub samples: Vec<f64>,
}
