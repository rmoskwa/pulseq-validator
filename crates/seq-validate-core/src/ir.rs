//! The validator's stable interpreted IR, built on the `pulseq-parse` parser.
//!
//! pulseq-parse lowers a `.seq` file through three layers: `raw` (faithful event
//! tables) → `model` (validated, deduplicated, `Arc`-shared events) → `interp`
//! (the *interpreted* sequence — what the scanner actually plays: FOV-scaled and
//! rotated gradients, decompressed shapes, larmor-folded RF/ADC freq+phase,
//! resolved soft delays, per-ADC label snapshots). The `interp` layer is our IR.
//!
//! This module is a thin *file facade* over the interpreted layer. Interpreted
//! timing — the **absolute start time of each block** and the total duration —
//! lives on the `interp` layer itself (`interp::Sequence::{block_starts, duration}`),
//! since it is a pure function of what the scanner plays; we surface it here.
//! What the facade genuinely adds is file provenance the lower layers drop: the
//! `[VERSION]` ([`Version`]) and the full `[DEFINITIONS]` table.
//!
//! The raw layer stays addressable via [`raw_sections`] for debugging and
//! round-trip inspection.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use pulseq_parse::{interp, model, raw};

// Re-export the interpreted event vocabulary so downstream checks have a single
// canonical path to the IR types.
pub use pulseq_parse::interp::{
    Adc, Block, BlockLabels, Gradient, Labels, Once, Rf, Shape, Transform, Trigger,
};

/// 1H Larmor frequency at 1 T, in Hz — the default used to fold relative and
/// offset RF/ADC freq+phase during interpretation. Only affects freq/phase
/// metrics, not geometry; override via [`Sequence::from_file_with`] when the
/// field strength matters.
pub const DEFAULT_LARMOR_HZ: f64 = 42_577_468.8;

/// Pulseq file format version, copied from the `[VERSION]` section. The `model`
/// and `interp` layers use it only to dispatch parsing and then discard it, so
/// the IR preserves it here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub revision: u32,
    /// Optional revision suffix (e.g. a `post1` tag); `None` for plain releases.
    pub suppl: Option<String>,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.{}{}",
            self.major,
            self.minor,
            self.revision,
            self.suppl.as_deref().unwrap_or("")
        )
    }
}

/// Event raster times, in seconds, from `[DEFINITIONS]`. Pre-1.4 files omit
/// these; the parser substitutes the Siemens-interpreter defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeRaster {
    /// Gradient raster.
    pub grad: f64,
    /// RF (radiofrequency) raster.
    pub rf: f64,
    /// ADC raster.
    pub adc: f64,
    /// Block-duration raster.
    pub block: f64,
}

/// The interpreted IR for one `.seq` file.
///
/// [`blocks`](Self::blocks) and [`starts`](Self::starts) are parallel vectors in
/// execution order: `starts[i]` is the absolute start time (seconds) of
/// `blocks[i]`. Use [`timed`](Self::timed) to iterate them together.
pub struct Sequence {
    /// File format version from `[VERSION]`.
    pub version: Version,
    /// `Name` definition, if present.
    pub name: Option<String>,
    /// Per-axis field of view in meters; `[1.0; 3]` if the file defined no FOV.
    pub fov: [f64; 3],
    /// Event raster times.
    pub time_raster: TimeRaster,
    /// The complete `[DEFINITIONS]` table, verbatim, sorted by key for stable
    /// snapshots. Recognized keys (`Name`, `FOV`, the rasters) are *also*
    /// surfaced as typed fields above; this map keeps the unabridged source.
    pub definitions: BTreeMap<String, String>,
    /// Interpreted blocks in execution order.
    pub blocks: Vec<Block>,
    /// Absolute start time (seconds) of each block; `starts[i]` aligns with
    /// `blocks[i]`. `starts[0]` is `0.0`.
    pub starts: Vec<f64>,
    /// Total sequence duration (seconds): the sum of all block durations.
    pub total_duration: f64,
    /// Non-fatal interpreter warnings raised during model→interp lowering.
    pub warnings: Vec<String>,
}

