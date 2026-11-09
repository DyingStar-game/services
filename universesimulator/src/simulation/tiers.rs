//! Mission tiers (playtest 2026-11-09): a shortage is cut into missions of a few sizes, so there is work
//! for one player, for a small team, and for a big operation over several days. The tier of a mission
//! follows from its quantity alone (in the resource's own unit: kg, l, m3), which both the planner,
//! cutting the shortage, and the publisher, setting the mission's seats and deadline, read.
//!
//! The limits are reckoned in weight, in truckloads (TRUCK_KG), the same number in every unit for now
//! (only gold, in kg, is simulated).

/// The size of a mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// One player, up to a truckload.
    Solo,
    /// Two or three players, a few truckloads.
    Team,
    /// Up to ten players, over several days.
    Operation,
}

/// What a truck may carry (its payload, kg): the unit of the tiers. A solo mission is one truckload.
pub const TRUCK_KG: i64 = 1_300;
/// Largest quantity of a solo mission: one truckload.
pub const SOLO_MAX: i64 = TRUCK_KG;
/// A team mission: from two truckloads...
pub const TEAM_MIN: i64 = 2 * TRUCK_KG;
/// ...up to four.
pub const TEAM_MAX: i64 = 4 * TRUCK_KG;
/// An operation: from ten truckloads...
pub const OPERATION_MIN: i64 = 10 * TRUCK_KG;
/// ...up to twenty.
pub const OPERATION_MAX: i64 = 20 * TRUCK_KG;

/// Hours to do one piece of solo or team work, by kind (a team shares the work, so it gets the same
/// time as one player -- ddurieux, 2026-11-09): carry goods between two warehouses...
pub const TRANSPORT_HOURS: i32 = 2;
/// ...carry ore and have it refined on the way...
pub const REFINING_TRANSPORT_HOURS: i32 = 3;
/// ...mine and shelve it...
pub const MINING_HOURS: i32 = 4;
/// ...mine, refine and shelve the metal.
pub const MINING_REFINING_HOURS: i32 = 6;
/// Hours of a team mission at the relaxed pace, and the share of the reward a relaxed mission pays.
pub const RELAXED_HOURS: i32 = 24;
pub const RELAXED_REWARD: f64 = 0.7;
/// Hours of an operation at the relaxed pace: ten players are not all on at once.
pub const OPERATION_HOURS: i32 = 168;

/// How fast a team mission or an operation is to be done: in the time of the work, for the whole
/// reward, or within a day (a week for an operation) for less (playtest 2026-11-09). Solo missions are
/// always express.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    Express,
    Relaxed,
}

/// The pace of a mission of [tier]: team missions and operations one in two at each (by [seed], the
/// leg's id: the same mission always gets the same), solo ones express.
pub fn pace_of(tier: Tier, seed: u128) -> Pace {
    match tier {
        Tier::Solo => Pace::Express,
        Tier::Team | Tier::Operation => {
            if seed.is_multiple_of(2) { Pace::Express } else { Pace::Relaxed }
        }
    }
}

/// Hours one piece of [kind] work takes (`transport`, `mining`, `mining_transport`), [refined] when the
/// goods are refined on the way.
pub fn work_hours(kind: &str, refined: bool) -> i32 {
    match kind {
        "transport" if refined => REFINING_TRANSPORT_HOURS,
        "transport" => TRANSPORT_HOURS,
        "mining_transport" => MINING_REFINING_HOURS,
        _ => MINING_HOURS,
    }
}

impl Tier {
    /// The tier of a mission of [quantity].
    pub fn of(quantity: i64) -> Tier {
        if quantity >= OPERATION_MIN {
            Tier::Operation
        } else if quantity >= TEAM_MIN {
            Tier::Team
        } else {
            Tier::Solo
        }
    }

    /// How many players may take it together (they share its progress).
    pub fn max_assignees(self) -> i32 {
        match self {
            Tier::Solo => 1,
            Tier::Team => 3,
            Tier::Operation => 10,
        }
    }

    /// Hours an assignment has to finish it, from its acceptance: the time of its [work_hours] at the
    /// express pace (the players share the work), else a day, a week for an operation.
    pub fn deadline_hours(self, pace: Pace, work_hours: i32) -> i32 {
        match (self, pace) {
            (_, Pace::Express) => work_hours,
            (Tier::Operation, Pace::Relaxed) => OPERATION_HOURS,
            (_, Pace::Relaxed) => RELAXED_HOURS,
        }
    }

