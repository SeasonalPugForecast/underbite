# USNO Astronomical Applications API fixtures

Recorded 2026-11-15 with `curl`, no key, no custom headers. Files keep the source's JSON bytes; none was
edited. Base URL: `https://aa.usno.navy.mil/api` (API version 4.0.1). See `docs/providers/usno.md`.
These data serve the astronomy tests only. All files are `tz=0` (UTC times, rounded to the minute).
Longitude 105.9830 is the antipode of -74.0170: its upper transits are The Battery's lower transits.

- `usno-rstt-40.7033_-74.0170-<date>.json` (15 files, 2026-11-08 to 2026-11-22) — `https://aa.usno.navy.mil/api/rstt/oneday?date=<date>&coords=40.7033,-74.0170&tz=0` (2026-11-15). 2026-11-13 has no moonset and 2026-11-18 has no moon upper transit: those events are absent from `moondata`.
- `usno-rstt-40.7033_105.9830-<date>.json` (15 files, 2026-11-08 to 2026-11-22) — `https://aa.usno.navy.mil/api/rstt/oneday?date=<date>&coords=40.7033,105.9830&tz=0` (2026-11-15). Antipodal longitude; 2026-11-09 has no moonrise.
- `usno-moon-phases-year-2026.json` — `https://aa.usno.navy.mil/api/moon/phases/year?year=2026` (2026-11-15; 50 phases, UT)
- `usno-moon-phases-year-2027.json` — `https://aa.usno.navy.mil/api/moon/phases/year?year=2027` (2026-11-15)
