//! The spot's time zone, and sun, moon and solunar periods for its local days.

use jiff::tz::TimeZone;
use underbite_core::astro::solunar::{DayBounds, SolunarSettings, sky_conditions};
use underbite_core::model::{SkyConditions, Spot};
use underbite_core::time::{UnixTime, local_date, local_date_bounds, zone};
use whether_core::model::WeatherSnapshot;

/// The spot's time zone: the one stored with the spot (configured, or given with `--tz`), else
/// the zone the forecast resolved for the place. `None` when neither names a known zone.
pub(crate) fn time_zone(
    spot: &Spot,
    weather: Option<&WeatherSnapshot>,
) -> Option<(String, TimeZone)> {
    let stored = Some(spot.time_zone.as_str()).filter(|name| !name.is_empty());
    let forecast = weather.and_then(|w| w.location.tz_name.as_deref());
    [stored, forecast]
        .into_iter()
        .flatten()
        .find_map(|name| zone(name).ok().map(|tz| (name.to_string(), tz)))
}

/// Sun, moon and solunar periods for `days` local days from today.
pub(crate) fn sky(
    spot: &Spot,
    tz: &TimeZone,
    now: UnixTime,
    days: u8,
    settings: SolunarSettings,
) -> SkyConditions {
    let mut bounds = Vec::new();
    let mut date = local_date(now, tz);
    for _ in 0..days {
        let Ok(day) = date else { break };
        let Ok((start, end)) = local_date_bounds(day, tz) else {
            break;
        };
        bounds.push(DayBounds {
            date: day,
            start,
            end,
        });
        date = local_date(end, tz);
    }
    sky_conditions(spot.latitude, spot.longitude, &bounds, settings)
}
