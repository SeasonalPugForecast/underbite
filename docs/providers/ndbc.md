# NOAA NDBC: realtime2 station files and the active-station list

Verified on 2026-11-08 against the live service and these official pages:

- Realtime access: <https://www.ndbc.noaa.gov/faq/rt_data_access.shtml>
- Measurement descriptions and units (column definitions): <https://www.ndbc.noaa.gov/faq/measdes.shtml>
- NWS disclaimer that applies to the site: <https://www.weather.gov/disclaimer>
- Service change notice 23-85 (seven-character station ids): <https://www.weather.gov/media/notification/pdf_2023_24/scn23-85_ndbc_web_file_format_change.pdf>

Recorded files are in `tests/fixtures/raw/ndbc/` (index: `tests/fixtures/raw/ndbc/README.md`). Test place: buoy
44025 (Long Island, 30 NM south of Islip, 40.258 N, 73.175 W). Because 44025 currently reports waves and water
temperature only, the files of 44065 (New York Harbor Entrance), 44069 (Great South Bay, has `.ocean`), 44091
(Barnegat, has `.adcp`) and the C-MAN style file `BATN6` (The Battery) were recorded as well.

## 1. Requests

| What | URL | Notes |
|---|---|---|
| Standard meteorological and wave summary values | `https://www.ndbc.noaa.gov/data/realtime2/44025.txt` | 160 KB, 1702 lines for 44025 |
| Detailed wave summary | `https://www.ndbc.noaa.gov/data/realtime2/44025.spec` | 120 KB; buoys that measure waves |
| Oceanographic (temperature at depth, salinity, oxygen, chlorophyll, turbidity, pH) | `https://www.ndbc.noaa.gov/data/realtime2/44069.ocean` | **44025 has none**: the request returns HTTP 404 with an HTML page (fixture `ndbc-44025.ocean-404.html`) |
| Ocean current profile | `https://www.ndbc.noaa.gov/data/realtime2/44091.adcp` | stations with `currents="y"` |
| Spectral directions and coefficients | `.swdir`, `.swdir2`, `.swr1`, `.swr2` | exist for 44025, about 1.1 MB each; not recorded and not needed |
| Active stations | `https://www.ndbc.noaa.gov/activestations.xml` | 274 KB, recorded complete |

The access page also lists the types `drift`, `cwind`, `data_spec` and `tide`. For 44025 `.cwind`, `.supl`, `.drift`,
`.adcp`, `.rain`, `.dart` and `.hkp` returned 404 (`.tide` and `.data_spec` were not tried, **unverified**).

- **File names**: `{station_id}.{type}`. Buoy ids are digits (`44025.txt`). Land (C-MAN and partner) stations
  use the upper-case id: `BATN6.txt` works, `batn6.txt` returns 404. `activestations.xml` lists ids in lower
  case for those stations (`batn6`), so upper-case them when building the URL.
- **Authentication**: none. **User-Agent**: not required (curl default accepted). **Rate limit**: none published
  and no limit headers; the page asks callers to "limit your retrievals to a minimal level" and says most stations
  report hourly with data available about 25 minutes after the hour. HTTPS is the stated protocol; anonymous FTP
  (`data.ndbc.noaa.gov`) is documented but "expected to be discontinued".
- **Caching**: responses carry `last-modified`, `etag`, `accept-ranges: bytes`, `cache-control: max-age=600,
  must-revalidate`, and are served through CloudFront (`x-cache`, `age`). Conditional requests
  (`If-None-Match`, `If-Modified-Since`) were not tried (**unverified**). `activestations.xml` has
  `cache-control: max-age=60`. Refresh a station file no more than every 10 minutes; hourly is enough for most
  stations.
- **Encoding**: `content-type: text/plain; charset=ISO-8859-1`, LF line ends, final newline. `activestations.xml`
  is `text/xml`, UTF-8, ASCII only on this date.

## 2. File layout (`.txt`, `.spec`, `.ocean`, `.adcp`)

1. Two comment lines beginning with `#`: the column names, then the units. Both are padded with spaces so that
   they line up with the data columns. Comment lines must be skipped (the documentation says lines beginning
   with `#` are comments).
