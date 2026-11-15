# NWS marine forecasts and alerts (`api.weather.gov`)

Verified on 2026-11-08 from the official documentation and live requests. Recorded responses are in
`tests/fixtures/raw/nws-marine/` (index: `README.md` there). Anything not checked is marked
**unverified**. API version reported by the OpenAPI document (`/openapi.json`): weather.gov API 3.12.2.

Documentation used: <https://www.weather.gov/documentation/services-web-api> and the OpenAPI document
<https://api.weather.gov/openapi.json>.

## Summary

| Item | Finding |
|---|---|
| Base URL | `https://api.weather.gov` (HTTPS, HTTP/2; no version in the path) |
| Authentication | none. The documentation says the `User-Agent` identifier "will be replaced with an API key in the future" |
| Required header | `User-Agent` identifying the application, for example `underbite-dev (contact via repository)`. The documentation's example is `(myweatherapp.com, contact@myweatherapp.com)` and says contact details let NWS reach the operator. All requests below succeeded with the contact-free string above |
| `Accept` | default is GeoJSON. `application/geo+json` and `application/ld+json` were tested; the documentation also lists DWML (forecast endpoints), OXML, CAP and Atom. The zone forecast is far smaller as JSON-LD (22 KB vs 100 KB). Products were served as JSON-LD whichever `Accept` was sent |
| Rate limit | "reasonable rate limits"; the number is not public; when exceeded the request fails and "may be retried after the limit clears (typically within 5 seconds)". No rate-limit headers were returned. Exact error status for a limited request: **unverified** (not provoked) |
| Caching | `Cache-Control` and `Expires` are set per endpoint (table below) and should drive the app's cache |
| Terms and licence | no licence or attribution wording on the documentation page; it describes the data as "intended to be open data, free to use for any purpose" and without fees. US Government work |
| Deprecations | none announced on the documentation page ("No significant known issues") |
| Time zone | timestamps are ISO 8601 with an offset (`2026-11-08T01:57:00-05:00`) or UTC (`+00:00`); `validTime` values are ISO 8601 `start/duration` intervals |

Cache lifetimes seen on 2026-11-08:

| Endpoint | `Cache-Control` |
|---|---|
| `/points/{lat},{lon}` | `max-age=86400, s-maxage=86400` |
| `/zones?...` | `max-age=2592000, s-maxage=120` |
| `/zones/{type}/{id}` | `max-age=1220917, s-maxage=604800` (counts down to a fixed expiry) |
| `/zones/{type}/{id}/forecast` | `max-age=3600, s-maxage=120` (also seen as 904 s: counts down to expiry) |
| `/alerts/active...` | `max-age=5, s-maxage=5` |
| `/products/types/{t}/locations/{o}/latest` | `max-age=60, s-maxage=60` |
| `/products/types/{t}/locations/{o}`, `/products/types/{t}` | `max-age=120, s-maxage=120` |

## Finding the marine zone for a point

Two equivalent routes; both returned the same zone at every test point.

1. `GET /points/{lat},{lon}` returns a Feature whose `properties` include `type` (`"land"` or `"marine"`),
   `cwa` and `gridId`/`gridX`/`gridY` (office and grid), `forecastZone` (a URL ending in the zone id),
   `timeZone`, `forecast`, `forecastHourly`, `forecastGridData`, `observationStations`, `radarStation`,
   `relativeLocation` and `astronomicalData` (sunrise, sunset, twilight times with local offsets).
   The response `id` normalises the coordinates (`40.7033,-74.0170` became `40.7033,-74.017`); behaviour with more than four decimals was not tested (**unverified**). A marine point is flagged by `"type": "marine"`.
2. `GET /zones?point={lat},{lon}` (optionally `&type=coastal` or `&type=offshore`) returns a
   FeatureCollection of the zones that contain the point: `properties.id`, `type`, `name`,
   `forecastOffice`, `gridIdentifier`, `cwa`, `timeZone`, `effectiveDate`, `expirationDate`.
   `geometry` is `null` in lists. Over land the list holds county, public, fire-weather zones and so on;
   over water it holds one marine zone.

