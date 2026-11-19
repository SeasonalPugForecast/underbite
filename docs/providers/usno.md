# USNO Astronomical Applications API

Verified on 2026-11-15 with `curl` against `https://aa.usno.navy.mil/api`. Fixtures are in
`tests/fixtures/raw/usno/` (index: `tests/fixtures/raw/usno/README.md`). The data serve the astronomy
tests only: they are the reference for the moon rise, set, transit and phase code, not a runtime source.

## Base, version and authentication

- Documentation: `https://aa.usno.navy.mil/data/api` ("Astronomical Applications API v4.0.1"); change log:
  `https://aa.usno.navy.mil/data/api_changelog` (v4.0.1 of 2022-09-30 is the latest entry; v4.0.0 of
  2021-07-08 changed dates to `yyyy-mm-dd`). Every response carries `"apiversion": "4.0.1"`.
- Authentication: none. The documentation describes an optional `ID` parameter (up to 8 alphanumeric
  characters, `AA` reserved) that lets the observatory count distinct users; it was not used.
- Headers: no `User-Agent` is required. Responses are `application/json` with
  `access-control-allow-origin: *` and `cache-control: no-cache`; no rate-limit headers.
- Rate limit: none published on the documentation page or the privacy notice. The 32 requests recorded
  were spaced 1.5 s apart and none was refused.
- Date range: years 1700 to 2100 inclusive for dates and for `moon/phases/year`.

## Terms of use

The API page states no terms, licence or limits. The observatory's privacy notice
(`https://aa.usno.navy.mil/about/PrivacyNotice`) says "All information on this site is considered public
information" that may be distributed or copied unless otherwise specified, and asks for "appropriate
byline/photo/image credits". The data are a US Government work. A credit such as "Moon data: U.S. Naval
Observatory" satisfies that request. No bulk-use limit is stated, so these files are recorded once and
the app does not call the service.

Re-checked 2026-11-18: unchanged. The privacy notice (`https://aa.usno.navy.mil/about/PrivacyNotice`) still says "All information on this site is considered public information and may be distributed or copied unless otherwise specified. Use of appropriate byline/photo/image credits is requested." The API page itself was not read again.

## Requests

One day of Sun and Moon data at The Battery, in UTC (fixture `usno-rstt-40.7033_-74.0170-2026-11-08.json`):

```text
https://aa.usno.navy.mil/api/rstt/oneday?date=2026-11-08&coords=40.7033,-74.0170&tz=0
```

- `date` `YYYY-MM-DD` (required); `coords` `latitude,longitude` in decimal degrees, north and east
  positive (required); `tz` hours east of UTC, integer or fraction, -12 to +14. With `tz=0` all times are
  UTC. `dst` (boolean, default false) is documented; with `tz=0` the response has `"isdst": false`.
  A `height` is accepted in the GeoJSON but documented as zero when omitted.
- The antipodal longitude of -74.0170 is 105.9830 (`coords=40.7033,105.9830`). The antipode's upper
  transit is the Battery's lower transit, because a body is on the lower meridian when it is on the upper
  meridian 180 degrees away. Check against the same day at latitude 0 (`coords=0.0,-74.0170`,
  not recorded): the moon upper transit was 16:05 at both latitudes on 2026-11-08, so the transit time
  depends on longitude only to the minute, as expected for a geocentric computation.
- Moon phases of a year (fixtures `usno-moon-phases-year-2026.json`, `...-2027.json`):

```text
https://aa.usno.navy.mil/api/moon/phases/year?year=2026
```

A related request, `moon/phases/date?date=YYYY-MM-DD&nump=N` (N from 1 to 99), is documented and was not
used.

## Rise and set definitions

The service documentation does not define them. USNO's page "Rise, Set, and Twilight Definitions"
(`https://aa.usno.navy.mil/faq/RST_defs`) states: sunrise and sunset are when the upper edge of the disc
is on the horizon, computed with the centre at a zenith distance of 90.8333 degrees (16 arcminutes
semidiameter plus 34 arcminutes refraction); for the Moon the same 34 arcminute refraction constant is used;
transit is the instant the centre crosses the meridian; rise and set use coordinates "with respect to the
Earth's center (the usual method)". The page does not use the word "topocentric". Whether the
`rstt/oneday` service applies exactly these rules was not verified separately: **unverified** beyond the
page and the observed values.

