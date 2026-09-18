//! Reading the rivers grid and drawing the water courses it describes.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
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

/// Tags keeping rivers' straying apart from the roads crossing them; see [`Wander`].
pub const CROSSING: u64 = 10;
pub const BEND: u64 = 11;

/// How wide a course is drawn, in px.
#[derive(Clone, Copy, Debug)]
pub struct Channel {
    /// The width of a course with one hex draining into it.
    pub headwater: f64,
    /// The widest a course is drawn, however much drains into it.
    pub max: f64,
}

impl Channel {
    /// The width of a course carrying `drains` hexes' worth of water.
    ///
    /// Real channels widen roughly with the square root of their discharge, and it draws well: a
    /// trunk stays clearly wider than the tributaries feeding it without swamping the hexes it runs
    /// through, which growing in step with the catchment would.
    fn width(&self, drains: u32) -> f64 {
        (self.headwater * f64::from(drains).sqrt()).min(self.max)
    }
}

/// One source-to-mouth course, as a closed outline to fill.
#[derive(Debug)]
pub struct Course {
    pub outline: Vec<Point>,
}

/// The river hexes, and which way the water runs between them.
#[derive(Debug)]
struct Network {
    /// Every river hex, with the coordinate it was written at.
    coords: BTreeMap<Axial, Coord>,
    /// The neighbour each hex drains into. The mouths are the hexes with no entry.
    downstream: BTreeMap<Axial, Axial>,
    /// The hexes from the mouths upward, so walking it backwards visits every hex before the one it
    /// drains into.
    order: Vec<Axial>,
}

pub fn read_rivers(
    path: &Path,
    map: &Map,
    layout: &Layout,
    top: Top,
    mouth: char,
    channel: Channel,
    wander: &Wander,
) -> Result<Vec<Course>, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("could not read rivers {}: {err}", path.display()))?;
    parse_rivers(&text, path.display(), map, layout, top, mouth, channel, wander)
}

/// Parses a staggered grid of river hexes and draws each source's course down to its mouth.
#[allow(clippy::too_many_arguments)]
fn parse_rivers(
    text: &str,
    file: impl Display,
    map: &Map,
    layout: &Layout,
    top: Top,
    mouth: char,
    channel: Channel,
    wander: &Wander,
) -> Result<Vec<Course>, String> {
    let network = network(text, &file, map, top, mouth)?;
    if network.coords.is_empty() {
        return Ok(Vec::new());
    }

    let drains = drainage(&network.order, &network.downstream);
    // A source is a river hex nothing drains into. Each one gets its own course, so hexes on a
    // shared trunk are drawn once per source above them. The courses overlap exactly, which is
    // invisible in one flat colour, and it saves stitching a branching outline.
    let fed: BTreeSet<Axial> = network.downstream.values().copied().collect();
    let mut courses = Vec::new();
    for (&source, coord) in &network.coords {
        if fed.contains(&source) {
            continue;
        }
        let mut chain = vec![source];
        while let Some(&next) = network.downstream.get(chain.last().expect("the chain starts with its source")) {
            chain.push(next);
        }
        if chain.len() < 2 {
            warn(format_args!("{file}: the river hex at {coord} stands alone, so nothing is drawn for it"));
            continue;
        }
        let line = course::centreline(&chain, layout, |hex| channel.width(drains[&hex]), wander);
        courses.push(Course { outline: course::outline(&line) });
    }
    Ok(courses)
}

