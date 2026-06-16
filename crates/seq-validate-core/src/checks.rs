//! Validation checks over the interpreted [`Sequence`](crate::Sequence).
//!
//! This is the validator's actual value-add and the home for every downstream
//! check. It is intentionally empty at the end of Step 1 (the vendor-parser
//! step only establishes the IR these checks consume); later steps populate it.
//!
//! The split is deliberate: [`crate::ir`] is a thin file facade over the
//! `pulseq-parse` interpreted layer, while *this* module is where the project's
//! own validation logic accrues.
