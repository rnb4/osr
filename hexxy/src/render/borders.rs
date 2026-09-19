//! Reading the border types and the borders grid, and tracing the areas they mark out.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    fs,
    path::Path,
};

use crate::{
    defs::Lines,
    hex::{Axial, Layout, Point, Top},
    map::{self, Map},
};

/// One kind of border, named by the character standing for it on the grid.
pub type Type = super::stroke::Kind;

/// One closed ring of border, as the points a stroke runs through.
#[derive(Debug)]
pub struct Border {
    /// The type it is drawn in, as an index into the types read alongside it.
    pub kind: usize,
    /// The corners the ring passes through; the path closes from the last back to the first.
    pub ring: Vec<Point>,
}

/// A corner named the same way by all three hexes holding it; see [`Layout::vertex`].
type Vertex = (Axial, usize);

pub fn read_types(path: &Path) -> Result<Vec<Type>, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("could not read border types {}: {err}", path.display()))?;
    parse_types(text.as_str(), path.display())
}

/// Parses '<char> <color> <width> <dash> [name]' lines.
///
/// The types are kept in a file rather than given as flags, the way the palette and the road classes are: a map with a
/// kingdom, three duchies and a disputed march on it wants the list written down once, not retyped on every run.
///
/// The types stay in the order they were written, because that is the order they are drawn in.
fn parse_types<'a>(lines: impl Into<Lines<'a>>, file: impl Display) -> Result<Vec<Type>, String> {
    let lines = lines.into();
    let mut types: Vec<Type> = Vec::new();
    for (line, n) in lines.iter() {
        let kind = super::stroke::parse_kind(line, "border type", "k #7a3b3b 0.08 none kingdom")
            .map_err(|err| format!("{file}:{n}: {err}"))?;
        if types.iter().any(|existing| existing.key == kind.key) {
            return Err(format!("{file}:{n}: duplicate border type '{}'", kind.key));
        }
        types.push(kind);
    }
    if types.is_empty() {
        return Err(format!("{file}: no border types"));
    }
    Ok(types)
}

pub fn read_borders(path: &Path, map: &Map, layout: &Layout, top: Top, types: &[Type]) -> Result<Vec<Border>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("could not read borders {}: {err}", path.display()))?;
    parse_borders(&text, path.display(), map, layout, top, types)
}

/// Parses a staggered grid of border hexes and traces the perimeter of every area it marks out.
fn parse_borders(
    text: &str,
    file: impl Display,
    map: &Map,
    layout: &Layout,
    top: Top,
    types: &[Type],
) -> Result<Vec<Border>, String> {
    let hexes = border_hexes(text, &file, map, top, types)?;

    let mut borders = Vec::new();
    for (kind, ty) in types.iter().enumerate() {
        let area: BTreeSet<Axial> = hexes.iter().filter(|&(_, &theirs)| theirs == kind).map(|(&hex, _)| hex).collect();
        // Half the stroke's own width, so its outer edge lands on the grid line. A border on its own still sits on the
        // line it belongs to, and two areas meeting along an edge draw a line each side of it rather than one over the
        // other.
        let inset = ty.width * layout.hex_width() / 2.0;
        borders.extend(rings(layout, &area).into_iter().map(|ring| Border { kind, ring: inset_ring(&ring, inset) }));
    }
    Ok(borders)
}