/// Reads the grid and works out which way the water runs.
fn network(text: &str, file: impl Display, map: &Map, top: Top, mouth: char) -> Result<Network, String> {
    // Every river hex takes the map's own axial for its coordinate, so a rivers grid that is out of
    // step with the map fails there rather than drawing the whole network half a hex out.
    let mut coords: BTreeMap<Axial, Coord> = BTreeMap::new();
    let mut mouths: BTreeSet<Axial> = BTreeSet::new();
    for (axial, cell) in map::place_grid(text, &file, map, top)? {
        coords.insert(axial, cell.coord);
        if cell.ch == mouth {
            mouths.insert(axial);
        }
    }
    if coords.is_empty() {
        return Ok(Network { coords, downstream: BTreeMap::new(), order: Vec::new() });
    }
    if mouths.is_empty() {
        return Err(format!("{file}: no river has a mouth; mark where one ends with '{mouth}'"));
    }

    // Breadth-first from the mouths, so every hex drains toward the nearest one and the network
    // comes out a forest rooted at the mouths. Adjacency alone decides this, so the order of
    // `NEIGHBOURS` does not matter.
    let mut downstream = BTreeMap::new();
    let mut order: Vec<Axial> = mouths.iter().copied().collect();
    let mut seen: BTreeSet<Axial> = mouths.clone();
    let mut queue: VecDeque<Axial> = mouths.iter().copied().collect();
    while let Some(hex) = queue.pop_front() {
        for step in Axial::NEIGHBOURS {
            let up = hex + step;
            if coords.contains_key(&up) && seen.insert(up) {
                downstream.insert(up, hex);
                order.push(up);
                queue.push_back(up);
            }
        }
    }

    let stranded: Vec<String> =
        coords.iter().filter(|(hex, _)| !seen.contains(hex)).map(|(_, coord)| coord.to_string()).collect();
    if !stranded.is_empty() {
        let shown = stranded.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        let more = if stranded.len() > 3 { format!(" and {} more", stranded.len() - 3) } else { String::new() };
        return Err(format!("{file}: river hexes at {shown}{more} reach no mouth; mark where they end with '{mouth}'"));
    }

    Ok(Network { coords, downstream, order })
}

/// How many hexes drain into each one, counting itself.
///
/// A tributary hands its whole catchment to the hex it joins, so a trunk grows at every confluence
/// without confluences being a case of their own.
fn drainage(order: &[Axial], downstream: &BTreeMap<Axial, Axial>) -> BTreeMap<Axial, u32> {
    let mut drains: BTreeMap<Axial, u32> = order.iter().map(|&hex| (hex, 1)).collect();
    // Furthest upstream first, so a hex's own count is final before it is added to the one below it.
    for hex in order.iter().rev() {
        if let Some(down) = downstream.get(hex) {
            let carried = drains[hex];
            *drains.get_mut(down).expect("a hex drains into another river hex") += carried;
        }
    }
    drains
}

#[cfg(test)]
mod tests {
    use super::{Channel, Network, Wander, course, drainage, network, parse_rivers};
    use crate::{
        hex::{Axial, Layout, Point, Top},
        map::{Map, Palette, parse_map, parse_palette},
    };

    const PALETTE: &str = "g #95c763 clear\n~ #78c7f5 water\n";

    // Four columns by three rows, with the sea in the bottom right corner.
    const MAP: &str = "g g g g\n g g g g\ng g g ~\n";

    // One course, from the top left corner down and along to the sea. The bottom left hex is left
    // out on purpose: it touches both the hex above it and the one along the row, so it would hang
    // off the trunk as a second, stub course rather than joining the chain.
    const RIVERS: &str = "r . . .\n r . . .\n. r r M\n";

    const CHANNEL: Channel = Channel { headwater: 2.0, max: 100.0 };

    fn palette() -> Palette {
        parse_palette(PALETTE, "p").unwrap()
    }

    fn layout(map: &Map) -> Layout {
        Layout::fit(Top::Pointy, 10.0, map.hexes.iter().map(|hex| hex.axial)).0
    }

    fn wander(amount: f64) -> Wander {
        Wander { amount, seed: 7, crossing_tag: super::CROSSING, bend_tag: super::BEND }
    }

    /// The course's curve, the way [`parse_rivers`] lays it out.
    fn centreline(
        chain: &[Axial],
        layout: &Layout,
        drains: &std::collections::BTreeMap<Axial, u32>,
        wander: &Wander,
    ) -> course::Centreline {
        course::centreline(chain, layout, |hex| CHANNEL.width(drains[&hex]), wander)
    }

    /// Every hex a course runs through, from its source down to its mouth.
    fn chain_from(net: &Network, source: Axial) -> Vec<Axial> {
        let mut chain = vec![source];
        while let Some(&down) = net.downstream.get(chain.last().unwrap()) {
            chain.push(down);
        }
        chain
    }

    /// The chains of every source in the network.
    fn chains(net: &Network) -> Vec<Vec<Axial>> {
        let fed: Vec<Axial> = net.downstream.values().copied().collect();
        net.coords.keys().copied().filter(|hex| !fed.contains(hex)).map(|source| chain_from(net, source)).collect()
    }