    /// The share of its reward a mission pays at [pace]: less when relaxed.
    pub fn reward_share(self, pace: Pace) -> f64 {
        if pace == Pace::Relaxed { RELAXED_REWARD } else { 1.0 }
    }

    /// The tier's name in English and French, for the mission's title.
    pub fn label(self, pace: Pace) -> (&'static str, &'static str) {
        match (self, pace) {
            (Tier::Solo, _) => ("Solo", "Solo"),
            (Tier::Team, Pace::Express) => ("Express team", "Équipe express"),
            (Tier::Team, Pace::Relaxed) => ("Team", "Équipe"),
            (Tier::Operation, Pace::Express) => ("Express operation", "Opération express"),
            (Tier::Operation, Pace::Relaxed) => ("Operation", "Opération"),
        }
    }
}

/// [quantity] cut into the quantities of its missions: operations while an operation's worth is left,
/// then team missions, then solo ones for the rest. Their sum is [quantity].
pub fn chunks(quantity: i64) -> Vec<i64> {
    let mut out = Vec::new();
    let mut left = quantity.max(0);
    while left >= OPERATION_MIN {
        let c = left.min(OPERATION_MAX);
        out.push(c);
        left -= c;
    }
    while left >= TEAM_MIN {
        let c = left.min(TEAM_MAX);
        out.push(c);
        left -= c;
    }
    while left > 0 {
        let c = left.min(SOLO_MAX);
        out.push(c);
        left -= c;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortage_is_cut_from_the_biggest_missions_down() {
        assert_eq!(chunks(30_000), vec![26_000, 4_000], "an operation, then a team for the rest");
        assert_eq!(chunks(6_000), vec![5_200, 800]);
        assert_eq!(chunks(2_100), vec![1_300, 800], "under two trucks: solo missions");
        assert_eq!(chunks(788), vec![788]);
        assert!(chunks(0).is_empty());
    }

    #[test]
    fn the_cut_adds_up_to_the_shortage() {
        for q in [1, 199, 601, 4_999, 5_000, 23_456] {
            assert_eq!(chunks(q).iter().sum::<i64>(), q);
        }
    }

    #[test]
    fn each_piece_is_of_its_tier() {
        assert_eq!(Tier::of(1_300), Tier::Solo);
        assert_eq!(Tier::of(3_000), Tier::Team);
        assert_eq!(Tier::of(13_000), Tier::Operation);
        assert_eq!(Tier::of(3_000).max_assignees(), 3);
        assert_eq!(Tier::of(13_000).deadline_hours(Pace::Relaxed, 2), OPERATION_HOURS);
        assert_eq!(Tier::of(13_000).deadline_hours(Pace::Express, 2), 2, "an express operation too");
    }

    #[test]
    fn solo_and_express_team_get_the_time_of_the_work() {
        let mining = work_hours("mining", false);
        assert_eq!(Tier::Solo.deadline_hours(Pace::Express, mining), MINING_HOURS);
        assert_eq!(Tier::Team.deadline_hours(Pace::Express, mining), MINING_HOURS, "shared work, same time");
        assert_eq!(Tier::Team.deadline_hours(Pace::Relaxed, mining), RELAXED_HOURS);
        assert_eq!(work_hours("transport", true), REFINING_TRANSPORT_HOURS);
        assert_eq!(work_hours("mining_transport", true), MINING_REFINING_HOURS);
    }

    #[test]
    fn a_relaxed_team_pays_less_and_teams_split_between_the_paces() {
        assert_eq!(Tier::Team.reward_share(Pace::Relaxed), RELAXED_REWARD);
        assert_eq!(Tier::Team.reward_share(Pace::Express), 1.0);
        assert_eq!(pace_of(Tier::Team, 4), Pace::Express);
        assert_eq!(pace_of(Tier::Team, 5), Pace::Relaxed);
        assert_eq!(pace_of(Tier::Solo, 5), Pace::Express);
        assert_eq!(pace_of(Tier::Operation, 4), Pace::Express);
        assert_eq!(Tier::Operation.reward_share(Pace::Relaxed), RELAXED_REWARD);
    }
}
