use crate::{error::SectionType, raw};

/// This will extract all sections of the correct type from the list of raw
/// sections and return the merged data (if file contained a section more than once)
pub fn get_section_data<T: SectionData>(sections: &mut Vec<raw::Section>) -> Vec<T> {
    sections
        .extract_if(.., |sec| T::can_extract(sec))
        .flat_map(T::extract)
        .collect()
}

/// Helper trait for a unified section interface, used by `get_section_data`
pub trait SectionData {
    /// Used by error messages to specify in which section the problem lies.
    const SECTION_TYPE: SectionType;

    /// checks if implementing section can be extracted from the dynamic `sec`
    fn can_extract(sec: &raw::Section) -> bool;

    /// panics if sec has the wrong type - check with can_extract first!
    fn extract(sec: raw::Section) -> Vec<Self>
    where
        Self: Sized;
}

/// Generates a `SectionData` impl for a variant carrying `Vec<T>`.
/// `$variant` must name a variant that exists in both `raw::Section` and
/// `error::SectionType`.
macro_rules! impl_section_data_vec {
    ($ty:ty, $variant:ident) => {
        impl SectionData for $ty {
            const SECTION_TYPE: SectionType = SectionType::$variant;

            fn can_extract(sec: &raw::Section) -> bool {
                matches!(sec, raw::Section::$variant(_))
            }

            fn extract(sec: raw::Section) -> Vec<Self> {
                match sec {
                    raw::Section::$variant(items) => items,
                    _ => unreachable!(),
                }
            }
        }
    };
}

/// Generates a `SectionData` impl for a variant carrying a single `T`,
/// wrapping it in a 1-element vec. `$variant` must name a variant that exists
/// in both `raw::Section` and `error::SectionType`.
macro_rules! impl_section_data_single {
    ($ty:ty, $variant:ident) => {
        impl SectionData for $ty {
            const SECTION_TYPE: SectionType = SectionType::$variant;

            fn can_extract(sec: &raw::Section) -> bool {
                matches!(sec, raw::Section::$variant(_))
            }

            fn extract(sec: raw::Section) -> Vec<Self> {
                match sec {
                    raw::Section::$variant(item) => vec![item],
                    _ => unreachable!(),
                }
            }
        }
    };
}

impl_section_data_single!(raw::Version, Version);
impl_section_data_single!(raw::Signature, Signature);
impl_section_data_vec!((String, String), Definitions);
impl_section_data_vec!(raw::Block, Blocks);
impl_section_data_vec!(raw::Rf, Rfs);
impl_section_data_vec!(raw::Gradient, Gradients);
impl_section_data_vec!(raw::Trap, Traps);
impl_section_data_vec!(raw::Adc, Adcs);
impl_section_data_vec!(raw::ExtensionRef, ExtensionRefs);
impl_section_data_vec!(raw::ExtensionSpec, ExtensionSpecs);
impl_section_data_vec!(raw::Shape, Shapes);
