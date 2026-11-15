# Weather fixtures (copies)

Copied without network access from whether's fixtures at revision 34bb5c0 with
`git show 34bb5c0:<path>`; the bytes are unchanged. Used by the weather leg of replay tests.

- `forecast_full_10001.json` — copied from whether's fixtures (`crates/whether-providers/tests/fixtures/open_meteo/forecast_full_10001.json`), recorded 2026-11-02 (16:30 UTC) from the Open-Meteo forecast API (`https://api.open-meteo.com/v1/forecast`, no key) for ZIP 10001 (40.71,-73.99, `America/New_York`) with whether's full variable set. whether's index does not keep the query string.
- `alerts_10001.json` — copied from whether's fixtures (`crates/whether-providers/tests/fixtures/nws/alerts_10001.json`), recorded 2026-11-02 from `api.weather.gov` (no key, `User-Agent` naming the app): no active alerts at New York.