## Response fields (GeoJSON `Feature`)

| Path | Example | Notes |
|---|---|---|
| `apiversion` | `"4.0.1"` | |
| `geometry.coordinates` | `[-74.017, 40.7033]` | longitude first |
| `properties.data.year`, `month`, `day` | `2026`, `11`, `8` | date requested |
| `properties.data.day_of_week` | `"Sunday"` | |
| `properties.data.tz` | `0.0` | hours east of UTC |
| `properties.data.isdst` | `false` | |
| `properties.data.label` | `null` | |
| `properties.data.curphase` | `"Waning Crescent"` | phase name for the day; the day of the new moon reads `"New Moon"`, the day of first quarter `"First Quarter"` |
| `properties.data.fracillum` | `"1%"` | string with a percent sign, whole percent |
| `properties.data.closestphase` | `{year, month, day, phase, time}` | principal phase nearest to the date; `time` is `HH:MM` UT. On 2026-11-08 it is the New Moon of 11-09 07:02; on 2026-11-13 the First Quarter of 11-17 11:48 |
| `properties.data.moondata[]` | `{"phen": "Rise", "time": "11:02"}` | `phen` is `Rise`, `Upper Transit` or `Set`; `time` is `HH:MM` in the requested zone |
| `properties.data.sundata[]` | `Begin Civil Twilight`, `Rise`, `Upper Transit`, `Set`, `End Civil Twilight` | same shape |

- A phenomenon that does not occur on that date in the requested zone is absent from the array, not
  `null` (the documentation speaks of `null` for bodies that stay above or below the limit; none was
  seen for the Moon here). Battery 2026-11-13 has no `Set`; 2026-11-18 has no `Upper Transit` (a gap
  because the moon crossed the meridian just after 00:00 UTC the next day); antipode 2026-11-09 has no
  `Rise`.
- Times are rounded to the minute, with no seconds and no date. Events are listed in clock order within the
  requested day, so a `Set` can precede the `Rise` (it belongs to the moonset that follows the previous
  UTC day's rise).
- No lower transit is listed; use the antipodal longitude.
- Moon phases: `{"apiversion", "numphases": 50, "phasedata": [{"day", "month", "phase", "time", "year"}]}`;
  `phase` is `New Moon`, `First Quarter`, `Full Moon` or `Last Quarter`; `time` is `HH:MM` UT. 2026
  returned 50 phases (first: Full Moon 2026-01-03 10:03, last: Last Quarter 2026-12-30 18:59); 2027
  returned 49.

## Recorded values (UTC, The Battery)

| Date | Rise | Upper transit | Set | Illuminated | Phase name |
|---|---|---|---|---|---|
| 2026-11-08 | 11:02 | 16:05 | 21:01 | 1% | Waning Crescent |
| 2026-11-09 | 12:07 | 16:53 | 21:33 | 0% | New Moon (07:02) |
| 2026-11-13 | 15:51 | 20:17 | none | 16% | Waxing Crescent |
| 2026-11-17 | 17:52 | 23:19 | 03:53 | 50% | First Quarter (11:48) |
| 2026-11-18 | 18:14 | none | 04:56 | 60% | Waxing Gibbous |
| 2026-11-22 | 19:53 | 02:19 | 09:29 | 94% | Waxing Gibbous |

Note that a UTC day listing `Set` earlier than `Rise` (for example 2026-11-14: set 00:46, rise 16:29) is
normal. The antipodal files give the Battery's lower transits: 2026-11-08 03:42 UTC, 2026-11-09 04:29.

## Not verified

- The exact rise/set rules inside `rstt/oneday` (see above).
- Behaviour for polar latitudes and for `tz` with a fractional part.
- Whether `closestphase` is the nearest phase or always the next one (all recorded days are consistent
  with the nearest).
