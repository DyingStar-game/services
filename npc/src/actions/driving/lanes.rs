//! Where on the road a vehicle drives (ADR services/0012). Offsets are lateral
//! distances from the road's centreline, positive to the LEFT of the direction
//! of travel (like the steering), so traffic keeps right with a negative
//! offset.
//!
//! Lane geometry follows the game's road ribbon (`road_terrain.gd`): 3.5 m
//! lanes; a highway has its lanes split half each way with a 0.5 m median; any
//! other road is one strip, half each way, with no median.
//!
//! On such a two-way strip a vehicle does not drive in the middle of its half
//! but close to its edge: on a 6 m road a 2.19 m truck at mid-half (1.5 m off
//! the centreline) would leave only 0.8 m to a truck coming the other way. Its
//! centre keeps [`EDGE_TO_CENTRE_M`] from the edge instead (1.75 m off the
//! centreline, 1.3 m between two trucks crossing).

/// Half of a highway's median, metres.
const MEDIAN_HALF_M: f64 = 0.25;

/// On a two-way road without lanes of its own (2 or 3 lanes), from the road's
/// edge to the centre of a vehicle keeping to it.
pub const EDGE_TO_CENTRE_M: f64 = 1.25;

/// The lanes of the road at one point of a route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lanes {
    /// Half the paved width (0 off the road).
    pub half_width_m: f64,
    /// Centre of the lane to cruise in.
    pub cruise_m: f64,
    /// Centre of the lane to overtake in (= `cruise_m` when there is none).
    pub overtake_m: f64,
    /// Whether traffic comes the other way in the overtaking lane.
    pub overtake_oncoming: bool,
}

impl Lanes {
    pub fn offroad() -> Self {
        Self { half_width_m: 0.0, cruise_m: 0.0, overtake_m: 0.0, overtake_oncoming: false }
    }

    pub fn is_road(&self) -> bool {
        self.half_width_m > 0.0
    }

    /// Lateral range a vehicle centre may use on the paved road, keeping
    /// `vehicle_half_width + margin` from both edges. Off the road (or on a
    /// road too narrow), `offroad_half_m` either side of the centreline.
    pub fn centre_range(&self, vehicle_half_width: f64, margin: f64, offroad_half_m: f64) -> (f64, f64) {
        let h = self.half_width_m - vehicle_half_width - margin;
        if self.is_road() && h > 0.0 { (-h, h) } else { (-offroad_half_m, offroad_half_m) }
    }
}

/// The lanes of a road `2 × half_width_m` wide with `lanes` lanes (both
/// directions together); off-road when either is 0.
pub fn lanes(half_width_m: f64, lanes: u8) -> Lanes {
    if half_width_m <= 0.0 || lanes == 0 {
        return Lanes::offroad();
    }
    match lanes {
        // One lane for both ways: the middle, and nowhere to overtake.
        1 => Lanes { half_width_m, cruise_m: 0.0, overtake_m: 0.0, overtake_oncoming: true },
        2 | 3 => {
            // Close to the edge, but never past the middle of the half.
            let off = (half_width_m - EDGE_TO_CENTRE_M).max(half_width_m / 2.0);
            Lanes { half_width_m, cruise_m: -off, overtake_m: off, overtake_oncoming: true }
        }
        n => {
            // Divided road: n/2 lanes each way, past the median. Cruise in the
            // rightmost lane, overtake in the one to its left (same way).
            let per_side = (n / 2) as f64;
            let lane = (half_width_m - MEDIAN_HALF_M) / per_side;
            let centre = |k: f64| -(MEDIAN_HALF_M + lane * (k + 0.5));
            Lanes {
                half_width_m,
                cruise_m: centre(per_side - 1.0),
                overtake_m: centre(per_side - 2.0),
                overtake_oncoming: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_lane_road_keeps_close_to_the_right_edge_and_overtakes_into_oncoming() {
        let l = lanes(3.0, 2);
        // 6 m road: centre 1.25 m from the edge, 1.75 m right of the centreline.
        assert_eq!((l.cruise_m, l.overtake_m, l.overtake_oncoming), (-1.75, 1.75, true));
        // Two 2.19 m trucks crossing keep 1.31 m between them.
        let gap = l.overtake_m - l.cruise_m - 2.0 * 1.095;
        assert!((gap - 1.31).abs() < 1e-9, "{gap}");
        // A 2.19 m truck with 0.1 m to the edges: centre within ±1.805 m.
        let (lo, hi) = l.centre_range(1.095, 0.1, 4.0);
        assert!((lo + 1.805).abs() < 1e-9 && (hi - 1.805).abs() < 1e-9);
        assert!(hi >= -l.cruise_m, "the cruise lane is within reach");
    }

    #[test]
    fn highway_cruises_in_the_outer_lane_and_overtakes_same_way() {
        let l = lanes(7.25, 4);
        assert!((l.cruise_m + 5.5).abs() < 1e-9, "{}", l.cruise_m);
        assert!((l.overtake_m + 2.0).abs() < 1e-9, "{}", l.overtake_m);
        assert!(!l.overtake_oncoming);
    }

    #[test]
    fn off_road_is_the_line_itself() {
        let l = lanes(0.0, 0);
        assert!(!l.is_road());
        assert_eq!(l.cruise_m, 0.0);
        assert_eq!(l.centre_range(0.9, 0.3, 4.0), (-4.0, 4.0));
    }
}
