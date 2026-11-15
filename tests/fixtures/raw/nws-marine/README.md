# NWS marine: recorded responses

All files are response bodies exactly as served by `https://api.weather.gov`, recorded on 2026-11-08 with
`User-Agent: underbite-dev (contact via repository)`. `A` is `https://api.weather.gov`. Files named `*.geo.json`
were requested with `Accept: application/geo+json`; `*.ld.json` with `Accept: application/ld+json`;
`*.problem.json` are `application/problem+json` error bodies (HTTP status in the row).
Alert, zone-forecast and product contents are live and change; the recording is a snapshot.

| File | URL | Date |
|---|---|---|
| `points-40.7033_-74.0170.geo.json` | `A/points/40.7033,-74.0170` (The Battery; `type` marine, `forecastZone` ANZ338) | 2026-11-08 |
| `points-39.5_-72.0-offshore.geo.json` | `A/points/39.5,-72.0` (offshore zone ANZ815, no `gridId`) | 2026-11-08 |
| `points-45.0_-30.0-invalid-point.problem.json` | `A/points/45.0,-30.0` (HTTP 404, mid-Atlantic, outside coverage) | 2026-11-08 |
| `points-41.55_-81.72.geo.json` | `A/points/41.55,-81.72` (Lake Erie off Cleveland; `forecastZone` LEZ146) | 2026-11-08 |
| `zones-coastal-point-40.7033_-74.0170.geo.json` | `A/zones?type=coastal&point=40.7033,-74.0170` (ANZ338 New York Harbor) | 2026-11-08 |
| `zones-coastal-point-40.25_-73.16.geo.json` | `A/zones?type=coastal&point=40.25,-73.16` (ANZ383, 20 to 60 NM) | 2026-11-08 |
| `zones-offshore-point-40.25_-73.16-empty.geo.json` | `A/zones?type=offshore&point=40.25,-73.16` (empty `features`) | 2026-11-08 |
| `zones-point-39.5_-72.0-offshore.geo.json` | `A/zones?point=39.5,-72.0` (ANZ815, type offshore) | 2026-11-08 |
| `zones-offshore-area-AN.geo.json` | `A/zones?type=offshore&area=AN` (18 Atlantic offshore zones) | 2026-11-08 |
| `zones-point-41.55_-81.72.geo.json` | `A/zones?point=41.55,-81.72` (LEZ146 Avon Point to Willowick OH, nearshore) | 2026-11-08 |
| `zones-point-41.9_-81.6-beyond-5nm.geo.json` | `A/zones?type=coastal&point=41.9,-81.6` (LEZ166, beyond 5 NM) | 2026-11-08 |
| `zones-coastal-area-LE.geo.json` | `A/zones?type=coastal&area=LE` (21 Lake Erie zones) | 2026-11-08 |
| `zone-coastal-ANZ338.ld.json` | `A/zones/coastal/ANZ338` (zone metadata with WKT polygon) | 2026-11-08 |
| `zone-coastal-LEZ146.ld.json` | `A/zones/coastal/LEZ146` | 2026-11-08 |
| `zone-forecast-ANZ338.ld.json` | `A/zones/forecast/ANZ338/forecast` (10 text periods) | 2026-11-08 |
| `zone-forecast-ANZ383.ld.json` | `A/zones/coastal/ANZ383/forecast` (10 text periods, "Seas" wording) | 2026-11-08 |
| `zone-forecast-LEZ146-not-found.problem.json` | `A/zones/forecast/LEZ146/forecast` (HTTP 404: no zone forecast for Great Lakes zones) | 2026-11-08 |
| `zone-forecast-ANZ820-offshore-unsupported.problem.json` | `A/zones/offshore/ANZ820/forecast` (HTTP 404, "Marine Forecast Not Supported") | 2026-11-08 |
| `alerts-active-zone-ANZ338.geo.json` | `A/alerts/active?zone=ANZ338` (one Small Craft Advisory) | 2026-11-08 |
| `alerts-active-zone-ANZ338-event-gale-warning-empty.geo.json` | `A/alerts/active?zone=ANZ338&event=Gale%20Warning` (empty) | 2026-11-08 |
| `alerts-active-zone-LEZ146.geo.json` | `A/alerts/active?zone=LEZ146` (one Small Craft Advisory) | 2026-11-08 |
| `alerts-types.json` | `A/alerts/types` (111 event names) | 2026-11-08 |
| `products-cwf-okx-latest.json` | `A/products/types/CWF/locations/OKX/latest` (Coastal Waters Forecast, text, sections per zone) | 2026-11-08 |
| `products-nsh-cle-latest.json` | `A/products/types/NSH/locations/CLE/latest` (Nearshore Marine Forecast, Lake Erie, text) | 2026-11-08 |
| `products-nsh-locations.json` | `A/products/types/NSH/locations` | 2026-11-08 |
| `products-off-locations.json` | `A/products/types/OFF/locations` | 2026-11-08 |
| `products-off-nt2-latest.json` | `A/products/types/OFF/locations/NT2/latest` (Offshore Waters Forecast, covers ANZ820 to ANZ835) | 2026-11-08 |
| `products-glf-locations.json` | `A/products/types/GLF/locations` (LE LH LM LO LS SC SL) | 2026-11-15 |
| `products-glf-le-latest.json` | `A/products/types/GLF/locations/LE/latest` (Open Lake Forecast for Lake Erie, KCLE, FZUS61; LEZ161 to LEZ169 sections) | 2026-11-15 |
| `products-glf-lo-latest.json` | `A/products/types/GLF/locations/LO/latest` (Lake Ontario, KBUF, FZUS61; LOZ061 to LOZ065) | 2026-11-15 |
| `alerts-active-zone-GMZ250-gale-warning.geo.json` | `A/alerts/active?zone=GMZ250&event=Gale%20Warning` (one live Gale Warning, NWS Corpus Christi, Gulf of America) | 2026-11-15 |
