use std::fmt::Display;

use crate::raw::Version;
use thiserror::Error;

#[derive(Error, Debug)]
#[error("Soft-delay id {id} is used with conflicting hints: {hint_a:?} vs {hint_b:?}")]
pub struct SoftDelayHintConflict {
    pub id: u32,
    pub hint_a: String,
    pub hint_b: String,
}

#[derive(Error, Debug)]
pub enum InterpreterError {
    #[error(
        "Block #{block_id}: RF has both an `rf_shims` extension and a pTx \
         shim shape — only one shim source is allowed per RF"
    )]
    ConflictingShimSources { block_id: u32 },
    #[error(
        "Block #{block_id}: multiple `rf_shims` extension instances — only \
         one is allowed per block"
    )]
    MultipleShimmingExtensions { block_id: u32 },
    #[error(
        "Block #{block_id}: multiple `rotations` extension instances — only \
         one is allowed per block"
    )]
    MultipleRotationExtensions { block_id: u32 },
    #[error("Block #{block_id}: encountered a shim with zero channels")]
    EmptyShim { block_id: u32 },
    #[error(
        "Soft delay #{id} (hint {hint:?}) is referenced by the sequence but \
         no value was provided in the `soft_delays` input"
    )]
    MissingSoftDelay { id: u32, hint: String },
    #[error(
        "Block #{block_id}: LABELSET {flag} = {value}, but boolean flags only \
         accept 0 or 1"
    )]
    FlagSetNonBoolean {
        block_id: u32,
        flag: String,
        value: i32,
    },
    #[error(
        "Fov transform's 3x3 part must be a uniformly-scaled rotation — \
         orthogonal columns with equal, non-zero norms. Non-uniform scale, \
         shear, and degenerate matrices aren't yet supported."
    )]
    NonUnitaryFov,
}

#[derive(Error, Debug)]
pub enum InterpreterWarning {
    #[error(
        "Block #{block_id}: RF shim has {got} channel(s), but earlier RFs in \
         the sequence used {expected}"
    )]
    InconsistentShimChannelCount {
        block_id: u32,
        expected: usize,
        got: usize,
    },
    #[error(
        "Block #{block_id}: soft delay computed to {computed}s, which is \
         shorter than the block's existing duration {block}s — ignored"
    )]
    SoftDelayShortensBlock {
        block_id: u32,
        computed: f64,
        block: f64,
    },
    #[error("Block #{block_id}: multiple soft-delay extensions on a single block")]
    MultipleSoftDelays { block_id: u32 },
}

#[derive(Error, Debug)]
pub enum ShapeDecompressionError {
    #[error("RLE count {value} is not integer at index {index}")]
    RleCountIsNotInteger { index: usize, value: f64 },
    #[error("Shape decompressed into {count} samples, expected {expected}")]
    WrongDecompressedCount { count: usize, expected: usize },
}

#[derive(Debug)]
pub enum EventType {
    Rf,
    Gx,
    Gy,
    Gz,
    Adc,
}

impl Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventType::Rf => "RF",
            EventType::Gx => "GX",
            EventType::Gy => "GY",
            EventType::Gz => "GZ",
            EventType::Adc => "ADC",
        }
        .fmt(f)
    }
}

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("{ty} is too long for the containing block #{block_id}: {dur}s > {block_dur}s")]
    EventTooLong {
        ty: EventType,
        block_id: u32,
        dur: f64,
        block_dur: f64,
    },
    #[error(
        "{ty} in block #{block_id} uses shapes with different sample counts: {length_1} vs {length_2}"
    )]
    ShapeMismatch {
        ty: EventType,
        block_id: u32,
        length_1: usize,
        length_2: usize,
    },
    #[error("{ty} in block #{block_id} contains a negative timing: {timing}")]
    NegativeTiming {
        ty: EventType,
        block_id: u32,
        timing: f64,
    },
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Syntax error in pulseq file: {0}")]
    SyntaxError(String),
    #[error("Failed to parse float: {0}")]
    ParseFloat(#[from] std::num::ParseFloatError),
    #[error("Unsupported pulseq file version: {0}")]
    UnsupportedVersion(Version),
    #[error("Failed to decompress shape: {0}")]
    ShapeDecompressionError(#[from] ShapeDecompressionError),
}

impl<'s> From<winnow::error::ParseError<&'s str, winnow::error::ContextError>> for ParseError {
    fn from(e: winnow::error::ParseError<&'s str, winnow::error::ContextError>) -> Self {
        ParseError::SyntaxError(e.to_string())
    }
}

impl From<winnow::error::ErrMode<winnow::error::ContextError>> for ParseError {
    fn from(e: winnow::error::ErrMode<winnow::error::ContextError>) -> Self {
        ParseError::SyntaxError(e.to_string())
    }
}

#[derive(Debug)]
pub enum SectionType {
    Version,
    Signature,
    Definitions,
    Blocks,
    Rfs,
    Gradients,
    Traps,
    Adcs,
    ExtensionRefs,
    ExtensionSpecs,
    Shapes,
}

impl Display for SectionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SectionType::Version => "Version",
            SectionType::Signature => "Signature",
            SectionType::Definitions => "Definitions",
            SectionType::Blocks => "Blocks",
            SectionType::Rfs => "Rfs",
            SectionType::Gradients => "Gradients",
            SectionType::Traps => "Traps",
            SectionType::Adcs => "Adcs",
            SectionType::ExtensionRefs => "ExtensionRefs",
            SectionType::ExtensionSpecs => "ExtensionSpecs",
            SectionType::Shapes => "Shapes",
        }
        .fmt(f)
    }
}

