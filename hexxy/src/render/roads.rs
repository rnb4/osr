//! Reading the roads grid and laying out the network of ways it describes.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Display,
    fs,
    path::Path,
};

use super::course::{self, Wander};
use crate::{
    hex::{Axial, Layout, Point, Top},
    map::{self, Coord, Map},
    warn,
};

/// Tags keeping roads' straying apart from the rivers they cross; see [`Wander`].
pub const CROSSING: u64 = 20;
pub const BEND: u64 = 21;

/// One kind of way, named by the character standing for it on the grid; see [`super::stroke::Kind`].
pub type Class = super::stroke::Kind;

pub fn read_classes(path: &Path) -> Result<Vec<Class>, String> {
    let text =
        fs::read_to_string(path).map_err(|err| format!("could not read road classes {}: {err}", path.display()))?;
    parse_classes(&text, path.display())
}

/// Parses '<char> <color> <width> <dash> [name]' lines.
///
/// The classes are kept in a file rather than given as flags, the way the palette and the border types are: a map with
/// a highway, a road and a trail on it wants the list written down once, not retyped on every run.
///
/// The classes stay in the order they were written, because that is the order they are drawn in.
fn parse_classes(text: &str, file: impl Display) -> Result<Vec<Class>, String> {
    let mut classes: Vec<Class> = Vec::new();
    for (line, n) in map::content_lines(text) {
        let class = super::stroke::parse_kind(line, "road class", "t #6b4f2a 0.05 0.18,0.12 trail")
            .map_err(|err| format!("{file}:{n}: {err}"))?;
        if classes.iter().any(|existing| existing.key == class.key) {
            return Err(format!("{file}:{n}: duplicate road class '{}'", class.key));
        }
        classes.push(class);
    }
    if classes.is_empty() {
        return Err(format!("{file}: no road classes"));
    }
    Ok(classes)
}

/// One run of road, as the curve a stroke follows.
#[derive(Debug)]
pub struct Road {
    /// The class it is drawn in, as an index into the classes read alongside it.
    pub class: usize,
    pub anchors: Vec<Point>,
    /// One shorter than `anchors`: the control for the curve from anchor `i` to anchor `i + 1`.
    pub controls: Vec<Point>,
}

pub fn read_roads(
    path: &Path,
    map: &Map,
    layout: &Layout,
    top: Top,
    classes: &[Class],
    wander: &Wander,
) -> Result<Vec<Road>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("could not read roads {}: {err}", path.display()))?;
    parse_roads(&text, path.display(), map, layout, top, classes, wander)
}

/// Parses a staggered grid of road hexes and lays out every run of way it joins up.
fn parse_roads(
    text: &str,
    file: impl Display,
    map: &Map,
    layout: &Layout,
    top: Top,
    classes: &[Class],
    wander: &Wander,
) -> Result<Vec<Road>, String> {
    let hexes = road_hexes(text, &file, map, top, classes)?;

    let mut roads = Vec::new();
    for class in 0..classes.len() {
        let links = links(&hexes, classes, class);
        for chain in chains(&links) {
            let line = course::centreline(&chain, layout, |_| 0.0, wander);
            roads.push(Road { class, anchors: line.anchors, controls: line.controls });
        }
    }

    // A road running nowhere has no edge to draw along, so it would silently vanish.
    for (hex, (_, coord)) in &hexes {
        if Axial::NEIGHBOURS.into_iter().all(|step| !hexes.contains_key(&(*hex + step))) {
            warn(format_args!("{file}: the road hex at {coord} stands alone, so nothing is drawn for it"));
        }
    }
    Ok(roads)
}