/// The type of every border hex.
fn border_hexes(
    text: &str,
    file: impl Display,
    map: &Map,
    top: Top,
    types: &[Type],
) -> Result<BTreeMap<Axial, usize>, String> {
    let mut hexes = BTreeMap::new();
    let mut unknown: BTreeMap<char, Vec<String>> = BTreeMap::new();
    for (axial, cell) in map::place_grid(text, &file, map, top)? {
        match types.iter().position(|ty| ty.key == cell.ch) {
            Some(kind) => {
                hexes.insert(axial, kind);
            }
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
        let known: Vec<String> = types.iter().map(|ty| format!("'{}' {}", ty.key, ty.name)).collect();
        return Err(format!(
            "unknown border characters (add them to the border types file; so far {}):\n{}",
            known.join(", "),
            lines.join("\n")
        ));
    }
    Ok(hexes)
}

/// The perimeter of `area`, as closed rings of hex corners.
///
/// An edge is on the perimeter when the hex across it is not in the area, so the edges inside the area drop out and
/// what is left surrounds it. Every edge is taken the way its own hex's corners run, which sends the whole perimeter
/// round one way, and that is what lets the edges be joined head to tail.
///
/// Only three hexes meet at a corner, and they are neighbours of one another, so three edges meet there and at most one
/// of them leaves the corner along the perimeter: whatever the area, the perimeter comes apart into separate closed
/// loops with nothing to choose between at any corner. An area with an enclave in it therefore gives a ring for its
/// outside and one more round the enclave, and two areas of one type can never meet at a corner alone, because two
/// hexes sharing a corner share an edge as well.
fn rings(layout: &Layout, area: &BTreeSet<Axial>) -> Vec<Vec<Point>> {
    let mut onwards: BTreeMap<Vertex, (Vertex, Point)> = BTreeMap::new();
    for &hex in area {
        let directions = layout.directions();
        let corners = layout.corners(layout.centre(hex), 1.0);
        for (i, &direction) in directions.iter().enumerate() {
            if !area.contains(&(hex + direction)) {
                onwards.insert(layout.vertex(hex, i), (layout.vertex(hex, i + 1), corners[i]));
            }
        }
    }

    let mut rings = Vec::new();
    for start in onwards.keys().copied().collect::<Vec<Vertex>>() {
        // Corners already walked were taken out of the map, so whatever is left starts a ring of its own.
        let Some((mut at, corner)) = onwards.remove(&start) else { continue };
        let mut ring = vec![corner];
        while at != start {
            let (next, corner) = onwards.remove(&at).expect("a perimeter is left as often as it is reached");
            ring.push(corner);
            at = next;
        }
        rings.push(ring);
    }
    rings
}

/// `ring` pulled `inset` px towards the area it surrounds.
///
/// The corners stay sharp: each one moves along the bisector of its two edges' inward normals, far enough that both
/// inset edges still pass through it. For unit normals `a` and `b` that point is `corner + (a + b) * inset / (1 + a·b)`,
/// which comes to `inset / sin 60°` at a corner turning out of the area and twice the inset at one turning into it.
fn inset_ring(ring: &[Point], inset: f64) -> Vec<Point> {
    // A ring runs the way its hexes' own corners do, and a quarter turn from that faces into the hex, so it faces into
    // the area whether this ring is the area's outside or the edge of an enclave in it.
    let inward = |from: usize, to: usize| (ring[to] - ring[from]).normalised().perpendicular();
    (0..ring.len())
        .map(|i| {
            let (before, after) = (inward((i + ring.len() - 1) % ring.len(), i), inward(i, (i + 1) % ring.len()));
            // Normals facing opposite ways leave nowhere to move to. Hex corners never turn that far back on
            // themselves, but it costs a line to say so rather than divide by zero over it.
            match 1.0 + before.x * after.x + before.y * after.y {
                turn if turn.abs() < 1e-9 => ring[i],
                turn => ring[i] + (before + after) * (inset / turn),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Border, Type, parse_borders, parse_types};
    use crate::{
        hex::{Axial, Layout, Top},
        map::{Map, Palette, parse_map, parse_palette},
    };

    const PALETTE: &str = "g #95c763 clear\n~ #78c7f5 water\n";

    // Four columns by three rows, the same fixture the rivers and roads use. The hex written at 1,1 has all six of its
    // neighbours on the map, so an area can be drawn right round it.
    const MAP: &str = "g g g g\n g g g g\ng g g ~\n";

    const TYPES: &str = "k #7a3b3b 0.08 none kingdom\nd #555555 0.04 0.06,0.06 duchy\n";

    fn palette() -> Palette {
        parse_palette(PALETTE, "p").unwrap()
    }

    fn layout(map: &Map) -> Layout {
        Layout::fit(Top::Pointy, 10.0, map.hexes.iter().map(|hex| hex.axial)).0
    }

    fn types() -> Vec<Type> {
        parse_types(TYPES, "b").unwrap()
    }

    fn read(map: &Map, layout: &Layout, borders: &str) -> Result<Vec<Border>, String> {
        parse_borders(borders, "d", map, layout, Top::Pointy, &types())
    }

    /// Ring lengths, smallest first: one point per perimeter edge, so these count the edges traced.
    fn lengths(borders: &[Border]) -> Vec<usize> {
        let mut lengths: Vec<usize> = borders.iter().map(|border| border.ring.len()).collect();
        lengths.sort_unstable();
        lengths
    }

    #[test]
    fn a_lone_hex_is_ringed_by_its_own_six_edges() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        let borders = read(&map, &layout, "k . . .\n . . . .\n. . . .\n").unwrap();
        assert_eq!(borders.len(), 1);
        assert_eq!(borders[0].kind, 0);
        assert_eq!(lengths(&borders), [6]);
    }

    #[test]
    fn neighbours_of_one_type_drop_the_edge_between_them() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        // Twelve edge slots less the shared one, which each hex leaves out.
        assert_eq!(lengths(&read(&map, &layout, "k k . .\n . . . .\n. . . .\n").unwrap()), [10]);
    }

    #[test]
    fn an_enclave_gives_a_ring_of_its_own() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        // The six hexes round 1,1, with 1,1 itself left out. The seven together have 30 edges, of which the ring's six
        // hexes hold 24: eighteen round the outside and six round the hole.
        let borders = read(&map, &layout, ". k k .\n k . k .\n. k k .\n").unwrap();
        assert_eq!(borders.len(), 2);
        assert!(borders.iter().all(|border| border.kind == 0));
        assert_eq!(lengths(&borders), [6, 18]);
    }

    #[test]
    fn separate_areas_of_one_type_each_get_a_ring() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        assert_eq!(lengths(&read(&map, &layout, "k . . k\n . . . .\n. . . .\n").unwrap()), [6, 6]);

        // A second type alongside is traced on its own, and the types keep the order the file gave them.
        let borders = read(&map, &layout, "k . . d\n . . . .\n. . . .\n").unwrap();
        assert_eq!(borders.iter().map(|border| border.kind).collect::<Vec<usize>>(), [0, 1]);
    }

    #[test]
    fn a_border_sits_just_inside_its_own_area() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        let borders = read(&map, &layout, "k . . .\n . . . .\n. . . .\n").unwrap();
        // Insetting a hexagon by half a stroke of 0.08 hex widths takes its corners in by that same fraction of the
        // radius, so a stroke drawn on the ring reaches back out to the grid line and no further.
        let centre = layout.centre(Axial::default());
        for corner in &borders[0].ring {
            let reach = (*corner - centre).length();
            assert!((reach - 0.92 * layout.radius()).abs() < 1e-9, "{reach}");
        }
    }

    #[test]
    fn characters_with_no_type_are_all_reported() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        let err = read(&map, &layout, "x x x x\n k . . .\n. . . .\n").unwrap_err();
        assert!(err.contains("'x' at d:1 col 0, d:1 col 1, d:1 col 2 and 1 more"), "{err}");
        assert!(err.contains("'k' kingdom, 'd' duchy"), "{err}");
    }

    #[test]
    fn border_hexes_must_be_on_the_map_and_in_step_with_it() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        let err = read(&map, &layout, ". . . .\n . . . .\nk k k k k\n").unwrap_err();
        assert!(err.contains("d:3: no hex at 4,2"), "{err}");

        let shifted: String = "k . . .\n k . . .\nk k k k\n".lines().map(|line| format!(" {line}\n")).collect();
        let err = read(&map, &layout, &shifted).unwrap_err();
        assert!(err.contains("staggered the other way"), "{err}");
    }

    #[test]
    fn type_files_are_checked() {
        let types = types();
        assert_eq!(types.iter().map(|ty| ty.key).collect::<Vec<char>>(), ['k', 'd']);
        assert_eq!(types[0].name, "kingdom");
        assert!(types[0].dash.is_empty());
        assert_eq!(types[1].dash, [0.06, 0.06]);

        let err = |text: &str| parse_types(text, "b").unwrap_err();
        assert!(err("k #000 0.05 none a\nk #fff 0.05 none b\n").contains("b:2: duplicate border type 'k'"));
        assert!(err("# only a comment\n").contains("b: no border types"));
        assert!(err(". #000 0.05 none").contains("b:1: '.' means \"no hex here\""));
        assert!(err("kk #000 0.05 none").contains("b:1: border type key must be one character"));
        assert!(err("k #000 0 none").contains("b:1: width must be a number more than zero"));
        assert!(err("k #000 0.05").contains("b:1: expected '<char> <color> <width> <dash> [name]'"));
        assert!(err("k #000 0.05 -1").contains("b:1: dash lengths cannot be negative"));
    }

    #[test]
    fn a_blank_grid_draws_nothing() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        assert!(read(&map, &layout, "").unwrap().is_empty());
        assert!(read(&map, &layout, ". . . .\n . . . .\n. . . .\n").unwrap().is_empty());
    }
}
