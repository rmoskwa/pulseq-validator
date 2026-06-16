/// The `raw` pulseq data as parsed directly from a .seq file. Contains blocks,
/// events, shapes and extensions, all indexed by their IDs. The structures
/// are modeled to support all pulseq versions, missing data added in later
/// versions of pulseq will be filled with default values for earlier files.
pub mod raw;

/// The `model` module converts the raw data into a more ideomatic model where
/// IDs are replaced with pointers and known definitions and extensions are
/// parsed. The loose collection of `raw` section is converted into a stricter
/// structure. Some newer pulseq requirements are enforced in this step.
/// (Formerly `seq`; renamed to avoid colliding with `.seq` / winnow's `seq!`.)
pub mod model;

/// The `interp`reted sequence transforms the parsed data into what the scanner
/// would actually execute. (Formerly `int`; renamed to avoid colliding with
/// the integer-parsing combinator also named `int`.) It does the following:
/// 1. apply FOV scaling (respecting no_rot / no_scale labels)
/// 2. compute labels for each ADC
/// 3. compute soft delays (takes arbitrary input vars!)
/// 4. unify rel. and offset freq / phase using larmor frequency
/// 5. apply the rotation extension
/// 6. store `Once` and `Pmc` flags as well as triggers in the blocks
/// 7. unifying shims stored directly in the RF
pub mod interp;

mod error;
pub use error::Error;
