//! Staggered text maps of terrain characters, and the palette naming them.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashMap},
    fmt::{self, Display},
    fs, iter,
    path::Path,
};

use crate::{
    defs::Lines,
    hex::{Axial, Top},
    warn,
};

/// Characters meaning "no hex here".
pub const BLANKS: [char; 2] = ['.', ' '];

#[derive(Debug)]
pub struct Terrain {
    pub key: char,
    pub color: String,
    pub name: Option<String>,
}

impl Terrain {
    /// The terrain's name, or its map character when the palette gives none.
    pub fn name(&self) -> Cow<'_, str> {
        match &self.name {
            Some(name) => name.into(),
            None => self.key.to_string().into(),
        }
    }
}

pub type Palette = HashMap<char, Terrain>;

/// A hex position as written in the map text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Coord {
    pub col: i32,
    pub row: i32,
}

impl Display for Coord {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{},{}", self.col, self.row)
    }
}

#[derive(Debug)]
pub struct Hex<'a> {
    pub coord: Coord,
    pub axial: Axial,
    pub terrain: &'a Terrain,
}

#[derive(Debug)]
pub struct Map<'a> {
    pub hexes: Vec<Hex<'a>>,
    /// Whether the image's first row (pointy-top) or column (flat-top) is the one shifted by half a hex.
    pub starts_offset: bool,
}

pub fn read_palette(path: &Path) -> Result<Palette, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("could not read palette {}: {err}", path.display()))?;
    parse_palette(text.as_str(), path.display())
}

pub fn read_map<'a>(path: &Path, palette: &'a Palette, top: Top) -> Result<Map<'a>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("could not read map {}: {err}", path.display()))?;
    parse_map(&text, path.display(), palette, top)
}

/// Parses '<char> <color> [name]' lines.
pub fn parse_palette<'a>(lines: impl Into<Lines<'a>>, file: impl Display) -> Result<Palette, String> {
    let lines = lines.into();
    let mut palette = Palette::new();
    for (line, n) in lines.iter() {
        let mut fields = line.split_whitespace();
        let (Some(key), Some(color)) = (fields.next(), fields.next()) else {
            return Err(format!("{file}:{n}: expected '<char> <color> [name]'"));
        };
        let mut chars = key.chars();
        let (Some(key), None) = (chars.next(), chars.next()) else {
            return Err(format!("{file}:{n}: terrain key must be one character, got \"{key}\""));
        };
        let name = fields.collect::<Vec<_>>().join(" ");
        let terrain = Terrain { key, color: color.to_string(), name: (!name.is_empty()).then_some(name) };
        if palette.insert(key, terrain).is_some() {
            return Err(format!("{file}:{n}: duplicate terrain key '{key}'"));
        }
    }
    if palette.is_empty() {
        return Err(format!("{file}: no terrain entries"));
    }
    Ok(palette)
}

/// A non-blank character in a staggered grid, and the hex it lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub coord: Coord,
    pub axial: Axial,
    pub ch: char,
    pub line: usize,
}

/// Parses a staggered grid of characters into the hexes they land on.
///
/// A character's position in its line and the line's index are its hex's doubled coordinates: pointy-top hexes step 2
/// across a row, and flat-top hexes step 2 down a column, split over two lines.
///
/// Returns the cells and whether the image's first row (pointy-top) or column (flat-top) is the one shifted by half a
/// hex. Every staggered file the tool reads comes through here, so the layouts cannot drift apart.
pub fn parse_grid(text: &str, file: impl Display, top: Top) -> Result<(Vec<Cell>, bool), String> {
    let mut cells = Vec::new();
    // Pointy-top rows alternate between indented and not; this records which parity of row is indented.
    let mut stagger = None;

    for (y, line) in (0..).zip(text.lines()) {
        let n = y + 1;
        let indent = line.chars().take_while(|&c| c == ' ').count() as i32 % 2;
        let mut off_grid = false;

        for (x, ch) in (0..).zip(line.chars()) {
            if BLANKS.contains(&ch) {
                continue;
            }
            let (coord, axial) = match top {
                Top::Pointy if x % 2 == indent => {
                    let parity = (x - y).rem_euclid(2);
                    if *stagger.get_or_insert(parity) != parity {
                        return Err(format!(
                            "{file}:{n}: rows must alternate between indented and not, and this one breaks the pattern"
                        ));
                    }
                    (Coord { col: x / 2, row: y }, Axial::new((x - y - parity) / 2, y))
                }
                Top::Flat if x % 4 == 2 * (y % 2) => {
                    let col = x / 2;
                    (Coord { col, row: y / 2 }, Axial::new(col, (y - col) / 2))
                }
                _ => {
                    off_grid = true;
                    continue;
                }
            };
            cells.push(Cell { coord, axial, ch, line: n as usize });
        }

        if off_grid {
            let layout = match top {
                Top::Pointy => "hexes sit 2 characters apart, with alternate rows indented by 1",
                Top::Flat => "hexes sit 4 characters apart, with odd lines indented by 2",
            };
            warn(format_args!("{file}:{n}: ignored characters between hexes; {layout}"));
        }
    }

    let starts_offset = match top {
        Top::Pointy => {
            let first_row = cells.iter().map(|cell| cell.coord.row).min();
            first_row.zip(stagger).is_some_and(|(row, parity)| (row + parity) % 2 == 1)
        }
        Top::Flat => cells.iter().map(|cell| cell.coord.col).min().is_some_and(|col| col % 2 == 1),
    };
    Ok((cells, starts_offset))
}