impl Sequence {
    /// Parse and interpret a `.seq` file using the default Larmor frequency
    /// ([`DEFAULT_LARMOR_HZ`]).
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        Self::from_file_with(path, DEFAULT_LARMOR_HZ)
    }

    /// Parse and interpret a `.seq` file with an explicit Larmor frequency [Hz].
    pub fn from_file_with<P: AsRef<Path>>(path: P, larmor: f64) -> Result<Self, Error> {
        let source = std::fs::read_to_string(path).map_err(|e| Error::Parse(e.into()))?;
        Self::from_source(&source, larmor)
    }

    /// Parse and interpret a `.seq` file from an in-memory string.
    pub fn from_source(source: &str, larmor: f64) -> Result<Self, Error> {
        // raw layer — faithful section tables. We lift two bits of file
        // provenance from here before the model conversion consumes them: the
        // `[VERSION]` and the *verbatim, unabridged* `[DEFINITIONS]` table. The
        // model layer keeps a definitions map too, but it promotes recognized
        // keys (Name, FOV, rasters) into typed fields and drops them from that
        // map, so only the raw layer still holds the complete table.
        let sections = raw::parse_file(source).map_err(|e| Error::Parse(e.into()))?;

        let version = sections
            .iter()
            .find_map(|s| match s {
                raw::Section::Version(v) => Some(Version {
                    major: v.major,
                    minor: v.minor,
                    revision: v.revision,
                    suppl: v.rev_suppl.clone(),
                }),
                _ => None,
            })
            .ok_or(Error::Missing("[VERSION] section"))?;

        let definitions: BTreeMap<String, String> = sections
            .iter()
            .find_map(|s| match s {
                raw::Section::Definitions(defs) => Some(defs.iter().cloned().collect()),
                _ => None,
            })
            .unwrap_or_default();

        // model layer — validated, deduplicated. Consumes `sections`.
        let seq = model::Sequence::from_parsed_file(sections).map_err(Error::Parse)?;
        let time_raster = TimeRaster {
            grad: seq.time_raster.grad,
            rf: seq.time_raster.rf,
            adc: seq.time_raster.adc,
            block: seq.time_raster.block,
        };

        // interp layer — the interpreted sequence. Identity transform: we validate
        // the file as authored (block-level rotation *extensions* are still
        // applied by the interpreter); no external FOV transform is imposed.
        let (int_seq, raw_warnings) =
            interp::Sequence::from_seq(&seq, Transform::default(), larmor, HashMap::new())
                .map_err(|e| Error::Interpret(e.to_string()))?;
        let warnings = raw_warnings.iter().map(|w| w.to_string()).collect();

        // Interpreted timing now lives on the `interp` layer (it is a pure
        // function of block durations — "what the scanner plays"). We surface it.
        let starts = int_seq.block_starts();
        let total_duration = int_seq.duration();

        let interp::Sequence { name, fov, blocks } = int_seq;

        Ok(Sequence {
            version,
            name,
            fov,
            time_raster,
            definitions,
            blocks,
            starts,
            total_duration,
            warnings,
        })
    }

    /// Iterate `(absolute_start_seconds, &block)` pairs in execution order.
    pub fn timed(&self) -> impl Iterator<Item = (f64, &Block)> {
        self.starts.iter().copied().zip(&self.blocks)
    }

    /// Absolute start time (seconds) of block `i`, or `None` if out of range.
    pub fn start(&self, i: usize) -> Option<f64> {
        self.starts.get(i).copied()
    }
}

/// Re-parse the **raw** layer (faithful, ID-indexed section tables) for
/// debugging or round-trip inspection. The interpreted [`Sequence`] is the
/// normal entry point; this exposes the unlowered tables when needed.
pub fn raw_sections(source: &str) -> Result<Vec<raw::Section>, Error> {
    raw::parse_file(source).map_err(|e| Error::Parse(e.into()))
}

/// Errors from building the IR.
#[derive(Debug)]
pub enum Error {
    /// Parse / conversion / validation / IO failure from the parser.
    Parse(pulseq_parse::Error),
    /// seq→int interpreter failure. Carried as a message because the
    /// interpreter's error type is not part of pulseq-parse's public API.
    Interpret(String),
    /// A required section was absent from an otherwise-parseable file.
    Missing(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(e) => write!(f, "{e}"),
            Error::Interpret(msg) => write!(f, "interpreter error: {msg}"),
            Error::Missing(what) => write!(f, "malformed .seq: missing {what}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Parse(e) => Some(e),
            _ => None,
        }
    }
}