2. Data rows, **newest first**, columns separated by one or more spaces (not fixed width: split on whitespace).
3. Time: the first five columns are `YY MM DD hh mm` (the name is `YY` but the value is a four-digit year), **UTC**
   (the documentation: "Both Realtime and Historical files show times in UTC only").
4. Missing value: the literal `MM`, in any column after the time. A whole column can be `MM` for the full file.
   Other non-numeric tokens exist only in text columns (below).
5. Window: the last 45 days. The 44025 file on 2026-11-08 runs from `2026 09 24 01 10` to `2026 11 08 13 40`
   (45.5 days).
6. Observation spacing depends on the station: 44025 every 30 minutes (minutes 10 and 40), 44065 every 10
   minutes, 44069 every 15 minutes, 44091 `.adcp` every 30 minutes, `BATN6` every 6 minutes. Gaps occur (44025 has steps of 60,
   90, 120, 150 and 240 minutes in the 45 days), so do not assume regular steps.
7. Every row of a `.txt`, `.spec` or `.ocean` file has the same number of columns as the header (verified for every
   recorded file): 19 in `.txt`, 15 in `.spec` and `.ocean`. `.adcp` rows are shorter than the header (section 2.4).

### 2.1 `.txt` columns (19), identical header on 44025, 44065, 44069, 44091 and `BATN6`

Header lines of 44025:

```text
#YY  MM DD hh mm WDIR WSPD GST  WVHT   DPD   APD MWD   PRES  ATMP  WTMP  DEWP  VIS PTDY  TIDE
#yr  mo dy hr mn degT m/s  m/s     m   sec   sec degT   hPa  degC  degC  degC  nmi  hPa    ft
```

| Column | Unit | Meaning | Format seen | Notes for 44025 on 2026-11-08 |
|---|---|---|---|---|
| `YY MM DD hh mm` | UTC | observation time | `2026 11 08 13 40` | |
| `WDIR` | degrees true | direction the wind comes from | integer | all 1700 rows `MM` |
| `WSPD` | m/s | wind speed, 8-minute mean on buoys, 2-minute on land stations | one decimal | all `MM` |
| `GST` | m/s | peak 5 or 8 second gust in that period | one decimal | all `MM` |
| `WVHT` | m | significant wave height (mean of highest third over a 20-minute sample) | one decimal | present in all rows, 0.2 to 5.8 |
| `DPD` | s | dominant wave period | integer | `MM` in 7 rows, 3 to 15 |
| `APD` | s | average wave period | one decimal | `MM` in 1 row, 3.1 to 8.3 |
| `MWD` | degrees true | direction the dominant-period waves come from | integer | `MM` in 8 rows, 1 to 359 |
| `PRES` | hPa | sea-level pressure | one decimal | all `MM` |
| `ATMP` | °C | air temperature | one decimal | all `MM` |
| `WTMP` | °C | sea-surface temperature | one decimal | `MM` in 86 rows, 15.8 to 20.2 |
| `DEWP` | °C | dew point | one decimal | all `MM` |
| `VIS` | nautical miles | visibility (buoys report 0 to 1.6) | | all `MM` |
| `PTDY` | hPa | 3-hour pressure tendency, signed with an explicit `+` (for example `+1.3`) | | all `MM` |
| `TIDE` | feet above MLLW | water level | | all `MM` (the only column in feet) |

Station 44025 therefore currently yields waves and sea temperature only, although `activestations.xml` lists it
with `met="y"`. 44065 has wind, pressure, air and water temperature (waves only in some rows: `WVHT` is `MM` in
3383 of 6407 rows) and no visibility or tide. `BATN6` (6-minute) has pressure and sea temperature, air temperature
in about half the rows, and `MM` for wind, waves, dew point, visibility and tide. Decide per column and per row whether a
value is present; never treat `MM` as zero.

### 2.2 `.spec` columns (15)

```text
#YY  MM DD hh mm WVHT  SwH  SwP  WWH  WWP SwD WWD  STEEPNESS  APD MWD
#yr  mo dy hr mn    m    m  sec    m  sec  -  degT     -      sec degT
2026 11 08 13 40  1.8   MM   MM   MM   MM  MM  MM VERY_STEEP  4.6 281
```

