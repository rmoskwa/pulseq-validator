use std::collections::HashMap;

use crate::{
    error::{ConversionError, MissingDefinition, ParseFovError},
    model,
};

/// Simple helper struct to parse definitions into - might be removed after some
/// more refactoring, but as it's contained in this file this is not urgent.
pub struct Defs {
    pub name: Option<String>,
    pub fov: Option<(f64, f64, f64)>,
    pub time_raster: model::TimeRaster,
    /// lower-cased strings from "RequiredExtensions" definition
    pub required_exts: Vec<String>,
    pub defs: HashMap<String, String>,
}

impl Defs {
    pub fn from_raw(defs: Vec<(String, String)>) -> Result<Self, ConversionError> {
        let def_count = defs.len();
        let mut defs: HashMap<_, _> = defs.into_iter().collect();
        if defs.len() < def_count {
            // Duplicated key
            return Err(ConversionError::NonUniqueDefinition);
        }

        let required_exts: Vec<String> = defs
            .remove("RequiredExtensions")
            .unwrap_or(String::new())
            .split_whitespace()
            .map(|s| s.trim().to_lowercase())
            .collect();

        // Pulseq 1.4+ mandates the time-raster definitions.
        let time_raster = model::TimeRaster {
            grad: defs
                .remove("GradientRasterTime")
                .ok_or(MissingDefinition::GradientRasterTime)?
                .parse()?,
            rf: defs
                .remove("RadiofrequencyRasterTime")
                .ok_or(MissingDefinition::RadiofrequencyRasterTime)?
                .parse()?,
            adc: defs
                .remove("AdcRasterTime")
                .ok_or(MissingDefinition::AdcRasterTime)?
                .parse()?,
            block: defs
                .remove("BlockDurationRaster")
                .ok_or(MissingDefinition::BlockDurationRaster)?
                .parse()?,
        };
        let name = defs.remove("Name");
        let fov = defs.remove("FOV").map(parse_fov).transpose()?;

        Ok(Defs {
            name,
            fov,
            time_raster,
            required_exts,
            defs,
        })
    }
}

fn parse_fov(s: String) -> Result<(f64, f64, f64), ParseFovError> {
    let splits: Vec<&str> = s.split_whitespace().collect();
    let splits: [&str; 3] = splits
        .try_into()
        .map_err(|vals: Vec<&str>| ParseFovError::WrongValueCount(vals.len()))?;

    Ok((splits[0].parse()?, splits[1].parse()?, splits[2].parse()?))
}