    fn read(map: &Map, rivers: &str) -> Result<Network, String> {
        network(rivers, "r", map, Top::Pointy, 'M')
    }

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    #[test]
    fn rivers_flow_to_their_marked_mouth() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let net = read(&map, RIVERS).unwrap();

        assert_eq!(net.coords.len(), 5);
        // Exactly one hex has nowhere to drain to, and following any hex down reaches it.
        let mouths: Vec<Axial> = net.coords.keys().copied().filter(|hex| !net.downstream.contains_key(hex)).collect();
        assert_eq!(mouths.len(), 1, "{mouths:?}");
        for &hex in net.coords.keys() {
            let mut at = hex;
            let mut steps = 0;
            while let Some(&down) = net.downstream.get(&at) {
                at = down;
                steps += 1;
                assert!(steps <= net.coords.len(), "{hex:?} loops");
            }
            assert_eq!(at, mouths[0], "{hex:?} drains somewhere else");
        }
    }

    #[test]
    fn tributaries_widen_the_trunk() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        // Two headwaters in the top corners joining the trunk along the bottom row.
        let net = read(&map, "r . . r\n r . r .\nr r r M\n").unwrap();
        let drains = drainage(&net.order, &net.downstream);

        let at = |col, row| {
            let hex = map.hexes.iter().find(|hex| hex.coord.col == col && hex.coord.row == row).expect("on the map");
            drains[&hex.axial]
        };
        // Each headwater carries only itself; the trunk carries everything above it, and the mouth
        // carries the lot.
        assert_eq!((at(0, 0), at(3, 0)), (1, 1));
        assert!(at(2, 2) > at(1, 2), "{} should beat {}", at(2, 2), at(1, 2));
        assert_eq!(at(3, 2), net.coords.len() as u32);

        assert!(CHANNEL.width(at(3, 2)) > CHANNEL.width(at(0, 0)));
        assert_eq!(Channel { headwater: 2.0, max: 3.0 }.width(at(3, 2)), 3.0, "the maximum caps the width");
    }

    #[test]
    fn straight_courses_cross_edges_halfway_between_centres() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);
        let net = read(&map, RIVERS).unwrap();
        let drains = drainage(&net.order, &net.downstream);

        let chain = chain_from(&net, *net.order.last().expect("a network has hexes"));
        let line = centreline(&chain, &layout, &drains, &wander(0.0));

        // Without wander the curve starts and ends on a hex centre, and crosses between them halfway.
        assert_eq!(line.anchors.len(), chain.len() + 1);
        assert_eq!(line.controls.len(), line.anchors.len() - 1);
        assert!(close(line.anchors[0], layout.centre(chain[0])));
        assert!(close(*line.anchors.last().unwrap(), layout.centre(*chain.last().unwrap())));
        for (i, pair) in chain.windows(2).enumerate() {
            let crossing = layout.centre(pair[0]).midpoint(layout.centre(pair[1]));
            assert!(close(line.anchors[i + 1], crossing), "anchor {}", i + 1);
        }
        // The source tapers to a point and the mouth is the widest part.
        assert_eq!(line.widths[0], 0.0);
        assert!(line.widths.last().unwrap() > &line.widths[1]);
    }

    #[test]
    fn each_source_starts_its_own_course() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);
        // Wander on, so straying cannot quietly change which courses there are.
        let courses =
            |rivers| parse_rivers(rivers, "r", &map, &layout, Top::Pointy, 'M', CHANNEL, &wander(0.4)).unwrap();

        assert_eq!(courses(RIVERS).len(), 1);
        // Two rivers side by side, each with its own source and its own mouth.
        assert_eq!(courses("r . . r\n r . . r\n. M . M\n").len(), 2);
        assert!(courses("").is_empty(), "an empty grid draws nothing");
        // Every outline is a closed band, so it has two banks' worth of points.
        assert!(courses(RIVERS)[0].outline.len() % 2 == 0);
    }

    #[test]
    fn rivers_that_reach_nowhere_are_reported() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();

        let err = read(&map, "r . . .\n r . . .\nr r r r\n").unwrap_err();
        assert!(err.contains("no river has a mouth"), "{err}");

        // A second network with no mouth of its own, off on the far side of the map.
        let err = read(&map, "r . . r\n r . . .\nr r r M\n").unwrap_err();
        assert!(err.contains("reach no mouth") && err.contains("3,0"), "{err}");
    }

    #[test]
    fn river_hexes_must_be_on_the_map() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        // The map's bottom row is four hexes wide; this grid puts a river in a fifth.
        let err = read(&map, "r . . .\n r . . .\nr r r M r\n").unwrap_err();
        assert!(err.contains("r:3: no hex at 4,2"), "{err}");
    }

    #[test]
    fn a_grid_out_of_step_with_the_map_is_reported() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        // Indenting every row by one more space still puts every hex on the map, so nothing else
        // notices; the hexes are simply not the ones the grid appears to name.
        let shifted: String = RIVERS.lines().map(|line| format!(" {line}\n")).collect();
        let err = read(&map, &shifted).unwrap_err();
        assert!(err.contains("staggered the other way"), "{err}");
    }

    #[test]
    fn wander_moves_a_course_off_the_lattice_but_keeps_it_inside_its_hexes() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);
        let net = read(&map, RIVERS).unwrap();
        let drains = drainage(&net.order, &net.downstream);
        let chain = chain_from(&net, *net.order.last().expect("a network has hexes"));

        let straight = centreline(&chain, &layout, &drains, &wander(0.0));
        let strayed = centreline(&chain, &layout, &drains, &wander(0.4));

        // Same shape of curve, different points.
        assert_eq!(strayed.anchors.len(), straight.anchors.len());
        assert!(
            strayed.anchors.iter().zip(&straight.anchors).any(|(a, b)| !close(*a, *b)),
            "wander left every anchor where it was"
        );
        // A crossing never strays past the corner of the edge it sits on: the edge is one radius
        // long, so half a radius from its midpoint is the limit.
        for (i, pair) in chain.windows(2).enumerate() {
            let middle = layout.centre(pair[0]).midpoint(layout.centre(pair[1]));
            let drift = (strayed.anchors[i + 1] - middle).length();
            assert!(drift <= layout.radius() / 2.0, "crossing {i} strayed {drift} past the corner");
        }
    }

    #[test]
    fn the_same_seed_always_draws_the_same_river() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);
        let net = read(&map, RIVERS).unwrap();
        let drains = drainage(&net.order, &net.downstream);
        let chain = chain_from(&net, *net.order.last().expect("a network has hexes"));

        let once = centreline(&chain, &layout, &drains, &wander(0.4));
        let again = centreline(&chain, &layout, &drains, &wander(0.4));
        assert!(once.anchors.iter().zip(&again.anchors).all(|(a, b)| close(*a, *b)));

        let other = Wander { seed: 8, ..wander(0.4) };
        let elsewhere = centreline(&chain, &layout, &drains, &other);
        assert!(
            once.anchors.iter().zip(&elsewhere.anchors).any(|(a, b)| !close(*a, *b)),
            "a different seed drew the same river"
        );
    }

    #[test]
    fn courses_sharing_a_trunk_stray_together() {
        let palette = palette();
        let map = parse_map(MAP, "m", &palette, Top::Pointy).unwrap();
        let layout = layout(&map);
        // Two headwaters running into one trunk.
        let net = read(&map, "r . . r\n r . r .\nr r r M\n").unwrap();
        let drains = drainage(&net.order, &net.downstream);

        let chains = chains(&net);
        assert!(chains.len() >= 2, "expected more than one source, got {chains:?}");
        let (first, second) = (&chains[0], &chains[1]);
        let shared = first.iter().rev().zip(second.iter().rev()).take_while(|(a, b)| a == b).count();
        assert!(shared >= 2, "the two courses share no trunk");

        // The trunk below a confluence is drawn once per source above it. Those copies land exactly
        // on top of each other only while they agree about where to stray, so compare the tails.
        let a = centreline(first, &layout, &drains, &wander(0.4));
        let b = centreline(second, &layout, &drains, &wander(0.4));
        for i in 1..=shared {
            let (pa, pb) = (a.anchors[a.anchors.len() - i], b.anchors[b.anchors.len() - i]);
            assert!(close(pa, pb), "trunk anchor {i} from the end differs: {pa:?} vs {pb:?}");
        }
    }
}
