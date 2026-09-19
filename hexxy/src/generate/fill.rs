//! Filling atlas hexes with sub-hexes.

use std::collections::{BTreeMap, HashMap};

use fastrand::Rng;

use super::rules::{Beside, BesideRule, Rules};
use crate::{
    hex::{ATLAS_SCALE, Axial},
    random::stable_rng,
};

/// Seeds the rolls deciding an atlas hex's own sub-hexes.
const OWNED: u64 = 1;
/// Seeds the roll for a half-hex shared along atlas edges.
const SHARED: u64 = 2;
/// Seeds the roll for a sub-hex's variant.
const VARIANT: u64 = 3;

/// Steps from an atlas hex's centre to the furthest sub-hex it owns outright.
const OWNED_RADIUS: i32 = ATLAS_SCALE / 2;
const SECONDARY_HEXES: usize = 6;
const TERTIARY_HEXES: usize = 3;
/// Weights of an atlas hex's primary, secondary and tertiary terrain when rolling a half-hex it shares.
const SHARED_WEIGHTS: [u32; 3] = [9, 6, 3];

/// The sub-hex at the centre of an atlas hex.
pub fn centre(atlas_hex: Axial) -> Axial {
    atlas_hex * ATLAS_SCALE
}

/// The base terrain of every sub-hex of `atlas`, which maps atlas hexes to their primary terrain, and how many of the
/// half-hexes are shared by more than one atlas hex.
pub fn terrain<'r>(atlas: &BTreeMap<Axial, &'r str>, rules: &'r Rules, seed: u64) -> (BTreeMap<Axial, &'r str>, usize) {
    let mut cells: BTreeMap<Axial, &'r str> =
        atlas.iter().flat_map(|(&hex, &primary)| owned(hex, primary, atlas, rules, seed)).collect();

    // Half-hexes lie one step beyond the sub-hexes an atlas hex owns, where no atlas hex owns them. Each is shared by
    // every atlas hex that close; at the edge of the map that can be just one.
    let mut parents: BTreeMap<Axial, Vec<&'r str>> = BTreeMap::new();
    for (&hex, &primary) in atlas {
        for cell in centre(hex).ring(OWNED_RADIUS + 1) {
            if !cells.contains_key(&cell) {
                parents.entry(cell).or_default().push(primary);
            }
        }
    }
    let shared = parents.values().filter(|primaries| primaries.len() > 1).count();
    cells.extend(parents.into_iter().map(|(cell, primaries)| (cell, half_hex(cell, &primaries, rules, seed))));
    (cells, shared)
}

/// The 19 sub-hexes an atlas hex owns outright, starting with its centre.
///
/// Around the centre, 6 whole sub-hexes are secondary terrain, 3 tertiary, and the other 9 primary. Secondary and
/// tertiary terrain grow as compact clusters from the edge facing a neighbouring atlas hex of that terrain, so terrain
/// flows across atlas boundaries instead of stopping at them.
fn owned<'r>(
    hex: Axial,
    primary: &'r str,
    atlas: &BTreeMap<Axial, &'r str>,
    rules: &'r Rules,
    seed: u64,
) -> Vec<(Axial, &'r str)> {
    let mut rng = stable_rng(seed, OWNED, hex.key());
    let relations = rules.get(primary);
    let middle = centre(hex);

    // Secondary or tertiary terrain the rules leave out is primary instead, like the sub-hexes no group takes.
    let secondary = pick(&mut rng, &relations.secondary);
    let tertiary = pick(&mut rng, &relations.tertiary);
    let mut groups: Vec<(&str, usize)> = secondary.map(|terrain| (terrain, SECONDARY_HEXES)).into_iter().collect();
    for _ in 0..TERTIARY_HEXES {
        // A tertiary sub-hex may be a wildcard instead, at the adjacent hex table's odds of wildcard against tertiary:
        // 1 in 12 against 2 in 12.
        let wildcard = !relations.wildcard.is_empty() && rng.u32(..3) == 0;
        let Some(terrain) = (if wildcard { pick(&mut rng, &relations.wildcard) } else { tertiary }) else {
            continue;
        };
        match groups.iter_mut().find(|(name, _)| *name == terrain) {
            Some((_, count)) => *count += 1,
            None => groups.push((terrain, 1)),
        }
    }

    let mut cells = vec![(middle, primary)];
    let mut free: Vec<Axial> = middle.disc(OWNED_RADIUS).filter(|&cell| cell != middle).collect();
    for (terrain, count) in groups {
        let toward: Vec<Axial> =
            Axial::NEIGHBOURS.into_iter().filter(|&step| atlas.get(&(hex + step)) == Some(&terrain)).collect();
        let step =
            if toward.is_empty() { Axial::NEIGHBOURS[rng.usize(..6)] } else { toward[rng.usize(..toward.len())] };
        let start = middle + step * OWNED_RADIUS;

        // The free sub-hexes nearest the start, with random tie-breaks so clusters are not perfectly round.
        let mut nearest: Vec<(i32, u32, Axial)> =
            free.iter().map(|&cell| (cell.distance(start), rng.u32(..), cell)).collect();
        nearest.sort_unstable();
        cells.extend(nearest[..count].iter().map(|&(_, _, cell)| (cell, terrain)));
        free = nearest[count..].iter().map(|&(_, _, cell)| cell).collect();
    }
    cells.extend(free.into_iter().map(|cell| (cell, primary)));
    cells
}