| Point | `/points` | `/zones?point` |
|---|---|---|
| 40.7033, -74.0170 (The Battery) | `type` marine, `gridId` OKX 33,41, `forecastZone` **ANZ338** | `ANZ338`, type `coastal`, "New York Harbor" |
| 40.25, -73.16 (near NDBC 44025) | `type` marine, `gridId` OKX, `forecastZone` **ANZ383** | `ANZ383`, `coastal`, "Waters from Fire Island Inlet NY to Moriches Inlet NY from 20 to 60 NM"; `?type=offshore` returns an empty `features` list |
| 39.5, -72.0 | `type` marine, **no `gridId`**, `forecastZone` **ANZ815** | `ANZ815`, type `offshore`, "South of Long Island between Montauk Point and Sandy Hook to 1000 FM" |
| 41.9, -81.6 (Lake Erie, past 5 NM) | marine, CLE, `forecastZone` **LEZ166** | `LEZ166`, `coastal`, "Avon Point to Willowick OH beyond 5 nm off shoreline to US-Canadian border" |
| 41.55, -81.72 (Lake Erie off Cleveland) | marine, CLE 82,67, `forecastZone` **LEZ146** | `LEZ146`, `coastal`, "Avon Point to Willowick OH" (nearshore, within 5 NM) |
| 41.5, -81.7 (Cleveland, on land) | land | county OHC035, fire OHZ011 and others, no marine zone |
| 45.0, -30.0 (mid-Atlantic) | **HTTP 404**, problem type `.../problems/InvalidPoint`, title "Data Unavailable For Requested Point" | HTTP 200, empty `features` |

For an offshore point with no grid, `forecast`, `forecastHourly` and related URLs in `/points` should not be
followed (**unverified**: only the New York grid was fetched).

### Zone types and ids

`NWSZoneType` in the OpenAPI document: `land`, `marine`, `forecast`, `public`, `coastal`, `offshore`,
`fire`, `county`. In responses a marine zone has `type`:

| Zone family | `type` in the API | Id pattern | Example |
|---|---|---|---|
| Atlantic, Gulf and Pacific coastal waters (Coastal Waters Forecast, to 60 NM) | `coastal` | `ANZ###`, `AMZ###`, `GMZ###`, `PZZ###` seen; Hawaii, Alaska and Pacific-island prefixes (`PHZ`, `PKZ`, `PMZ`, `PSZ`) are **unverified** | ANZ338 New York Harbor, ANZ383 |
| Offshore waters (Offshore Waters Forecast) | `offshore` | `ANZ8##`, `ANZ9##`, and the same prefixes elsewhere | ANZ815, ANZ820 (Hudson Canyon to Baltimore Canyon), ANZ920 |
| Great Lakes nearshore (to 5 NM), and open lake beyond 5 NM | both `coastal` | `LEZ###` seen; the other lakes follow the area codes (`LMZ`, `LOZ`, `LHZ`, `LSZ`, `LCZ`, `SLZ`), **unverified** | LEZ146 nearshore; LEZ166 beyond 5 NM |

The `type` value alone does not distinguish Great Lakes nearshore from open-lake zones: the prefix (`LE`,
`LO`, ...) identifies the lake and the name ends with "beyond 5 nm" for the open-lake zones (Lake Erie ids
`LEZ142` to `LEZ149` and `LEZ020`/`040`/`041` are nearshore, `LEZ162` to `LEZ169` and `LEZ061` are beyond 5 NM,
`LEZ444` is Michigan waters; list: `/zones?type=coastal&area=LE`, 21 zones, fixture
`zones-coastal-area-LE.geo.json`). The `area` filter takes marine area codes: `AM`, `AN`, `GM`, `LC`, `LE`,
`LH`, `LM`, `LO`, `LS`, `PH`, `PK`, `PM`, `PS`, `PZ`, `SL` (OpenAPI `MarineAreaCode`). Region codes (for
alerts): `AL`, `AT`, `GL`, `GM`, `PA`, `PI`. The 21 `LEZ` zones come back under both `type=coastal` and
`type=marine`; zone metadata is served under `/zones/coastal/{id}` and `/zones/forecast/{id}` alike.

