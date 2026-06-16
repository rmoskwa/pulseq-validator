use winnow::ascii::{line_ending, till_line_ending};
use winnow::combinator::{alt, eof, opt, preceded, repeat};
use winnow::prelude::*;
use winnow::token::take_while;

use crate::error::ShapeDecompressionError;

pub fn decompress_shape(
    samples: Vec<f64>,
    num_samples: u32,
) -> Result<Vec<f64>, ShapeDecompressionError> {
    // First, decompress into the deriviate of the shape
    let mut deriv = Vec::with_capacity(num_samples as usize);

    // The two samples before the current one, to detect RLE
    let mut a = f64::NAN;
    let mut b = f64::NAN;
    // After a detected RLE, skip the RLE check for two samples
    let mut skip: u32 = 0;

    for (index, sample) in samples.into_iter().enumerate() {
        if a == b && skip == 0 {
            if sample != sample.round() {
                Err(ShapeDecompressionError::RleCountIsNotInteger {
                    index,
                    value: sample,
                })?;
            }

            skip = 2;
            for _ in 0..sample as usize {
                deriv.push(b);
            }
        } else {
            skip = skip.saturating_sub(1);
            deriv.push(sample);
        }

        a = b;
        b = sample;
    }

    if deriv.len() != num_samples as usize {
        Err(ShapeDecompressionError::WrongDecompressedCount {
            count: deriv.len(),
            expected: num_samples as usize,
        })?;
    }

    // Then, do a cumultative sum to get the shape
    Ok(deriv
        .into_iter()
        .scan(0.0, |acc, x| {
            *acc += x;
            Some(*acc)
        })
        .collect())
}

// Simple parsers that are not really specific to pulseq

/// Matches at least one whitespace but now newline
pub fn ws(input: &mut &str) -> ModalResult<()> {
    take_while(1.., (' ', '\t')).void().parse_next(input)
}

/// Matches as many whitespaces and comments as possible but expects at least one '\n'
pub fn nl(input: &mut &str) -> ModalResult<()> {
    // matches comments or empty lines, stops at line ending
    let comment = || alt((ws, ('#', till_line_ending).void()));
    // consume the line ending here not in comment to support ending in comment
    alt((
        eof.void(),
        (
            repeat::<_, _, (), _, _>(1.., (opt(comment()), line_ending)),
            opt(comment()),
        )
            .void(),
    ))
    .parse_next(input)
}

/// Shorthand for tag + whitespace
pub fn tag_ws(tag_str: &'static str) -> impl FnMut(&mut &str) -> ModalResult<()> {
    move |input: &mut &str| (tag_str, ws).void().parse_next(input)
}

/// Shorthand for tag + newline
pub fn tag_nl(tag_str: &'static str) -> impl FnMut(&mut &str) -> ModalResult<()> {
    move |input: &mut &str| (tag_str, nl).void().parse_next(input)
}

pub fn ident(input: &mut &str) -> ModalResult<String> {
    take_while(1.., |c: char| c.is_ascii_graphic())
        .map(str::to_owned)
        .parse_next(input)
}

/// (opt(ws), int)
pub fn int(input: &mut &str) -> ModalResult<u32> {
    preceded(opt(ws), winnow::ascii::dec_uint).parse_next(input)
}

/// (opt(ws), signed int) - used for `time_id` fields where pulseq 1.5 introduced
/// the `-1` sentinel (samples at every half-tick).
pub fn signed_int(input: &mut &str) -> ModalResult<i32> {
    preceded(opt(ws), winnow::ascii::dec_int).parse_next(input)
}

/// (opt(ws), float)
pub fn float(input: &mut &str) -> ModalResult<f64> {
    preceded(opt(ws), winnow::ascii::float).parse_next(input)
}
