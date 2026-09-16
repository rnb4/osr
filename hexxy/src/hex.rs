//! Hex coordinates and where they land on the page.

use std::{
    array,
    cmp::{max, min},
    collections::HashSet,
    fmt::{self, Display},
    ops::{Add, Mul, Sub},
};

use clap::ValueEnum;

const SQRT_3: f64 = 1.732_050_807_568_877_2;

/// Sub-hexes across one atlas hex. The atlas hex at axial `a` is centred on the sub-hex at `a * ATLAS_SCALE`.
pub const ATLAS_SCALE: i32 = 5;

#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Top {
    /// Vertices at top and bottom; hexes form rows
    #[default]
    Pointy,
    /// Vertices at left and right; hexes form columns
    Flat,
}

impl Display for Top {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Top::Pointy => "pointy",
            Top::Flat => "flat",
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The point halfway to `other`.
    pub fn midpoint(self, other: Point) -> Point {
        Point::new((self.x + other.x) / 2.0, (self.y + other.y) / 2.0)
    }

    /// This point turned a quarter turn, for stepping sideways off a direction.
    pub fn perpendicular(self) -> Point {
        Point::new(-self.y, self.x)
    }

    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// This point scaled to unit length, or the origin if it has no length to scale.
    pub fn normalised(self) -> Point {
        match self.length() {
            0.0 => Point::default(),
            length => self * (1.0 / length),
        }
    }
}

impl Add for Point {
    type Output = Point;

    fn add(self, other: Point) -> Point {
        Point::new(self.x + other.x, self.y + other.y)
    }
}

impl Sub for Point {
    type Output = Point;

    fn sub(self, other: Point) -> Point {
        Point::new(self.x - other.x, self.y - other.y)
    }
}

impl Mul<f64> for Point {
    type Output = Point;

    fn mul(self, factor: f64) -> Point {
        Point::new(self.x * factor, self.y * factor)
    }
}

/// Axial hex coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Axial {
    pub q: i32,
    pub r: i32,
}

impl Axial {
    /// Steps to the six neighbouring hexes.
    pub const NEIGHBOURS: [Axial; 6] = [
        Axial::new(1, 0),
        Axial::new(1, -1),
        Axial::new(0, -1),
        Axial::new(-1, 0),
        Axial::new(-1, 1),
        Axial::new(0, 1),
    ];

    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    /// Steps between this hex and `other`.
    pub fn distance(self, other: Axial) -> i32 {
        let (dq, dr) = (self.q - other.q, self.r - other.r);
        (dq.abs() + dr.abs() + (dq + dr).abs()) / 2
    }

    /// Every hex within `radius` steps of this one.
    pub fn disc(self, radius: i32) -> impl Iterator<Item = Axial> {
        (-radius..=radius).flat_map(move |dq| {
            (max(-radius, -dq - radius)..=min(radius, radius - dq)).map(move |dr| self + Axial::new(dq, dr))
        })
    }

    /// Every hex exactly `radius` steps from this one.
    pub fn ring(self, radius: i32) -> impl Iterator<Item = Axial> {
        self.disc(radius).filter(move |&hex| hex.distance(self) == radius)
    }

    /// A number identifying this hex, for seeding randomness.
    pub fn key(self) -> u64 {
        (u64::from(self.q as u32) << 32) | u64::from(self.r as u32)
    }
}

impl Add for Axial {
    type Output = Axial;

    fn add(self, other: Axial) -> Axial {
        Axial::new(self.q + other.q, self.r + other.r)
    }
}

impl Mul<i32> for Axial {
    type Output = Axial;

    fn mul(self, factor: i32) -> Axial {
        Axial::new(self.q * factor, self.r * factor)
    }
}

/// Pixel positions for hexes of one size and orientation.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    top: Top,
    radius: f64,
    origin: Point,
}

