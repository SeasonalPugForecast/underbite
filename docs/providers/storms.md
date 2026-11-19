# Storm outlook and NHC hurricanes

Checked on 2026-11-15. Fixtures are in `tests/fixtures/raw/storms/` (index in its `README.md`).

## Code reused

The requests and parsers come from the whether project (`whether-providers`, revision 34bb5c0):
`open_meteo::storm` (`point_request`, `meta_request`, `parse_points`, `parse_run_meta`) and `nhc`
(`current_storms_request`, `adeck_request`, `bdeck_request`). All are sans-IO. Coordinates are
rounded to 2 decimals (`COORDINATE_DECIMALS`).

## Open-Meteo storm point request

No authentication, no special headers. Response is JSON, times as unix seconds (`timeformat=unixtime`)
with `timezone=auto`; the models are suffixed on each column (`pressure_msl_gfs_seamless`).

The Battery (40.70,-74.02), four models, 10 days:

```text
https://api.open-meteo.com/v1/forecast?latitude=40.70&longitude=-74.02&models=ecmwf_ifs025,gfs_seamless,gem_seamless,icon_seamless&hourly=precipitation,snowfall,wind_gusts_10m,pressure_msl&forecast_days=10&timezone=auto&timeformat=unixtime&wind_speed_unit=kmh&precipitation_unit=mm
```

| Field (per model suffix) | Unit | Notes |
|---|---|---|
| `precipitation` | mm | hourly |
| `snowfall` | cm | hourly |
| `wind_gusts_10m` | km/h | requested with `wind_speed_unit=kmh` |
| `pressure_msl` | hPa | hourly |
| `time` | unix seconds, UTC | |

Missing values are JSON `null` (ICON's last 52 hours are null in whether's recording). A model
Open-Meteo leaves out is missing from the response. A 400 body is `{"error":true,"reason":"..."}`.
The response format above is taken from whether's recording for New York ZIP 10001
(`whether-open-meteo-storm-point-10001.json`, 2026-11-14).

**Unverified: to record later.** The Battery point response itself: the request returned HTTP 429
on 2026-11-15, see below.

### Rate limit

On 2026-11-15 (about 22:22 UTC) the request above was answered with HTTP 429 and this JSON body:
`{"reason":"Daily API request limit exceeded. Please try again tomorrow.","error":true}`. Headers:

```text
X-RateLimit-Limit: 10000.0
X-RateLimit-Remaining: 0.0
X-RateLimit-Reset: <unix seconds, about 1.6 hours later>
X-RateLimit-Window: 86400
Retry-After: <seconds until the reset>
```

So the free API's daily limit is 10,000 calls per IP address, and it does send rate-limit headers
when the limit is exceeded. The limit is shared with every other Open-Meteo use from the same
address (Open-Meteo Marine, other apps). The `meta.json` responses carried no rate-limit headers.

## Run times: `meta.json`

`GET https://api.open-meteo.com/data/<domain>/static/meta.json`, no authentication, JSON. Verified
2026-11-15 for each domain.

| Model | Domain | `update_interval_seconds` | Temporal resolution |
|---|---|---|---|
| `ecmwf_ifs025` | `ecmwf_ifs025` | 21600 | 10800 s |
| `gfs_seamless` | `ncep_gfs025` | 21600 | 3600 s |
| `gem_seamless` | `cmc_gem_gdps_15km` | 43200 | 3600 s |
| `icon_seamless` | `dwd_icon` | 21600 | 3600 s |

| Field | Unit |
|---|---|
| `last_run_initialisation_time` | unix seconds, UTC; start of the run |
| `last_run_availability_time` | unix seconds, UTC; when the run became readable |
| `last_run_modification_time` | unix seconds, UTC |
| `data_end_time` | unix seconds, UTC |
| `update_interval_seconds`, `temporal_resolution_seconds`, `chunk_time_length` | seconds, seconds, steps |
| `crs_wkt` | text, present on some domains |

The older `cmc_gem_gdps` domain stopped updating in May 2026 (per whether's notes); use
`cmc_gem_gdps_15km`.

## Check interval

The "storm ahead" check uses one point request at most every 3 hours per spot, and only for spots
refreshed anyway. A new run appears at most every 6 hours (12 for GEM), so a newer run is detected
from `meta.json` before a larger request is made.

## NHC

No authentication. NHC requests are the same for every place and carry no coordinates.

- `https://www.nhc.noaa.gov/CurrentStorms.json`: active storms. On 2026-11-15 the answer was
  `{"activeStorms": []}` (HTTP 200, `Content-Type: application/json`, `last-modified` 2026-11-12).
- `https://ftp.nhc.noaa.gov/atcf/aid_public/a<id>.dat.gz`: a-deck, guidance and official forecast
  tracks, gzip; `<id>` is like `al102026`. whether sends `Cache-Control: no-cache`.
- `https://ftp.nhc.noaa.gov/atcf/btk/b<id>.dat`: b-deck, observed track, plain text.

ATCF rows are comma separated: basin, number, run start (`YYYYMMDDHH`, UTC), technique number, aid
code, forecast hour, latitude and longitude in tenths of a degree with a hemisphere letter (`395N`,
`668W`), maximum wind (kt), central pressure (hPa), storm type, then wind radii.

**Unverified (live):** a-deck and b-deck, and a non-empty `CurrentStorms.json`, because no storm was
active on 2026-11-15. The recordings of the sample storm Josephine (`al102026`) were made by whether
and copied with the files named `whether-nhc-*`; the a-deck is 309 KB gzip. whether notes that
public a-decks are 20 to 640 KB compressed in 2026.

## Credits and terms

- Open-Meteo data: CC BY 4.0, "Weather data by Open-Meteo.com". The ECMWF model data it serves is
  CC BY 4.0, credited to ECMWF. The free API is for non-commercial use.
- NHC data: US government work, public domain; credit "National Hurricane Center (NOAA)".
- Any storm display says the cone and tracks are NHC's, links the advisory, and is not for
  navigation or as a sole safety source.

Re-checked 2026-11-18: Open-Meteo: see `open-meteo-marine.md` (terms and licence read 2026-11-18); the forecast API page that lists ECMWF as a data provider was not read, so the ECMWF credit wording is **unverified**. NHC (`https://www.nhc.noaa.gov/disclaimer.shtml`): information on the server is in the public domain unless annotated otherwise and may be used freely, must not be modified and presented as official, and the server "is not an official source" for the products; no credit wording is prescribed.