Zone metadata `GET /zones/coastal/ANZ338` (JSON-LD): `id`, `type`, `name`, `effectiveDate`,
`expirationDate` (`2200-01-01` means open-ended), `geometry` (WKT polygon in JSON-LD, GeoJSON polygon
otherwise; 21 KB to 100 KB), `forecastOffice`, `gridIdentifier`, `cwa`, `timeZone`. `state` is `null` and
`observationStations` is empty for marine zones. Zone boundaries were last modified 2026-04-16 (Last-Modified header, `effectiveDate`).

## The marine zone forecast

### `GET /zones/{type}/{id}/forecast`

Structured, but text only. It works for Coastal Waters Forecast zones and returns the same data under
`/zones/coastal/ANZ338/forecast` and `/zones/forecast/ANZ338/forecast`.

JSON-LD (`Accept: application/ld+json`) keys: `zone` (URL), `updated` (ISO 8601 with offset, the issuance time of the
product, `2026-11-08T01:57:00-05:00`), `geometry` (WKT) and `periods`. GeoJSON wraps the same fields in
`properties` and carries a GeoJSON polygon.

`periods[]`: exactly 10 entries on 2026-11-08, each with only three fields:

| Field | Type | Example |
|---|---|---|
| `number` | integer | `1` |
| `name` | string | `Today`, `Tonight`, `Mon`, `Mon Night`, ... `Thu Night` |
| `detailedForecast` | string | `W winds 15 to 20 kt with gusts up to 25 kt. Waves around 2 ft. Wave Detail: W 2 ft at 3 seconds.` |

There are no start and end times, no numeric wind, wave or period fields and no units: winds are in knots
and waves or seas in feet, written in the text. The text format differs by office: New York Harbor uses
"Waves around 2 ft" with a "Wave Detail" sentence, ANZ383 uses "Seas 5 to 8 ft, occasionally to 10 ft", other offices use
"Northeast winds 15 to 20 knots". Any parser must treat the sentences as free text (search for `kt`/`knots`,
`ft`, `seconds`) or show them as is. Period dates are not given; `name` is a day-part label, and the
`updated` stamp is the only date. Day parts follow the issuing office's local time (the text product header shows EST).

Not every marine zone has a forecast here:

| Request | Result |
|---|---|
| `/zones/forecast/ANZ338/forecast`, `/zones/coastal/ANZ383/forecast`, `ANZ335`, `PZZ535`, `GMZ330` | HTTP 200, 10 periods |
| `/zones/forecast/LEZ146/forecast`, `/zones/coastal/LEZ146/forecast`, `/zones/coastal/LEZ166/forecast` (Great Lakes) | **HTTP 404**, `application/problem+json`, title "Not Found" |
| `/zones/offshore/ANZ820/forecast`, `/zones/forecast/ANZ820/forecast` | **HTTP 404**, title "Marine Forecast Not Supported", detail "Forecasts for marine areas are not yet supported by this API." |

So the structured endpoint covers coastal (CWF) zones of the Atlantic, Pacific and Gulf coasts, and nothing on
the Great Lakes or offshore. A cache for a 404 is `private, max-age=3600`.

### Gridpoint data for marine points

