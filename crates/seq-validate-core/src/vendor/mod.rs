//! Vendor-specific conformance checks.
//!
//! The hardware checks are vendor-*parametric*: the same limit comparisons run
//! against whichever numbers a [`Profile`](crate::profile::Profile) supplies.
//! These checks are different — each encodes a vendor's structural *gimmick*: a
//! rule that vendor's interpreter imposes which has no analogue elsewhere. So
//! they cannot be expressed as a profile number; they are code.
//!
//! The modular seam is [`Check::vendor_scope`](crate::checks::Check::vendor_scope):
//! a vendor check declares the vendors it applies to, and
//! [`run_all`](crate::checks::run_all) only invokes it when the active profile's
//! vendor matches. Adding a vendor's rule is local: drop a `vendor/<name>.rs`,
//! return its checks here, and the runner + catalog pick it up.

use crate::checks::Check;

mod ge;

/// The vendor-specific checks, wired into [`crate::checks::registry`].
pub(crate) fn checks() -> Vec<Box<dyn Check>> {
    ge::checks()
}