/// The class of every road hex, with the coordinate it was written at.
fn road_hexes(
    text: &str,
    file: impl Display,
    map: &Map,
    top: Top,
    classes: &[Class],
) -> Result<BTreeMap<Axial, (usize, Coord)>, String> {
    let mut hexes = BTreeMap::new();
    let mut unknown: BTreeMap<char, Vec<String>> = BTreeMap::new();
    for (axial, cell) in map::place_grid(text, &file, map, top)? {
        match classes.iter().position(|class| class.key == cell.ch) {
            Some(class) => {
                hexes.insert(axial, (class, cell.coord));
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
        let known: Vec<String> = classes.iter().map(|class| format!("'{}' {}", class.key, class.name)).collect();
        return Err(format!(
            "unknown road characters (add them to the road classes file; so far {}):\n{}",
            known.join(", "),
            lines.join("\n")
        ));
    }
    Ok(hexes)
}

/// Which hexes `class` is drawn between: every pair of neighbouring road hexes, given to the narrower
/// of the two classes meeting there.
///
/// A trail running into a highway hex is therefore a trail all the way to that hex's centre, and the
/// highway stays a highway. Both runs end on the same bend point, so the join needs nothing of its own.
fn links(hexes: &BTreeMap<Axial, (usize, Coord)>, classes: &[Class], class: usize) -> BTreeMap<Axial, BTreeSet<Axial>> {
    // The narrower class wins, and where two are drawn at the same width the one given first does, so
    // the choice never turns on the order a map happens to be read in.
    let rank = |class: usize| (classes[class].width, class);
    let mut links: BTreeMap<Axial, BTreeSet<Axial>> = BTreeMap::new();
    for (&hex, &(mine, _)) in hexes {
        for step in Axial::NEIGHBOURS {
            let Some(&(theirs, _)) = hexes.get(&(hex + step)) else { continue };
            if class == if rank(mine) <= rank(theirs) { mine } else { theirs } {
                links.entry(hex).or_default().insert(hex + step);
            }
        }
    }
    links
}

/// Splits one class's links into the longest runs that can each be drawn as a single stroke.
///
/// Drawing every link on its own would work, but a dashed class would restart its pattern at each hex,
/// which reads as a row of ticks rather than a trail.
fn chains(links: &BTreeMap<Axial, BTreeSet<Axial>>) -> Vec<Vec<Axial>> {
    let mut taken: BTreeSet<(Axial, Axial)> = BTreeSet::new();
    let mut chains = Vec::new();

    // Ends and junctions first, so a run is drawn in one piece; whatever is left is a closed loop, which
    // any of its hexes can start.
    let ends = links.iter().filter(|(_, next)| next.len() != 2).map(|(&hex, _)| hex);
    for start in ends.chain(links.keys().copied()) {
        for &next in &links[&start] {
            if let Some(chain) = walk(links, start, next, &mut taken) {
                chains.push(chain);
            }
        }
    }
    chains
}

/// Follows the run leaving `start` towards `next` until it reaches an end, a junction, or its own tail,
/// marking the links it takes. Gives nothing back when that run has already been drawn.
fn walk(
    links: &BTreeMap<Axial, BTreeSet<Axial>>,
    start: Axial,
    next: Axial,
    taken: &mut BTreeSet<(Axial, Axial)>,
) -> Option<Vec<Axial>> {
    if !taken.insert(link(start, next)) {
        return None;
    }
    let mut chain = vec![start, next];
    let mut at = next;
    // A hex with two links is somewhere in the middle of a run, so the way carries on through it.
    while links[&at].len() == 2 {
        let behind = chain[chain.len() - 2];
        let on = *links[&at].iter().find(|&&hex| hex != behind).expect("the other of two links");
        if !taken.insert(link(at, on)) {
            break;
        }
        chain.push(on);
        at = on;
    }
    Some(chain)
}

/// A link named the same way from either end.
fn link(a: Axial, b: Axial) -> (Axial, Axial) {
    if a < b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::{Class, Road, parse_classes, parse_roads};
    use crate::{
        hex::{Layout, Point, Top},
        map::{Map, Palette, parse_map, parse_palette},
        render::course::Wander,
    };

    const PALETTE: &str = "g #95c763 clear\n~ #78c7f5 water\n";

    // Four columns by three rows, the same fixture the rivers use.
    const MAP: &str = "g g g g\n g g g g\ng g g ~\n";

    const CLASSES: &str = "R #6b4f2a 0.09 none road\nt #6b4f2a 0.05 0.18,0.12 trail\n";

    fn palette() -> Palette {
        parse_palette(PALETTE, "p").unwrap()
    }

    fn layout(map: &Map) -> Layout {
        Layout::fit(Top::Pointy, 10.0, map.hexes.iter().map(|hex| hex.axial)).0
    }

    fn classes() -> Vec<Class> {
        parse_classes(CLASSES, "r").unwrap()
    }

    fn wander(amount: f64) -> Wander {
        Wander { amount, seed: 7, crossing_tag: super::CROSSING, bend_tag: super::BEND }
    }

    fn read(map: &Map, layout: &Layout, roads: &str, amount: f64) -> Result<Vec<Road>, String> {
        parse_roads(roads, "d", map, layout, Top::Pointy, &classes(), &wander(amount))
    }

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    #[test]
    fn adjacent_road_hexes_join_into_one_run() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        // A way along the bottom row: one run, with an anchor on each of its three crossings and one at
        // either end.
        let roads = read(&map, &layout, ". . . .\n . . . .\nR R R R\n", 0.0).unwrap();
        assert_eq!(roads.len(), 1);
        assert_eq!(roads[0].anchors.len(), 5);
        assert_eq!(roads[0].controls.len(), roads[0].anchors.len() - 1);

        // Two ways with a gap between them stay two runs.
        assert_eq!(read(&map, &layout, "R R . .\n . . . .\n. . R R\n", 0.0).unwrap().len(), 2);
        assert!(read(&map, &layout, "", 0.0).unwrap().is_empty(), "an empty grid draws nothing");
    }

    #[test]
    fn a_junction_splits_the_runs_meeting_there() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        // Three arms on one hex. They are every other neighbour of it, because two neighbours next to
        // each other in the ring touch, which would close a loop round the junction instead.
        let roads = read(&map, &layout, ". R . .\n . R R .\n. R . .\n", 0.0).unwrap();
        assert_eq!(roads.len(), 3, "three arms leave the junction");
        // Each arm is one link: an anchor at either end and one on the crossing between them.
        assert!(roads.iter().all(|road| road.anchors.len() == 3), "{roads:?}");
    }

    #[test]
    fn a_loop_is_drawn_as_one_run() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        // Three hexes all touching each other: every hex has two links, so nothing is an end.
        let roads = read(&map, &layout, "R R . .\n R . . .\n. . . .\n", 0.0).unwrap();
        assert_eq!(roads.len(), 1);
        assert!(close(roads[0].anchors[0], *roads[0].anchors.last().unwrap()), "the loop does not close");
    }

    #[test]
    fn a_way_meeting_a_wider_one_is_drawn_up_to_its_centre() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        // A road of three hexes with a trail coming onto the middle one.
        let roads = read(&map, &layout, ". R . .\n . R R .\n. t . .\n", 0.4).unwrap();
        let (trails, ways): (Vec<&Road>, Vec<&Road>) = roads.iter().partition(|road| road.class == 1);
        assert_eq!((trails.len(), ways.len()), (1, 1), "the trail is one run and the road another");

        // The link between the two classes belongs to the trail, so the road is still its own three hexes.
        assert_eq!(ways[0].anchors.len(), 4);
        assert_eq!(trails[0].anchors.len(), 3);

        // The road passes through the junction hex, so its bend there is a control; the trail stops on
        // that same point, and the two meet exactly however far they stray.
        let junction = map.hexes.iter().find(|hex| hex.coord.col == 1 && hex.coord.row == 1).unwrap().axial;
        let meeting = ways[0].controls[1];
        let ends = [trails[0].anchors[0], *trails[0].anchors.last().unwrap()];
        assert!(ends.iter().any(|&end| close(end, meeting)), "{ends:?} misses the road at {meeting:?}");
        assert!(!close(meeting, layout.centre(junction)), "nothing strayed, so this proves little");
    }

    #[test]
    fn road_hexes_must_be_on_the_map_and_in_step_with_it() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        let err = read(&map, &layout, ". . . .\n . . . .\nR R R R R\n", 0.0).unwrap_err();
        assert!(err.contains("d:3: no hex at 4,2"), "{err}");

        // Indenting every row by one more space still puts every hex on the map, so nothing else
        // notices; the hexes are simply not the ones the grid appears to name.
        let shifted: String = "R . . .\n R . . .\nR R R R\n".lines().map(|line| format!(" {line}\n")).collect();
        let err = read(&map, &layout, &shifted, 0.0).unwrap_err();
        assert!(err.contains("staggered the other way"), "{err}");
    }

    #[test]
    fn characters_with_no_class_are_all_reported() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        let err = read(&map, &layout, "x x x x\n R . . .\n. . . .\n", 0.0).unwrap_err();
        assert!(err.contains("'x' at d:1 col 0, d:1 col 1, d:1 col 2 and 1 more"), "{err}");
        assert!(err.contains("'R' road") && err.contains("'t' trail"), "{err}");
    }

    #[test]
    fn a_lone_road_hex_draws_nothing() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);

        assert!(read(&map, &layout, "R . . .\n . . . .\n. . . .\n", 0.0).unwrap().is_empty());
    }

    #[test]
    fn the_same_seed_always_draws_the_same_road() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);
        let grid = ". . . .\n . . . .\nR R R R\n";

        let straight = read(&map, &layout, grid, 0.0).unwrap();
        let once = read(&map, &layout, grid, 0.4).unwrap();
        let again = read(&map, &layout, grid, 0.4).unwrap();
        assert!(once[0].anchors.iter().zip(&again[0].anchors).all(|(a, b)| close(*a, *b)));
        assert!(once[0].anchors.iter().zip(&straight[0].anchors).any(|(a, b)| !close(*a, *b)), "nothing strayed");

        // A crossing never strays past the corner of the edge it sits on: the edge is one radius long,
        // so half a radius from its midpoint is the limit.
        for (strayed, middle) in once[0].anchors.iter().zip(&straight[0].anchors) {
            assert!((*strayed - *middle).length() <= layout.radius() / 2.0, "strayed past the corner");
        }
    }

    #[test]
    fn class_files_are_checked() {
        let classes = classes();
        assert_eq!(classes.iter().map(|class| class.key).collect::<Vec<char>>(), ['R', 't']);
        assert_eq!(classes[0].name, "road");
        assert!(classes[0].dash.is_empty());
        assert_eq!(classes[1].dash, [0.18, 0.12]);
        assert_eq!(parse_classes("t #6b4f2a 0.05 0.18,0.12 mule trail\n", "r").unwrap()[0].name, "mule trail");
        assert_eq!(
            parse_classes("R #6b4f2a 0.09 none\n", "r").unwrap()[0].name,
            "R",
            "an unnamed class is called after its key"
        );

        let err = |text: &str| parse_classes(text, "r").unwrap_err();
        assert!(err("R #000 0.09 none road\nR #fff 0.05 none path\n").contains("r:2: duplicate road class 'R'"));
        assert!(err("# only a comment\n").contains("r: no road classes"));
        assert!(err(". #6b4f2a 0.09 none").contains("r:1: '.' means \"no hex here\""));
        assert!(err("RR #6b4f2a 0.09 none").contains("r:1: road class key must be one character"));
        assert!(err("R #6b4f2a 0 none").contains("r:1: width must be a number more than zero"));
        assert!(err("R #6b4f2a 0.09").contains("r:1: expected '<char> <color> <width> <dash> [name]'"));
        assert!(err("R #6b4f2a wide none").contains("r:1: width"));
        assert!(err("R #6b4f2a 0.09 3,x").contains("r:1: dash"));
    }
}