`/points` for a marine point also yields `forecastGridData` (for example `/gridpoints/OKX/33,41`). The
documentation says "at this time coastal marine grid forecasts are only available from the
`forecastGridData` property". On 2026-11-08 the New York Harbor grid had `waveHeight` (`wmoUnit:m`, 6
intervals of `validTime` such as `2026-11-08T04:00:00+00:00/P1DT23H`, value 0.3048) and `wavePeriod`
(`nwsUnit:s`, 5 intervals), plus `windSpeed`/`windGust` (`wmoUnit:km_h-1`), `windDirection`, `temperature` and
the other land fields; `waveDirection`, `primarySwell*`, `secondarySwell*` and `windWaveHeight` were empty
arrays. The grid is coarse: a handful of multi-hour intervals, and the first `waveHeight` value is 0.3048 m (exactly one foot).
`/gridpoints/OKX/33,41/forecast` returned a *land* style forecast ("Sunny. High near 62 ...", 14 periods) for
the harbour point, which is not a marine forecast. These responses were not recorded as fixtures
(about 100 KB); a Great Lakes or offshore grid was not fetched (**unverified**).

### Text products: `/products`

The full marine text products are in the products API and cover the Great Lakes and offshore zones that the
zone endpoint does not.

| Code | Product name | Covers | Observed location ids |
|---|---|---|---|
| `CWF` | Coastal Waters Forecast | coastal zones to 60 NM | issuing WFO id, e.g. `OKX` (WMO header `FZUS51 KOKX`) |
| `NSH` | Nearshore Marine Forecast | Great Lakes, within 5 NM | `APX BUF CLE DLH DTX GRB GRR IWX LOT MKX MQT` (`/products/types/NSH/locations`) |
| `GLF` | Great Lakes Forecast (open lakes) | Great Lakes beyond 5 NM | `LE LH LM LO LS SC SL` |
| `OFF` | Offshore Forecast | offshore waters | `N01`..`N15`, `NT1`..`NT5`, `PZ5`..`PZ8`, plus Alaska and Hawaii offices; the western-Atlantic NY offshore zones (ANZ820 to ANZ835) are in `NT2` (header `FZNT22 KWBC`, `OFFNT2`) |
| `HSF` | High Seas Forecast | open ocean | **unverified** |
| `MWW` | Marine Weather Message | marine warnings text | listed in `/products/types`; contents **unverified** |
| `SMW` | Special Marine Warning | short-fuse warnings | listed; contents **unverified** |
| `SRF` | Surf Forecast | surf zone | listed; contents **unverified** |

Requests:

- `GET /products/types/CWF/locations/OKX/latest` is the latest product (JSON-LD).
- `GET /products/types/CWF/locations/OKX` is a list of recent issuances (`id`, `wmoCollectiveId`,
  `issuingOffice`, `issuanceTime`, `productCode`, `productName`) without text. The type-wide list
  `/products/types/CWF` was about 460 KB, so avoid it.
- `GET /products/{id}` returns one product by id.

Product object: `id`, `wmoCollectiveId` (`FZUS51`), `issuingOffice` (`KOKX`), `issuanceTime` (UTC),
`productCode`, `productName`, `productText` (the plain-text bulletin, starting with a blank line and the WMO
heading). The text has a fixed layout:

```text
FZUS51 KOKX 080657
CWFOKX
Coastal Waters Forecast
National Weather Service New York NY
157 AM EST Sun Nov 8 2026
...
ANZ338-081930-
New York Harbor-
157 AM EST Sun Nov 8 2026

...SMALL CRAFT ADVISORY IN EFFECT THROUGH MONDAY AFTERNOON...

.TODAY...W winds 15 to 20 kt with gusts up to 25 kt. Waves around
2 ft. Wave Detail: W 2 ft at 3 seconds.
.TONIGHT...
$$
```

- A section starts at a UGC line (`ANZ338-081930-`, `LEZ142-143-081515-`, `LEZ146>148-081515-`): zone ids
  with `-` separators, `>` for a numeric range (`LEZ146>148` is LEZ146, LEZ147, LEZ148), then a `DDHHMM`
  expiry in UTC, `081930` meaning the 8th at 19:30 UTC, and ends at `$$`.
