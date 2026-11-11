//! Mission tiers (playtest 2026-11-09): a shortage is cut into missions of a few sizes, so there is work
//! for one player, for a small team, and for a big operation over several days. The tier of a mission
//! follows from its quantity alone (in the resource's own unit: kg, l, m3), which both the planner,
//! cutting the shortage, and the publisher, setting the mission's seats and deadline, read.
//!
//! The limits are reckoned in truckloads of the resource (truckload): a truck carries TRUCK_CRATES
//! crates, whatever they hold, so a truckload of ore weighs what that many crates of ore weigh.

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

/// Crates a truck's bed holds (the game's mini truck: one layer of 4 x 4 ore crates, playtest 2026-11-10).
/// Not its rated payload (1 300 kg): the bed is full by volume long before.
pub const TRUCK_CRATES: f64 = 16.0;
/// A solo mission: up to one truckload...
pub const SOLO_TRUCKS: i64 = 1;
/// ...a team one: from two truckloads to four...
pub const TEAM_MIN_TRUCKS: i64 = 2;
pub const TEAM_MAX_TRUCKS: i64 = 4;
/// ...an operation: from ten to twenty.
pub const OPERATION_MIN_TRUCKS: i64 = 10;
pub const OPERATION_MAX_TRUCKS: i64 = 20;

/// A shortage is cut into at most this many missions; what is left goes into the last one (a resource
/// whose truckload is a unit or two would give hundreds).
pub const MAX_PIECES: usize = 24;

/// A truckload of a resource, in its own unit: TRUCK_CRATES crates of [crate_kg] each, at
/// [mass_per_unit_kg] a unit. At least 1.
pub fn truckload(crate_kg: f64, mass_per_unit_kg: f64) -> i64 {
    let unit = if mass_per_unit_kg > 0.0 { mass_per_unit_kg } else { 1.0 };
    ((TRUCK_CRATES * crate_kg / unit).round() as i64).max(1)
}

/// Hours to do one piece of solo or team work, by kind (a team shares the work, so it gets the same
/// time as one player -- ddurieux, 2026-11-09): carry goods between two warehouses...
pub const TRANSPORT_HOURS: i32 = 2;
/// ...have the ore of a warehouse refined in its own village...
pub const REFINING_HOURS: i32 = 2;
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
    /// The tier of a mission of [quantity], for a resource whose truckload is [truck].
    pub fn of(quantity: i64, truck: i64) -> Tier {
        if quantity >= OPERATION_MIN_TRUCKS * truck {
            Tier::Operation
        } else if quantity >= TEAM_MIN_TRUCKS * truck {
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

/// [quantity] cut into the quantities of its missions, in truckloads of [truck]. First a solo mission, and a team one when what is
/// left holds it: every shortage has work for a lone player and, big enough, for a team. Then operations
/// while an operation's worth is left, team missions, and solo ones for the rest. Their sum is [quantity].
pub fn chunks(quantity: i64, truck: i64) -> Vec<i64> {
    let truck = truck.max(1);
    let (solo_max, team_min, team_max) = (SOLO_TRUCKS * truck, TEAM_MIN_TRUCKS * truck, TEAM_MAX_TRUCKS * truck);
    let (operation_min, operation_max) = (OPERATION_MIN_TRUCKS * truck, OPERATION_MAX_TRUCKS * truck);
    let mut out = Vec::new();
    let mut left = quantity.max(0);
    if left > 0 {
        let solo = left.min(solo_max);
        out.push(solo);
        left -= solo;
    }
    if left >= team_min {
        let team = left.min(team_max);
        out.push(team);
        left -= team;
    }
    while left >= operation_min {
        let c = left.min(operation_max);
        out.push(c);
        left -= c;
    }
    while left >= team_min {
        let c = left.min(team_max);
        out.push(c);
        left -= c;
    }
    while left > 0 {
        let c = left.min(solo_max);
        out.push(c);
        left -= c;
    }
    if out.len() > MAX_PIECES {
        let rest: i64 = out.drain(MAX_PIECES..).sum();
        out[MAX_PIECES - 1] += rest;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shortage_is_cut_into_a_solo_a_team_then_the_biggest_first() {
        // A resource whose crate holds 22 kg: 16 crates, a truckload of 352 kg.
        let truck = truckload(22.0, 1.0);
        assert_eq!(truck, 352);
        assert_eq!(chunks(12_000, truck), vec![352, 1_408, 7_040, 1_408, 1_408, 352, 32], "a solo, a team, an operation, then teams and solos");
        assert_eq!(chunks(4_183, truck), vec![352, 1_408, 1_408, 1_015], "a solo and teams");
        assert_eq!(chunks(500, truck), vec![352, 148], "under a solo and a team: solo missions");
        assert_eq!(chunks(250, truck), vec![250]);
        assert!(chunks(0, truck).is_empty());
    }

    #[test]
    fn a_tiny_truckload_does_not_give_hundreds_of_missions() {
        let pieces = chunks(10_000, 1);
        assert_eq!(pieces.len(), MAX_PIECES);
        assert_eq!(pieces.iter().sum::<i64>(), 10_000, "nothing is lost: the rest is in the last one");
    }

    #[test]
    fn the_cut_adds_up_to_the_shortage() {
        for q in [1, 199, 601, 4_999, 5_000, 23_456] {
            assert_eq!(chunks(q, 352).iter().sum::<i64>(), q);
        }
    }

    #[test]
    fn each_piece_is_of_its_tier() {
        assert_eq!(Tier::of(352, 352), Tier::Solo);
        assert_eq!(Tier::of(1_000, 352), Tier::Team);
        assert_eq!(Tier::of(4_000, 352), Tier::Operation);
        assert_eq!(Tier::of(1_000, 352).max_assignees(), 3);
        assert_eq!(Tier::of(4_000, 352).deadline_hours(Pace::Relaxed, 2), OPERATION_HOURS);
        assert_eq!(Tier::of(4_000, 352).deadline_hours(Pace::Express, 2), 2, "an express operation too");
        assert_eq!(truckload(15.0, 1.0), 240, "ingots: lighter crates, a lighter truckload");
        assert_eq!(truckload(0.01, 1.0), 1, "never nothing");
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