impl Layout {
    /// A layout of hexes `radius` from centre to vertex, shifted so the bounding box of `hexes` starts at the origin.
    /// Returns the layout and the size of that box.
    pub fn fit(top: Top, radius: f64, hexes: impl IntoIterator<Item = Axial>) -> (Layout, Point) {
        let unshifted = Layout { top, radius, origin: Point::default() };
        let mut low = Point::new(f64::INFINITY, f64::INFINITY);
        let mut high = Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY);
        for corner in hexes.into_iter().flat_map(|hex| unshifted.corners(unshifted.centre(hex), 1.0)) {
            low = Point::new(low.x.min(corner.x), low.y.min(corner.y));
            high = Point::new(high.x.max(corner.x), high.y.max(corner.y));
        }
        (Layout { origin: low, ..unshifted }, high - low)
    }

    /// Centre to vertex.
    pub fn radius(&self) -> f64 {
        self.radius
    }

    /// Flat side to flat side for pointy-top hexes, vertex to vertex for flat-top ones.
    pub fn hex_width(&self) -> f64 {
        match self.top {
            Top::Pointy => SQRT_3 * self.radius,
            Top::Flat => 2.0 * self.radius,
        }
    }

    pub fn centre(&self, hex: Axial) -> Point {
        let (q, r) = (f64::from(hex.q), f64::from(hex.r));
        let unit = match self.top {
            Top::Pointy => Point::new(SQRT_3 * (q + r / 2.0), 1.5 * r),
            Top::Flat => Point::new(1.5 * q, SQRT_3 * (r + q / 2.0)),
        };
        Point::new(unit.x * self.radius, unit.y * self.radius) - self.origin
    }

    /// Corners of a hex `scale` grid hexes across, centred on `centre`. Edge i runs from corner i to corner i + 1.
    pub fn corners(&self, centre: Point, scale: f64) -> [Point; 6] {
        let start = match self.top {
            Top::Pointy => 30.0,
            Top::Flat => 0.0,
        };
        let size = scale * self.radius;
        array::from_fn(|i| {
            let angle = (start + 60.0 * i as f64).to_radians();
            centre + Point::new(size * angle.cos(), size * angle.sin())
        })
    }

    /// Every edge of a set of tiling hexes exactly once.
    ///
    /// `cells` are positions on a hex lattice, `centre` places each one, and the hexes are `scale` grid hexes across.
    /// Neighbouring cells share an edge, and stroking every outline would draw it twice, showing up as darker lines
    /// when the stroke is semi-transparent.
    pub fn edges(
        &self,
        cells: impl IntoIterator<Item = Axial>,
        scale: f64,
        centre: impl Fn(Axial) -> Point,
    ) -> Vec<(Point, Point)> {
        let directions = self.directions();
        let mut seen = HashSet::new();
        let mut edges = Vec::new();
        for cell in cells {
            let corners = self.corners(centre(cell), scale);
            for (i, &direction) in directions.iter().enumerate() {
                // The neighbour sees the same edge facing the opposite way.
                if seen.insert(min((cell, i), (cell + direction, (i + 3) % 6))) {
                    edges.push((corners[i], corners[(i + 1) % 6]));
                }
            }
        }
        edges
    }

    /// The step to the neighbour across each edge.
    fn directions(&self) -> [Axial; 6] {
        let steps = match self.top {
            Top::Pointy => [(0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1), (1, 0)],
            Top::Flat => [(1, 0), (0, 1), (-1, 1), (-1, 0), (0, -1), (1, -1)],
        };
        steps.map(|(q, r)| Axial::new(q, r))
    }
}

#[cfg(test)]
mod tests {
    use super::{Axial, Layout, Top};

    fn close(a: super::Point, b: super::Point) -> bool {
        (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9
    }

    #[test]
    fn directions_point_across_their_edges() {
        for top in [Top::Pointy, Top::Flat] {
            let (layout, _) = Layout::fit(top, 10.0, [Axial::default()]);
            let corners = layout.corners(layout.centre(Axial::default()), 1.0);
            for (i, step) in layout.directions().into_iter().enumerate() {
                let theirs = layout.corners(layout.centre(step), 1.0);
                let j = (i + 3) % 6;
                assert!(close(corners[i], theirs[(j + 1) % 6]) && close(corners[(i + 1) % 6], theirs[j]), "{top} {i}");
            }
        }
    }

    /// Anything running from hex to hex crosses their shared edge at its midpoint, and that midpoint is just the point
    /// halfway between the two centres, so a course can be laid out without ever naming an edge.
    #[test]
    fn neighbour_centres_meet_at_their_shared_edge() {
        for top in [Top::Pointy, Top::Flat] {
            let (layout, _) = Layout::fit(top, 10.0, [Axial::default()]);
            let centre = Axial::default();
            let corners = layout.corners(layout.centre(centre), 1.0);
            for (i, step) in layout.directions().into_iter().enumerate() {
                let edge = corners[i].midpoint(corners[(i + 1) % 6]);
                assert!(close(edge, layout.centre(centre).midpoint(layout.centre(centre + step))), "{top} {i}");
            }
        }
    }

    #[test]
    fn shared_edges_are_drawn_once() {
        let (layout, _) = Layout::fit(Top::Pointy, 10.0, [Axial::default()]);
        let flower: Vec<Axial> = Axial::default().disc(1).collect();
        assert_eq!(flower.len(), 7);
        assert_eq!(layout.edges(flower, 1.0, |cell| layout.centre(cell)).len(), 30);
    }

    #[test]
    fn discs_and_rings_count_their_hexes() {
        let centre = Axial::new(5, -5);
        assert_eq!(centre.disc(2).count(), 19);
        assert_eq!(centre.ring(3).count(), 18);
        assert!(centre.ring(3).all(|hex| hex.distance(centre) == 3));
        assert!(Axial::NEIGHBOURS.iter().all(|&step| (centre + step).distance(centre) == 1));
    }

    #[test]
    fn fit_starts_at_origin() {
        let (layout, size) = Layout::fit(Top::Pointy, 10.0, [Axial::new(0, 0), Axial::new(1, 0)]);
        let corners = layout.corners(layout.centre(Axial::new(0, 0)), 1.0);
        assert!(corners.iter().all(|c| c.x >= -1e-9 && c.y >= -1e-9));
        assert!((size.x - 2.0 * layout.hex_width()).abs() < 1e-9 && (size.y - 20.0).abs() < 1e-9, "{size:?}");
    }
}
