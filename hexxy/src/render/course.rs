//! The curve anything running hex to hex follows, and the straying that keeps it off the lattice.
//!
//! A line crosses each hex from one edge to another, so putting the anchors on the crossings and the
//! controls on the hex centres gives the rounded bends hex maps are drawn with, with no smoothing pass
//! of its own. Rivers fill the band this lays out; roads stroke it.

use std::f64::consts::TAU;

use crate::{
    hex::{Axial, Layout, Point},
    random::stable_rng,
};

/// Samples per curve segment when a line is flattened into its outline. Six is enough that the bends
/// read as smooth at any grid size this tool is used at.
const STEPS: usize = 6;

/// How far a line strays from the regular lattice, and which way.
#[derive(Clone, Copy, Debug)]
pub struct Wander {
    /// As a fraction of the hex radius. Zero runs every line dead straight between hex centres, which
    /// reads as drawn by a machine.
    pub amount: f64,
    /// Picks which way each line strays, so the same seed always draws the same map.
    pub seed: u64,
    /// Tags keeping one kind of line's straying apart from another's. A road crossing a river shares
    /// its hexes, and sharing a tag would lay the road exactly along the water.
    pub crossing_tag: u64,
    pub bend_tag: u64,
}

impl Wander {
    /// Where a line crosses out of `hex` into `next`.
    ///
    /// Keyed on the pair of hexes rather than on the line, so every line of this kind crossing here
    /// strays the same way. The trunk below a confluence is drawn once per source above it, and those
    /// copies only stay exactly on top of each other while they agree about this; roads meeting at a
    /// junction lean on the same agreement.
    pub fn crossing(&self, layout: &Layout, hex: Axial, next: Axial) -> Point {
        let (from, to) = (layout.centre(hex), layout.centre(next));
        let middle = from.midpoint(to);
        if self.amount <= 0.0 {
            return middle;
        }
        let (low, high) = if hex < next { (hex, next) } else { (next, hex) };
        let key = low.key().wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ high.key();
        let shift = stable_rng(self.seed, self.crossing_tag, key).f64() * 2.0 - 1.0;
        // The shared edge runs across the line joining the two centres and is one radius long, so half
        // a radius is as far as a crossing could move before reaching a corner.
        middle + (to - from).perpendicular().normalised() * (shift * self.amount * layout.radius() / 2.0)
    }

    /// The point inside `hex` that a line bends around, off the true centre.
    ///
    /// Keyed on the hex for the same reason as [`Wander::crossing`].
    pub fn bend(&self, layout: &Layout, hex: Axial) -> Point {
        let centre = layout.centre(hex);
        if self.amount <= 0.0 {
            return centre;
        }
        let mut rng = stable_rng(self.seed, self.bend_tag, hex.key());
        let angle = rng.f64() * TAU;
        // Kept well inside the hex: the bend only shapes the curve, so a little goes a long way.
        let reach = rng.f64() * self.amount * layout.radius() / 4.0;
        centre + Point::new(angle.cos(), angle.sin()) * reach
    }
}

/// The curve a line follows: the anchors it passes through, the control point bending it between each
/// pair, and the width at each anchor.
#[derive(Debug)]
pub struct Centreline {
    pub anchors: Vec<Point>,
    /// One shorter than `anchors`: the control for the curve from anchor `i` to anchor `i + 1`.
    pub controls: Vec<Point>,
    pub widths: Vec<f64>,
}

/// The curve running the length of `chain`, as wide at each hex as `width` says.
///
/// Neighbouring hex centres sit either side of their shared edge's midpoint, so a crossing starts out
/// as the point halfway between them; [`Wander`] then moves it along the edge, and the centre it bends
/// around off the centre, to keep the line off the lattice. The ends stop on the first and last hexes'
/// bend points, which is what lets a road ending at a junction meet the way running through it.
///
/// The first width is zero, so a river's source comes to a point where its two banks meet; anything
/// stroked at one width, like a road, has no use for the widths at all.
pub fn centreline(chain: &[Axial], layout: &Layout, width: impl Fn(Axial) -> f64, wander: &Wander) -> Centreline {
    let start = wander.bend(layout, chain[0]);

    let mut anchors = vec![start];
    let mut widths = vec![0.0];
    let mut controls = Vec::new();

    for (i, pair) in chain.windows(2).enumerate() {
        let (hex, next) = (pair[0], pair[1]);
        let crossing = wander.crossing(layout, hex, next);
        // The first run leaves the first hex's own centre, so it has no bend to make; later ones turn
        // around the hex they cross.
        controls.push(if i == 0 { start.midpoint(crossing) } else { wander.bend(layout, hex) });
        anchors.push(crossing);
        widths.push((width(hex) + width(next)) / 2.0);
    }

    // The last run crosses into the final hex and stops there.
    let last = *chain.last().expect("a line has at least two hexes");
    let end = wander.bend(layout, last);
    controls.push(anchors.last().expect("the loop pushed an anchor").midpoint(end));
    anchors.push(end);
    widths.push(width(last));

    Centreline { anchors, controls, widths }
}