- Zone names follow, then the issue time in local time, optional `...HEADLINES...` (the active
  advisory), and periods starting with `.` and an all-caps label (`.TODAY...`, `.MON NIGHT...`), wrapped at about 65
  columns.
- Great Lakes NSH text adds "The water temperature off Toledo is 58 degrees, off Cleveland 61 degrees, and
  off Erie 59 degrees." (Fahrenheit, one sentence at the end of the section).
- Several zones can share one section (NSH `LEZ144-145`, `LEZ146>148`), so the sections must be keyed by
  every expanded id.
- Lake Erie NSH wording: "Southwest winds 15 to 25 knots becoming west. Sunny. Waves 1 to 3 feet building to 2 to 4
  feet." with an extra "Waves occasionally around 9 feet." line.

The text is in the office's local time and states knots and feet. No machine-readable wind or wave values
exist in these products; a `MFM` (Marine Forecast Matrix) product is listed as a type and was not fetched
(**unverified**).

## Active marine alerts

Query by zone, point or area:

| Request | Use |
|---|---|
| `GET /alerts/active?zone=ANZ338` (also `/alerts/active/zone/ANZ338`) | alerts for a forecast or county zone id; the OpenAPI document allows a comma-separated list (not requested) |
| `GET /alerts/active?point=40.7033,-74.0170` | alerts whose area contains the point; for a point over water it gave the same single alert as the zone |
| `GET /alerts/active?area=LE` | marine area code (or a state code); 33 KB for Lake Erie |
| `GET /alerts/active?region=GL` (marine region) | whole region; the Great Lakes small-craft query was 142 KB, so filter. `?region_type=marine` is in the OpenAPI document but was not requested |
| `&event=Small%20Craft%20Advisory` | filter by event name (letters, digits and spaces only) |
| `&status=`, `&severity=`, `&urgency=`, `&certainty=`, `&message_type=`, `&code=` | additional filters from the OpenAPI document, not requested (**unverified**); `zone`, `point`, `area` and `region` cannot be combined (stated in the OpenAPI parameter descriptions) |

An empty result is a normal HTTP 200 FeatureCollection with `"features": []` and a `title` such as
"Current Gale Warning events for ANZ338" (fixture `alerts-active-zone-ANZ338-event-gale-warning-empty.geo.json`).
Alerts are valid per `expires`, and the endpoint is cached for 5 seconds only.

Alert properties seen: `id` (a `urn:oid:...` string), `areaDesc` (zone names joined with `; `), `geocode.SAME`,
`geocode.UGC` (the zone ids covered: `ANZ331`, `ANZ332`, `ANZ335`, `ANZ338`, `ANZ340`, `ANZ345`), `affectedZones`
(URLs), `sent`, `effective`, `onset`, `expires`, `ends`, `status` (`Actual`), `messageType` (`Alert`, `Update`,
`Cancel`), `category` (`Met`), `severity` (`Minor`), `certainty` (`Likely`), `urgency` (`Expected`), `response`
(`Avoid`), `event`, `eventCode` (`NationalWeatherService: ["SCY"]`, `SAME: ["NWS"]`), `sender`, `senderName`
("NWS Upton NY"), `headline`, `description` (the full text with `* WHAT...`, `* WHERE...`, `* WHEN...`,
`* IMPACTS...` bullets), `instruction`, `web`, `parameters` (`VTEC`, `AWIPSidentifier` `MWWOKX`,
`WMOidentifier`, `NWSheadline`, `eventEndingTime`, `expiredReferences`), `references`. Geometry is `null`;
use `geocode.UGC` and `affectedZones`.

Observed on 2026-11-08: `Small Craft Advisory` for ANZ338 (New York Harbor, VTEC `/O.EXT.KOKX.SC.Y.0100...`,
ending 2026-11-09 18:00 EST) and for LEZ146 to LEZ148 (VTEC `/O.CON.KCLE.SC.Y.0053...`, ending 2026-11-09 15:00 EST).
The VTEC code `SC.Y` is Small Craft Advisory.

