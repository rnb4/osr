//! Reading the features file.

use std::{collections::HashMap, fmt::Display, fs, path::Path};

use crate::{
    hex::Axial,
    defs::content_lines,
    map::{Coord, Map},
};

#[derive(Debug)]
pub struct FeatureSpec {
    pub coord: Coord,
    pub axial: Axial,
    pub kind: String,
    pub label: Option<String>,
    pub line: usize,
}

pub fn read_features(path: &Path, map: &Map) -> Result<Vec<FeatureSpec>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("could not read features {}: {err}", path.display()))?;
    parse_features(&text, path.display(), map)
}

/// Parses '<col> <row> <type> [label]' lines, each naming a hex on `map`.
fn parse_features(text: &str, file: impl Display, map: &Map) -> Result<Vec<FeatureSpec>, String> {
    let axial_of: HashMap<Coord, Axial> = map.hexes.iter().map(|hex| (hex.coord, hex.axial)).collect();
    content_lines(text)
        .map(|(line, n)| {
            let mut fields = line.split_whitespace();
            let (Some(col), Some(row), Some(kind)) = (fields.next(), fields.next(), fields.next()) else {
                return Err(format!("{file}:{n}: expected '<col> <row> <type> [label]'"));
            };
            let (Ok(col), Ok(row)) = (col.parse(), row.parse()) else {
                return Err(format!("{file}:{n}: col and row must be whole numbers"));
            };
            let coord = Coord { col, row };
            let axial = *axial_of.get(&coord).ok_or_else(|| format!("{file}:{n}: no hex at {coord}"))?;
            let label = fields.collect::<Vec<_>>().join(" ");
            Ok(FeatureSpec {
                coord,
                axial,
                kind: kind.to_string(),
                label: (!label.is_empty()).then_some(label),
                line: n,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_features;
    use crate::{
        hex::{Axial, Top},
        map::{Coord, parse_map, parse_palette},
    };

    #[test]
    fn features_name_hexes_on_the_map() {
        let palette = parse_palette("~ #78c7f5 water\ng #95c763\n", "p").unwrap();
        let map = parse_map("~ g\n g ~\n", "m", &palette, Top::Pointy).unwrap();
        let specs = parse_features("# col row type label\n1 1 tower  Kelvin  Watch\n0 0 city\n", "f", &map).unwrap();
        assert_eq!((specs[0].coord, specs[0].axial), (Coord { col: 1, row: 1 }, Axial::new(1, 1)));
        assert_eq!(specs[0].label.as_deref(), Some("Kelvin Watch"));
        assert_eq!((specs[1].line, specs[1].label.as_deref()), (3, None));
        assert!(parse_features("5 5 city", "f", &map).unwrap_err().contains("f:1: no hex at 5,5"));
    }
}