/// Parses a staggered map of terrain characters, naming each one from the palette.
pub fn parse_map<'a>(text: &str, file: impl Display, palette: &'a Palette, top: Top) -> Result<Map<'a>, String> {
    let (cells, starts_offset) = parse_grid(text, &file, top)?;
    let mut hexes = Vec::new();
    let mut unknown: BTreeMap<char, Vec<String>> = BTreeMap::new();

    for cell in cells {
        match palette.get(&cell.ch) {
            Some(terrain) => hexes.push(Hex { coord: cell.coord, axial: cell.axial, terrain }),
            None => unknown.entry(cell.ch).or_default().push(format!("{file}:{} col {}", cell.line, cell.coord.col)),
        }
    }

    if !unknown.is_empty() {
        let lines: Vec<String> = unknown
            .iter()
            .map(|(ch, spots)| {
                let shown = spots.iter().take(3).map(String::as_str).collect::<Vec<_>>().join(", ");
                match spots.len() {
                    count if count > 3 => format!("  '{ch}' at {shown} and {} more", count - 3),
                    _ => format!("  '{ch}' at {shown}"),
                }
            })
            .collect();
        return Err(format!("unknown terrain characters (add them to the palette):\n{}", lines.join("\n")));
    }

    if hexes.is_empty() {
        return Err(format!("{file}: map has no hexes"));
    }
    Ok(Map { hexes, starts_offset })
}

/// Parses a staggered grid laid out like `map`, and places every cell on the hex the map put there.
///
/// Rivers, roads and borders are each typed as their own grid beside the map, and all of them have to land on the map's
/// own hexes: a grid that is out of step fails here rather than drawing the whole network half a hex out.
pub fn place_grid(text: &str, file: impl Display, map: &Map, top: Top) -> Result<Vec<(Axial, Cell)>, String> {
    let (cells, _) = parse_grid(text, &file, top)?;
    let axial_of: HashMap<Coord, Axial> = map.hexes.iter().map(|hex| (hex.coord, hex.axial)).collect();

    cells
        .into_iter()
        .map(|cell| {
            let axial = *axial_of
                .get(&cell.coord)
                .ok_or_else(|| format!("{file}:{}: no hex at {} on the map", cell.line, cell.coord))?;
            // Both grids work out an axial for the coordinate they wrote, and they only agree when the
            // rows are staggered the same way. A grid indented by an odd number of spaces still lands
            // every hex on the map, just not the hexes it looks like it names, so this is what catches
            // it; the map's own axial is the one used from here on.
            if axial != cell.axial {
                return Err(format!(
                    "{file}:{}: the grid is staggered the other way from the map, putting {} half a hex out; check the \
                     leading spaces",
                    cell.line, cell.coord
                ));
            }
            Ok((axial, cell))
        })
        .collect()
}