### Marine event names

The list in `/alerts/types` (111 events; fixture `alerts-types.json`) includes these marine-related names,
exact spelling:

- Wind and sea: `Small Craft Advisory`, `Gale Watch`, `Gale Warning`, `Storm Watch`, `Storm Warning`,
  `Hurricane Force Wind Watch`, `Hurricane Force Wind Warning`, `Hazardous Seas Watch`,
  `Hazardous Seas Warning`, `Brisk Wind Advisory`, `Lake Wind Advisory`, `Special Marine Warning`,
  `Marine Weather Statement`, `Freezing Spray Advisory`, `Heavy Freezing Spray Watch`,
  `Heavy Freezing Spray Warning`.
- Shore: `High Surf Advisory`, `High Surf Warning`, `Rip Current Statement`, `Beach Hazards Statement`,
  `Low Water Advisory`, `Coastal Flood Advisory`/`Statement`/`Watch`/`Warning`, `Lakeshore Flood
  Advisory`/`Statement`/`Watch`/`Warning`, `Storm Surge Watch`, `Storm Surge Warning`.
- Other: `Dense Fog Advisory`, `Tsunami Advisory`, `Tsunami Watch`, `Tsunami Warning`, `Tropical Storm Watch`,
  `Tropical Storm Warning`, `Hurricane Watch`, `Hurricane Warning`.

The list has no separate "Small Craft Advisory for Hazardous Seas" or "for Winds" entries. A `Gale Warning`
was not active on 2026-11-08, so its payload was not recorded (**unverified** beyond the event name and its
VTEC code `GL.W`, which is not confirmed by a live alert).

## Differences between NWS zone types and what the app should do

- To show "marine forecast" text for a point: `/points` (or `/zones?point`) to get the zone id; for `ANZ`,
  `AMZ`, `GMZ`, `PZZ` coastal zones read `/zones/coastal/{id}/forecast`; for Great Lakes zones
  fetch the `NSH` product for the zone's office (`gridIdentifier` in the zone metadata, here `CLE`) and cut the
  section for the zone id; for offshore zones fetch the `OFF` product (`NT2` for the New York offshore
  zones).
- To warn: `/alerts/active?zone={id}` or `?point=`, matching on `event` and `status`.
- Coastal-zone ids can be cached for a long time (the zone list is stable and refreshed with an
  effective date), while the forecast and alerts follow their `Cache-Control`.

## Test places

- New York Harbor: zone ANZ338 (coastal, WFO OKX, time zone America/New_York).
- 40.25, -73.16: zone ANZ383 (coastal, 20 to 60 NM); no offshore zone at this point.
- Lake Erie off Cleveland: zone LEZ146 (nearshore, WFO CLE), LEZ166 (beyond 5 NM); zone forecast is a
  404 for both, and the `NSH` product is used.

## Addendum 2026-11-15: open-lake forecasts (GLF)

Verified with `curl` against `https://api.weather.gov` with `User-Agent: underbite-dev (contact via repository)`.

- Product: `GLF`, "Great Lakes Forecast", titled "Open Lake Forecast for Lake Erie" and described as the
  forecast "for waters beyond five nautical miles of shore". It complements `NSH` (nearshore, out to 5 NM).
- Locations: `/products/types/GLF/locations` lists `LE LH LM LO LS SC SL` (Erie, Huron, Michigan, Ontario,
  Superior, St. Clair, St. Lawrence). The listing carries names as `null`.
- Latest: `/products/types/GLF/locations/{code}/latest` returns the full text in `productText` (JSON-LD,
  fields `id`, `wmoCollectiveId`, `issuingOffice`, `issuanceTime`, `productCode`, `productName`). The
  response is HTTP 200 with `cache-control: public, max-age=60`; no rate-limit headers were present.