/// A half-hex, rolled from the primary, secondary and tertiary terrain of every atlas hex sharing it. Wildcards never
/// appear on half-hexes.
fn half_hex<'r>(cell: Axial, primaries: &[&'r str], rules: &'r Rules, seed: u64) -> &'r str {
    let [primary_weight, secondary_weight, tertiary_weight] = SHARED_WEIGHTS;
    let mut pool: Vec<(&'r str, u32)> = Vec::new();
    for &primary in primaries {
        let relations = rules.get(primary);
        let options = std::iter::once((relations.name.as_str(), primary_weight))
            .chain(relations.secondary.iter().map(|name| (name.as_str(), secondary_weight)))
            .chain(relations.tertiary.iter().map(|name| (name.as_str(), tertiary_weight)));
        for (terrain, weight) in options {
            match pool.iter_mut().find(|(name, _)| *name == terrain) {
                Some((_, total)) => *total += weight,
                None => pool.push((terrain, weight)),
            }
        }
    }
    let weights: Vec<u32> = pool.iter().map(|&(_, weight)| weight).collect();
    pool[weighted(&mut stable_rng(seed, SHARED, cell.key()), &weights)].0
}

/// The palette variant for each sub-hex in `cells`, which maps sub-hexes to their base terrain.
///
/// A sub-hex within reach of the terrain a `beside` rule names takes that variant. Otherwise a sub-hex keeps the
/// variant in `drawn` if its terrain has it, as an atlas hex's centre does, and the rest roll one of their terrain's
/// variants.
pub fn variants<'r>(
    cells: &BTreeMap<Axial, &'r str>,
    drawn: &HashMap<Axial, String>,
    rules: &'r Rules,
    seed: u64,
) -> BTreeMap<Axial, &'r str> {
    cells
        .iter()
        .map(|(&cell, &terrain)| {
            let relations = rules.get(terrain);
            let near = |rule: &BesideRule| {
                let mut around = cell.disc(rule.reach).filter(|&hex| hex != cell).filter_map(|hex| cells.get(&hex));
                around.any(|&other| match &rule.target {
                    Beside::Other => other != terrain,
                    Beside::Terrain(name) => other == name,
                })
            };
            let beside = relations.beside.iter().find(|rule| near(rule)).map(|rule| &rule.variant);
            let kept = || {
                let name = drawn.get(&cell)?;
                relations.variants.iter().map(|(variant, _)| variant).find(|&variant| variant == name)
            };
            let rolled = || {
                let weights: Vec<u32> = relations.variants.iter().map(|&(_, weight)| weight).collect();
                &relations.variants[weighted(&mut stable_rng(seed, VARIANT, cell.key()), &weights)].0
            };
            (cell, beside.or_else(kept).unwrap_or_else(rolled).as_str())
        })
        .collect()
}

/// A random one of `options`, if there are any.
fn pick<'r>(rng: &mut Rng, options: &'r [String]) -> Option<&'r str> {
    (!options.is_empty()).then(|| options[rng.usize(..options.len())].as_str())
}

/// The index of a random choice among `weights`.
fn weighted(rng: &mut Rng, weights: &[u32]) -> usize {
    let mut roll = rng.u32(..weights.iter().sum::<u32>());
    weights
        .iter()
        .position(|&weight| {
            let hit = roll < weight;
            roll = roll.wrapping_sub(weight);
            hit
        })
        .expect("the roll is below the total weight")
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};

    use super::{centre, owned, terrain, variants};
    use crate::{generate::rules::Rules, hex::Axial, map::parse_palette};

    const PALETTE: &str =
        "f #1 forest\nl #2 light-forest\nF #3 heavy-forest\np #4 plains\nh #5 hills\nw #6 water\n~ #7 coast\n";

    const RULES: &str = "
[forest]
secondary plains
tertiary  hills
wildcard  water

[plains]
secondary forest
tertiary  hills

[hills]
secondary plains
tertiary  forest

[water]
secondary plains
tertiary  forest
";

    const VARIANT_RULES: &str = "
[forest]
secondary water
tertiary  water
variants  light-forest 2  heavy-forest 1