| Column | Unit | Meaning | Notes |
|---|---|---|---|
| `WVHT` | m | significant wave height | |
| `SwH`, `SwP` | m, s | swell height, swell period | all `MM` at 44025 on this date |
| `WWH`, `WWP` | m, s | wind-wave height, period | all `MM` at 44025 |
| `SwD`, `WWD` | compass text | swell and wind-wave direction as a compass point (`SE`, `SSE`, `WNW`, `NW`, `W`), **not** degrees although the unit row shows `-` and `degT` | `MM` when absent; present at 44065 |
| `STEEPNESS` | - | one of `VERY_STEEP`, `STEEP`, `AVERAGE`, `N/A` (44025 has all four; `N/A` is not `MM` and is used when it cannot be computed) | text |
| `APD` | s | average wave period | |
| `MWD` | degrees true | mean wave direction | |

### 2.3 `.ocean` columns (15), recorded for 44069

```text
#YY  MM DD hh mm   DEPTH  OTMP   COND   SAL   O2% O2PPM  CLCON  TURB    PH    EH
#yr  mo dy hr mn       m  degC  mS/cm   psu     %   ppm   ug/l   FTU     -    mv
```

`DEPTH` is the measurement depth in metres, `OTMP` the direct water temperature in °C, `COND` mS/cm, `SAL` PSU,
`O2%`, `O2PPM`, `CLCON` (chlorophyll, µg/L), `TURB` (turbidity, **FTU**, the app's canonical unit is FNU, so
note the difference), `PH`, `EH` (mV). At 44069 only `DEPTH`, `OTMP` and `SAL` have values. Which sensors exist
varies by station, so read the header every time and map by name.

### 2.4 `.adcp` columns, recorded for 44091

The header lists `DEP01 DIR01 SPD01 … DEP08 DIR08 SPD08` (depth in metres, direction in degrees true the water
flows toward, speed in cm/s) but the data rows carry only the bins that reported: 44091 rows have three values
after the time (`1 110 28`). The number of values per row can therefore be smaller than the header. 44025 has no
`.adcp`.

### 2.5 Column sets that vary

- The `.txt` column set was identical at all five recorded stations. The documentation shows a different layout for
  drifting buoys and uncrewed surface vehicles (`#YY MM DD hhmm LAT LON WDIR …`, time as `hhmm`) in `.drift`;
  not fetched (**unverified**).
- `.ocean` and `.adcp` column sets depend on the sensors; `.adcp` rows are ragged.
- Column sets may change when a new year starts (older descriptions of NDBC files say so). This could **not be
  verified**: the 45-day window on 2026-11-08 does not span a year boundary. Re-check after 2027-01-01 by parsing
  the header on every fetch and mapping by name.
- Seven-character station ids exist (18 in `activestations.xml`, for example `4403585`, `5401000`), introduced by
  service change notice 23-85 (effective about 2023-08-15; the notice concerns the latest-observation file and
  the `hourly2` directory, and says the file names of such stations are seven characters). Do not assume five
  characters.

## 3. `activestations.xml`

`<stations created="2026-11-08T14:25:05UTC" count="1357">` followed by one empty `<station …/>` element per line
and a leading comment: "Site Elevation (elev attribute), when present, is reported in meters above mean sea
level." Fixture: `ndbc-activestations.xml` (complete).

| Attribute | Meaning | Values seen |
|---|---|---|
| `id` | station id (lower case for land stations) | 4, 5 or 7 characters |
| `lat`, `lon` | decimal degrees, west negative | `40.258`, `-73.175` (integers for some PIRATA buoys) |
| `elev` | metres above mean sea level; **missing on 264 stations** | `0`, `11` |
| `name` | free text; may be empty | `LONG ISLAND - 30 NM South of Islip, NY` |
| `owner`, `pgm` | operator and programme | `NDBC` / `NDBC Meteorological/Ocean`; `IOOS Partners`, `NOS/CO-OPS`, `NERRS`, `Tsunami`, `TAO`, `International Partners`, `Marine METAR` |
| `type` | `buoy`, `fixed`, `dart`, `tao`, `usv`, `oilrig`, `other` | |
| `met` | has meteorological data (`y`/`n`) | 44025 is `y` although its meteorological columns are empty now |
| `currents` | has current data (`.adcp`) | 44091 `y` |
| `waterquality` | has oceanographic data (`.ocean`) | 130 stations `y`; 44069 `y`, 44025 `n`, matching the files |
| `dart` | tsunami meter | |