- `LE`: issued by `KCLE` (Cleveland) under WMO `FZUS61`, about every 6 hours (observed 253 PM EST).
  `LO` is issued by `KBUF` (Buffalo), also `FZUS61`. The location code is a Great Lake, not an office; the
  `LE` product is not a per-office product, so Lake Erie's open-lake zones from Buffalo (LEZ061) and
  Cleveland are both inside the one `LE` product.
- Layout: a header (`FZUS61 KCLE 151953`, `GLFLE`), a title, issue time, a `.SYNOPSIS...` paragraph, then
  blocks that each start with a UGC line followed by zone descriptions (wrapped, joined by `-`), the issue
  time line, the periods and `$$`. UGC lines use the NSH grammar: ids separated by `-`, `>` ranges, then
  a `DDHHMM` expiry: `LEZ162-163-160315-`, `LEZ164>168-160315-`, `LEZ061-169-160315-`, `LEZ161-160315-`.
- Zone mapping on 2026-11-15: `LEZ162` Detroit River Lt. to Maumee Bay to Reno Beach; `LEZ163` Reno Beach to
  The Islands; `LEZ164>168` The Islands to Vermilion, Vermilion to Avon Point, Avon Point to Willowick,
  Willowick to Geneva-on-the-Lake, Geneva-on-the-Lake to Conneaut (so **LEZ166 is Avon Point to Willowick,
  beyond 5 NM**, inside the `LEZ164>168` range); `LEZ061` Ripley to Buffalo; `LEZ169` Conneaut to Ripley;
  `LEZ161` is a MAFOR (marine forecast code) block, not a normal forecast. Zone descriptions are hard
  wrapped, so names must be rebuilt from lines up to the issue-time line. The nearshore `LEZ142` to
  `LEZ149` stay in `NSH` `CLE`; the Lake Erie zones from `LEZ141` upward are not all in one product.
- Periods: `.TONIGHT...`, `.MONDAY...`, `.MONDAY NIGHT...` up to eight periods, same dot-prefixed layout as
  NSH, wrapped at about 65 columns. Text is "{direction} winds {n} to {m} knots{ becoming ...}. Waves {n} to
  {m} feet." (also "N feet or less", "around N knots", "building to", "subsiding to", "increasing to").
  Unlike NSH, there is no `Waves ... occasional` clause in the sample and wave height is the significant
  wave height, with the occasional height defined in the preamble only. Units are knots and feet, times are
  local (EST here, shown in the header lines).
- Which product answers a given zone: query `/zones/coastal/{zone}` or `/points`; a LEZ zone numbered
  from 142 to 149 maps to `NSH` and 160 to 169 to `GLF`, with `LEZ061` and `LOZ06x` open-lake zones in the
  same GLF text. The numbering rule is inferred from one product per lake and is **unverified** for Huron,
  Michigan, Superior and St. Clair (`LH LM LS SC SL` were not fetched).
- `LO` (Lake Ontario) UGC lines: `LOZ062-160400-`, `LOZ063>065-160400-`, `LOZ061-160400-`; same layout.

### Live Gale Warning payload

`/alerts/active?zone=GMZ250&event=Gale%20Warning` returned one alert (event `Gale Warning`, severity
`Moderate`, urgency `Expected`, `geocode.UGC` of four GMZ zones, `VTEC` `/O.CON.KCRP.GL.W.0007.../`, `eventCode.NationalWeatherService` `GLW`). It has the same shape as the Small Craft Advisory
payload recorded earlier. The unfiltered `/alerts/active?event=Gale%20Warning` response was about 480 KB
(125 alerts), so the app must always filter by zone. Alert `expires` can be earlier than `ends`.

### Still unverified

- The documented status code for rate limiting (the API documentation does not state one; none was
  provoked).
- GLF for `LH LM LS SC SL`, and LEZ166 on a day with a Gale Warning on the Great Lakes.