[water]
secondary forest
tertiary  forest
beside    other coast
";

    fn rules_with_reach(reach: i32) -> Rules {
        rules(&VARIANT_RULES.replace("beside    other coast", &format!("beside    other coast {reach}")))
    }

    fn rules(text: &str) -> Rules {
        let palette = parse_palette(PALETTE, "p").unwrap();
        Rules::parse(&crate::defs::split(text, "r").unwrap(), "r", &palette).unwrap()
    }

    #[test]
    fn owned_sub_hexes_split_nine_six_three() {
        let rules = rules(RULES);
        let atlas = BTreeMap::from([(Axial::default(), "forest")]);
        let mut wildcards = 0;
        for seed in 0..60 {
            let cells = owned(Axial::default(), "forest", &atlas, &rules, seed);
            assert_eq!(cells.len(), 19);
            assert_eq!(cells[0], (Axial::default(), "forest"));
            let count = |name| cells[1..].iter().filter(|&&(_, terrain)| terrain == name).count();
            assert_eq!((count("forest"), count("plains")), (9, 6), "seed {seed}");
            assert_eq!(count("hills") + count("water"), 3, "seed {seed}");
            wildcards += count("water");
        }
        assert!((1..60 * 3).contains(&wildcards), "{wildcards} wildcards");
    }

    #[test]
    fn missing_secondary_and_tertiary_terrain_stays_primary() {
        let rules = rules("[water]\nsecondary\nwildcard forest\n\n[forest]\nsecondary water\ntertiary water\n");
        let atlas = BTreeMap::from([(Axial::default(), "water")]);
        for seed in 0..30 {
            let cells = owned(Axial::default(), "water", &atlas, &rules, seed);
            let forest = cells.iter().filter(|&&(_, terrain)| terrain == "forest").count();
            assert_eq!(cells.len(), 19);
            assert!(forest <= 3, "seed {seed}: {forest} forest");
        }
        let (cells, _) = terrain(&atlas, &rules, 1);
        let owned: Vec<Axial> = centre(Axial::default()).disc(2).collect();
        assert!(cells.iter().filter(|(cell, _)| !owned.contains(cell)).all(|(_, &terrain)| terrain == "water"));
    }

    #[test]
    fn half_hexes_never_use_wildcards() {
        let rules = rules(RULES);
        let atlas = BTreeMap::from([(Axial::new(0, 0), "forest"), (Axial::new(1, 0), "plains")]);
        let owned: Vec<Axial> = atlas.keys().flat_map(|&hex| centre(hex).disc(2)).collect();
        for seed in 0..40 {
            let (cells, shared) = terrain(&atlas, &rules, seed);
            assert_eq!(shared, 2);
            for (cell, name) in cells.iter().filter(|(cell, _)| !owned.contains(cell)) {
                assert!(["forest", "plains", "hills"].contains(name), "seed {seed}: {cell:?} is {name}");
            }
        }
    }

    #[test]
    fn rolls_are_stable_and_local() {
        let rules = rules(RULES);
        let home = Axial::new(0, 0);
        let one = BTreeMap::from([(home, "forest"), (Axial::new(3, 0), "plains")]);
        let other = BTreeMap::from([(home, "forest"), (Axial::new(3, 0), "hills")]);
        assert_eq!(terrain(&one, &rules, 9), terrain(&one, &rules, 9));
        assert_eq!(owned(home, "forest", &one, &rules, 9), owned(home, "forest", &other, &rules, 9));
        assert_ne!(owned(home, "forest", &one, &rules, 9), owned(home, "forest", &one, &rules, 10));
    }

    #[test]
    fn variants_follow_beside_rules_drawn_centres_and_weights() {
        let rules = rules(VARIANT_RULES);
        let mut cells =
            BTreeMap::from([(Axial::new(0, 0), "water"), (Axial::new(1, 0), "water"), (Axial::new(2, 0), "forest")]);
        let drawn =
            HashMap::from([(Axial::new(0, 0), "coast".to_string()), (Axial::new(2, 0), "heavy-forest".to_string())]);
        let chosen = variants(&cells, &drawn, &rules, 1);
        assert_eq!(chosen[&Axial::new(0, 0)], "water");
        assert_eq!(chosen[&Axial::new(1, 0)], "coast");
        assert_eq!(chosen[&Axial::new(2, 0)], "heavy-forest");

        let far = rules_with_reach(2);
        let shore = BTreeMap::from([
            (Axial::new(0, 0), "water"),
            (Axial::new(1, 0), "water"),
            (Axial::new(2, 0), "water"),
            (Axial::new(3, 0), "forest"),
        ]);
        let chosen = variants(&shore, &HashMap::new(), &far, 1);
        let water: Vec<&str> = (0..3).map(|q| chosen[&Axial::new(q, 0)]).collect();
        assert_eq!(water, ["water", "coast", "coast"]);

        cells.extend((10..3010).map(|q| (Axial::new(q, 0), "forest")));
        let chosen = variants(&cells, &HashMap::new(), &rules, 1);
        let light = chosen.values().filter(|&&variant| variant == "light-forest").count();
        assert!((1800..2200).contains(&light), "{light} light forest of 3001");
    }
}