#[derive(Error, Debug)]
pub enum ParseFovError {
    #[error(transparent)]
    ParseFloat(#[from] std::num::ParseFloatError),
    #[error("Expected 3 values, got {0}")]
    WrongValueCount(usize),
}

#[derive(Error, Debug)]
pub enum MissingDefinition {
    #[error("Pulseq since 1.4 mandates time raster definitions, but is GradientRasterTime missing")]
    GradientRasterTime,
    #[error(
        "Pulseq since 1.4 mandates time raster definitions, but is RadiofrequencyRasterTime missing"
    )]
    RadiofrequencyRasterTime,
    #[error("Pulseq since 1.4 mandates time raster definitions, but is AdcRasterTime missing")]
    AdcRasterTime,
    #[error(
        "Pulseq since 1.4 mandates time raster definitions, but is BlockDurationRaster missing"
    )]
    BlockDurationRaster,
}

#[derive(Error, Debug)]
pub enum ExtensionError {
    #[error("Extension '{ext}' expected {expected} field(s), got {got}")]
    WrongFieldCount {
        ext: &'static str,
        expected: usize,
        got: usize,
    },
    #[error("Extension '{ext}': failed to parse integer field: {source}")]
    ParseInt {
        ext: &'static str,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("Extension '{ext}': failed to parse float field: {source}")]
    ParseFloat {
        ext: &'static str,
        #[source]
        source: std::num::ParseFloatError,
    },
    #[error("Unknown label flag: '{0}'")]
    UnknownLabel(String),
    #[error("LABELINC requires a counter flag, got '{0}'")]
    LabelIncNotCounter(String),
    #[error(
        "Shim extension declares {declared} channel(s), but data contains {got} float(s) (expected {expected})"
    )]
    ShimCountMismatch {
        declared: u32,
        got: usize,
        expected: usize,
    },
}

#[derive(Error, Debug)]
pub enum ConversionError {
    #[error("Expected a single [VERSION] section, found {0}")]
    VersionSectionCount(usize),
    #[error("{0} Section contains non-unique IDs")]
    EventIdReuse(SectionType),
    #[error("Found re-used IDs between Trap and Gradient events")]
    GradTrapIdReuse,
    #[error("Definitions contain non-unique keys")]
    NonUniqueDefinition,
    #[error("Referenced {ty} with id {id} does not exist")]
    BrokenRef { ty: EventType, id: u32 },
    #[error("Referenced extension with id {id} does not exist")]
    InvalidExtensionRef { id: u32 },
    #[error("Extension ref chain starting at id {start_id} contains a cycle")]
    ExtensionRefCycle { start_id: u32 },
    #[error(transparent)]
    MissingDefinition(#[from] MissingDefinition),
    #[error("Failed to parse FOV: {0}")]
    ParseFovError(#[from] ParseFovError),
    #[error(transparent)]
    ParseFloat(#[from] std::num::ParseFloatError),
    #[error("Failed to parse extension data: {0}")]
    ExtensionParseError(#[from] ExtensionError),
    #[error("Shape with index {0} does not exist")]
    ShapeNotFound(u32),
    #[error("Can't use 0 as shape index")]
    ShapeIndexZero,
    #[error("Encountered a shape with no samples")]
    EmptyShape,
    #[error("time_id = -1 requires an odd sample count (M = 2N-1), got {0}")]
    HalfTickShapeEvenSampleCount(usize),
    #[error("Unknown time_id sentinel {0} (only 0 and -1 are recognised; positive = shape id)")]
    UnknownTimeId(i32),
    #[error("Used a shape of length {shape_len} together with a time shape of length {time_len}")]
    TimeShapeMismatch { shape_len: usize, time_len: usize },
    #[error("Used a shape as time shape which contained negative values.")]
    TimeShapeNegative,
    #[error("Used a shape as time shape which contained non-integer values.")]
    TimeShapeNonInteger,
    #[error("Used a shape as time shape which is not strictly increasing")]
    TimeShapeNonIncreasing,
    #[error("Unsupported extension: '{0}'")]
    UnsupportedExtension(String),
    #[error(transparent)]
    SoftDelayHintConflict(#[from] SoftDelayHintConflict),
}

#[derive(Error)]
pub enum Error {
    #[error(transparent)]
    ParseError(#[from] ParseError),
    #[error("Sequence validation failed: {0}")]
    ValidationError(#[from] ValidationError),
    #[error("Failed to convert parsed file into sequence: {0}")]
    ConversionError(#[from] ConversionError),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}