/// Writes `cells` as a staggered map that [`parse_map`] reads back, each hex drawn with its character.
///
/// The text starts at its first line and column, so the cells are moved to get there. Returns the text and that move:
/// the cell at `axial` is read back at `axial + shift`.
pub fn write_map(cells: &[(Axial, char)], top: Top) -> (String, Axial) {
    // Column within the line, and line number: the inverse of the doubled coordinates `parse_map` reads.
    let doubled = |axial: Axial| match top {
        Top::Pointy => (2 * axial.q + axial.r, axial.r),
        Top::Flat => (2 * axial.q, 2 * axial.r + axial.q),
    };
    let lowest = |pick: fn((i32, i32)) -> i32| cells.iter().map(|&(axial, _)| pick(doubled(axial))).min().unwrap_or(0);
    let (min_x, min_y) = (lowest(|(x, _)| x), lowest(|(_, y)| y));

    // Moving by one step changes the doubled coordinates by one or two, so the text may start one column or line in.
    let shift = match top {
        Top::Pointy => {
            let r = -min_y;
            Axial::new((1 - (min_x + r)).div_euclid(2), r)
        }
        Top::Flat => {
            let q = -min_x / 2;
            Axial::new(q, (1 - (min_y + q)).div_euclid(2))
        }
    };

    let mut lines: BTreeMap<i32, BTreeMap<i32, char>> = BTreeMap::new();
    for &(axial, ch) in cells {
        let (x, y) = doubled(axial + shift);
        lines.entry(y).or_default().insert(x, ch);
    }

    let mut text = String::new();
    for y in 0..=lines.keys().next_back().copied().unwrap_or(-1) {
        let mut column = 0;
        for (&x, &ch) in lines.get(&y).into_iter().flatten() {
            text.extend(iter::repeat_n(' ', (x - column) as usize));
            text.push(ch);
            column = x + 1;
        }
        text.push('\n');
    }
    (text, shift)
}

#[cfg(test)]
mod tests {
    use super::{Axial, Palette, Top, parse_map, parse_palette, write_map};

    fn palette() -> Palette {
        parse_palette("# comment\n~ #78c7f5 shallow  water\ng #95c763\n", "p").unwrap()
    }

    fn cells(text: &str, top: Top) -> Vec<(i32, i32, i32, i32, char)> {
        let palette = palette();
        let map = parse_map(text, "m", &palette, top).unwrap();
        map.hexes.iter().map(|h| (h.coord.col, h.coord.row, h.axial.q, h.axial.r, h.terrain.key)).collect()
    }

    #[test]
    fn palette_names_are_optional() {
        let palette = palette();
        assert_eq!(palette[&'~'].name(), "shallow water");
        assert_eq!(palette[&'g'].name(), "g");
        assert!(parse_palette("g #fff\ng #000", "p").unwrap_err().contains("duplicate"));
        assert!(parse_palette("gg #fff", "p").unwrap_err().contains("one character"));
        assert!(parse_palette("g", "p").is_err());
    }

    #[test]
    fn pointy_rows_stagger_and_ragged_edges_are_blank() {
        assert_eq!(
            cells("~ g\n g ~\n    ~\n", Top::Pointy),
            [(0, 0, 0, 0, '~'), (1, 0, 1, 0, 'g'), (0, 1, 0, 1, 'g'), (1, 1, 1, 1, '~'), (2, 2, 1, 2, '~')]
        );
    }

    #[test]
    fn pointy_rows_must_alternate() {
        let palette = palette();
        assert!(parse_map("~ g\n~ g\n", "m", &palette, Top::Pointy).unwrap_err().contains("m:2: rows must alternate"));
        let map = parse_map(" ~ g\n~ g\n", "m", &palette, Top::Pointy).unwrap();
        assert!(map.starts_offset);
    }

    #[test]
    fn flat_lines_interleave_columns() {
        assert_eq!(
            cells("~   g\n  g\n.   ~\n", Top::Flat),
            [(0, 0, 0, 0, '~'), (2, 0, 2, -1, 'g'), (1, 0, 1, 0, 'g'), (2, 1, 2, 0, '~')]
        );
    }

    #[test]
    fn unknown_characters_are_all_reported() {
        let palette = palette();
        let err = parse_map("x x x x\n q\n", "m", &palette, Top::Pointy).unwrap_err();
        assert!(err.contains("'q' at m:2 col 0"), "{err}");
        assert!(err.contains("'x' at m:1 col 0, m:1 col 1, m:1 col 2 and 1 more"), "{err}");
    }

    #[test]
    fn written_maps_read_back() {
        let palette = palette();
        let cells = [
            (Axial::new(-3, 2), '~'),
            (Axial::new(-2, 2), 'g'),
            (Axial::new(-3, 3), 'g'),
            (Axial::new(1, -1), '~'),
            (Axial::new(4, 5), '~'),
        ];
        for top in [Top::Pointy, Top::Flat] {
            let (text, shift) = write_map(&cells, top);
            let map = parse_map(&text, "m", &palette, top).unwrap_or_else(|err| panic!("{top}: {err}\n{text}"));
            let mut read: Vec<(Axial, char)> = map.hexes.iter().map(|hex| (hex.axial, hex.terrain.key)).collect();
            let mut expected: Vec<(Axial, char)> = cells.iter().map(|&(axial, ch)| (axial + shift, ch)).collect();
            read.sort();
            expected.sort();
            assert_eq!(read, expected, "{top}\n{text}");
        }
    }
}