/// The closed outline of a line: its curve sampled, each sample pushed out to both banks, and the right
/// bank walked back to close the shape.
///
/// Offsetting the curve analytically would give a shorter path, but it is fiddly to get right at tight
/// bends; sampling first is robust, and the extra points cost little beside the terrain polygons already
/// in the file.
pub fn outline(line: &Centreline) -> Vec<Point> {
    let mut samples: Vec<(Point, f64)> = Vec::with_capacity(line.controls.len() * STEPS + 1);
    for (i, &control) in line.controls.iter().enumerate() {
        let (from, to) = (line.anchors[i], line.anchors[i + 1]);
        let (start, end) = (line.widths[i], line.widths[i + 1]);
        // A segment's last sample is the next segment's first, so it is left to that one.
        for step in 0..STEPS {
            let t = f64::from(step as u32) / f64::from(STEPS as u32);
            samples.push((quadratic(from, control, to, t), start + (end - start) * t));
        }
    }
    samples.push((
        *line.anchors.last().expect("a centreline has anchors"),
        *line.widths.last().expect("a centreline has widths"),
    ));

    let mut left = Vec::with_capacity(samples.len() * 2);
    let mut right = Vec::with_capacity(samples.len());
    for (i, &(point, width)) in samples.iter().enumerate() {
        // A sample at either end has only one neighbour to take its direction from.
        let before = samples[i.saturating_sub(1)].0;
        let after = samples[(i + 1).min(samples.len() - 1)].0;
        let offset = (after - before).perpendicular().normalised() * (width / 2.0);
        left.push(point + offset);
        right.push(point - offset);
    }
    right.reverse();
    left.append(&mut right);
    left
}

/// A point along the quadratic curve running from `from` to `to`, bending around `control`.
fn quadratic(from: Point, control: Point, to: Point, t: f64) -> Point {
    let u = 1.0 - t;
    from * (u * u) + control * (2.0 * u * t) + to * (t * t)
}

#[cfg(test)]
mod tests {
    use super::{Wander, centreline};
    use crate::hex::{Axial, Layout, Point, Top};

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    fn wander(amount: f64) -> Wander {
        Wander { amount, seed: 7, crossing_tag: 1, bend_tag: 2 }
    }

    /// The two ends stop on their hexes' bend points, so two lines meeting at a hex meet exactly there
    /// however far they stray.
    #[test]
    fn lines_meeting_at_a_hex_share_its_end_point() {
        let chain = [Axial::new(0, 0), Axial::new(1, 0), Axial::new(2, 0)];
        let other = [Axial::new(2, -1), Axial::new(2, 0)];
        let (layout, _) = Layout::fit(Top::Pointy, 10.0, chain.iter().copied().chain(other));

        let wander = wander(0.4);
        let line = centreline(&chain, &layout, |_| 0.0, &wander);
        let joining = centreline(&other, &layout, |_| 0.0, &wander);

        assert!(close(*line.anchors.last().unwrap(), *joining.anchors.last().unwrap()));
        assert!(!close(*line.anchors.last().unwrap(), layout.centre(Axial::new(2, 0))), "nothing strayed");
    }

    /// Tags are what keep one kind of line off another: the same seed and the same hexes still draw
    /// different lines once the tags differ.
    #[test]
    fn tags_stray_independently() {
        let chain = [Axial::new(0, 0), Axial::new(1, 0)];
        let (layout, _) = Layout::fit(Top::Pointy, 10.0, chain);

        let line = centreline(&chain, &layout, |_| 0.0, &wander(0.4));
        let same = centreline(&chain, &layout, |_| 0.0, &wander(0.4));
        let tagged = centreline(&chain, &layout, |_| 0.0, &Wander { crossing_tag: 3, bend_tag: 4, ..wander(0.4) });

        assert!(line.anchors.iter().zip(&same.anchors).all(|(a, b)| close(*a, *b)));
        assert!(line.anchors.iter().zip(&tagged.anchors).any(|(a, b)| !close(*a, *b)));
    }
}
