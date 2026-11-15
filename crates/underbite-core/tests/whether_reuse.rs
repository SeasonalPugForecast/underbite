//! whether's core is reachable at the pinned revision with the parts Underbite reuses.

use whether_core::sun::{SunDay, sun_day};
use whether_core::time::LocalDate;

#[test]
fn sun_day_at_the_battery_on_the_june_solstice() {
    let date = LocalDate {
        year: 2026,
        month: 6,
        day: 21,
    };
    let SunDay::RiseSet { rise, set } = sun_day(date, 40.7033, -74.0170) else {
        panic!("the sun rises and sets in New York");
    };
    let length = set.0 - rise.0;
    // About 15 h 5 min of daylight at the solstice.
    assert!(
        (15 * 3600..15 * 3600 + 10 * 60).contains(&length),
        "{length} s"
    );
}