Treat the flags as hints, not guarantees: `met="y"` at 44025 does not mean its wind columns have values.
Station search: filter by `lat`/`lon`, then request `.txt` and use the first row with a non-`MM` value for each
needed column. Nearby stations of interest for New York Harbor: 44025 (40.258, -73.175), 44065 (40.368, -73.701),
44069 (40.699, -73.087), `batn6` (The Battery, 40.701, -74.014), `sdhn4` (Sandy Hook), `robn4` (Robbins Reef).

## 4. Field table for the model

| Wanted value | File and column | Unit in file | Canonical unit | Presence at 44025 (2026-11-08) |
|---|---|---|---|---|
| wave height | `.txt` `WVHT` (or `.spec`) | m | m | yes |
| dominant period | `.txt` `DPD` | s | s | yes |
| average period | `.txt` `APD` | s | s | yes |
| wave direction | `.txt` `MWD` | degrees true from | | yes |
| swell, wind-wave parts | `.spec` `SwH SwP SwD WWH WWP WWD` | m, s, compass text | | all `MM` |
| wind speed, gust, direction | `.txt` `WSPD`, `GST`, `WDIR` | m/s, m/s, degrees from | m/s | no |
| pressure and trend | `.txt` `PRES`, `PTDY` | hPa | hPa | no |
| air temperature, dew point | `.txt` `ATMP`, `DEWP` | °C | °C | no |
| sea-surface temperature | `.txt` `WTMP` | °C | °C | yes |
| water temperature at depth, salinity, turbidity, chlorophyll | `.ocean` | °C, psu, FTU, µg/L | °C, PSU, FNU | no file |

## 5. Licence, terms and attribution

- NDBC data are US Government works. The NWS disclaimer (<https://www.weather.gov/disclaimer>) states that
  information is in the public domain unless noted otherwise and may be used for lawful purposes provided the
  user does not claim it as their own, imply NOAA or NWS endorsement, or present altered content as official.
  The page sets no credit line; the data are provided "as is" and anyone using them assumes the risk. Suggested credit
  text: "Buoy observations: NOAA National Data Buoy Center."
- Stations in `activestations.xml` owned by others (`IOOS Partners`, `NERRS`, `NOS/CO-OPS`, `Marine METAR`, and
  so on) are redistributed by NDBC; whether their data carry additional terms was not checked (**unverified**).
- Realtime values are the output of automated checks only; a value shown from a file is "realtime" data, not the
  reviewed archive. Wave and wind sensors on buoys fail and are repaired without notice, so a fresh `MM` column is
  normal.

Re-checked 2026-11-18: public-domain and credit wording unchanged (`https://www.weather.gov/disclaimer`, to which the "Disclaimer" links on the NDBC home and station pages point).

Partner stations: no NDBC page read states additional terms for them. `activestations.xml` (read 2026-11-18) carries `pgm` values `IOOS Partners` (515 stations), `NOS/CO-OPS` (292), `International Partners` (140), `NERRS` (104) and `Marine METAR` (35), and an `owner` attribute naming the operator (for example `MYSOUND`, `Stony Brook University`, `National Estuarine Research Reserve System`, `Federal Aviation Administration`). The station pages for 44069 and ANMN6 show "Owned and maintained by <owner>"; the page for KATP (Marine METAR) shows no owner line and says "Data from this station are not quality controlled by NDBC." The weather.gov disclaimer has a "Use of Third-Party Data and Products" paragraph: third-party information is "used under license by the individual third-party provider", and readers are asked to "contact the third-party provider for information on your rights to further use these data/products". So NDBC publishes no per-partner licence; the rights to partner-owned stations are **unverified** beyond that paragraph. Pages tried that do not exist (404): `/data_policy.shtml`, `/ioos_partners.shtml`, `/faq/partners.shtml`.

## 6. Outages, changes and deprecations

- 2023-08-15 (about): station ids may be seven characters in some NDBC files (notice 23-85).
- Anonymous FTP is "expected to be discontinued" (documentation, undated).
- 2026-08-25: NDBC moves glider BUFR messages to a new template (notice 26-71); not related to the text files.
- No outage was observed during about 25 requests on 2026-11-08, and no dated notice of a realtime2 text-format
  change for 2026 was found.
