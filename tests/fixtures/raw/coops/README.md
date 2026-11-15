# CO-OPS raw fixtures

Raw response bytes from NOAA CO-OPS, recorded on 2026-11-08 (UTC) unless a line says otherwise. No keys or
credentials are involved. `DG` below is `https://api.tidesandcurrents.noaa.gov/api/prod/datagetter` and `MD` is
`https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi`. The shorthand `COMMON` means the query string
`application=underbite&time_zone=gmt&units=metric&format=json`. Where a line says "HTTP 400" the server
answered with that status; all other files are HTTP 200.

Files whose names end in `-trimmed` were cut down after download: the `stations` array was filtered to
latitude 40.4 to 41.7 and longitude -74.3 to -73.6 and re-serialised with two-space indentation; every kept entry
is unmodified and `count` still holds the original total (302, 3502 and 4499).

## Data API: predictions

- `coops-8518750-hilo-2026-11-08.json`: `DG?product=predictions&begin_date=20261108&end_date=20261114&datum=MLLW&station=8518750&interval=hilo&COMMON` (2026-11-08 00:00 to 2026-11-14 23:59 UTC, 27 high/low events, metres above MLLW)
- `coops-8518750-predictions-6min-2026-11-08.json`: `DG?product=predictions&begin_date=20261108&end_date=20261114&datum=MLLW&station=8518750&interval=6&COMMON` (2026-11-08 00:00 to 2026-11-14 23:54 UTC, 1680 points, metres above MLLW; the same week as the hilo file)
- `coops-8518919-hilo-2026-11-08.json`: `DG?product=predictions&begin_date=20261108&end_date=20261114&datum=MLLW&station=8518919&interval=hilo&COMMON` (Tarrytown, a subordinate tide station; same week)
- `coops-NYH1927-bin13-max-slack-2026-11-08.json`: `DG?product=currents_predictions&begin_date=20261108&end_date=20261114&station=NYH1927&bin=13&interval=max_slack&COMMON` (harmonic current station, same week)
- `coops-NYH1927-bin13-currents-predictions-6min-2026-11-08.json`: `DG?product=currents_predictions&begin_date=20261108&end_date=20261114&station=NYH1927&bin=13&interval=6&COMMON` (same week, 6-minute, default `Velocity_Major` shape)
- `coops-NYH1927-bin13-speed-dir-6min-2026-11-08.json`: `DG?product=currents_predictions&begin_date=20261108&end_date=20261108&station=NYH1927&bin=13&interval=6&vel_type=speed_dir&COMMON` (one day, `Speed`/`Direction` shape)
- `coops-ACT3646-bin1-max-slack-2026-11-08.json`: `DG?product=currents_predictions&begin_date=20261108&end_date=20261114&station=ACT3646&bin=1&interval=max_slack&COMMON` (subordinate current station, same week)

## Data API: observations

- `coops-8518750-water-level-2026-11-07.json`: `DG?product=water_level&datum=MLLW&begin_date=20261107&end_date=20261107&station=8518750&COMMON` (preliminary, 240 points)
- `coops-8518750-water-level-latest.json`: `DG?product=water_level&datum=MLLW&date=latest&station=8518750&COMMON` (one point, recorded 2026-11-08 14:2x UTC)
- `coops-8518750-hourly-height-2020-01-01-verified.json`: `DG?product=hourly_height&datum=MLLW&begin_date=20200101&end_date=20200102&station=8518750&COMMON` (verified data, no `q` field)
- `coops-8518750-water-temperature-2026-11-07.json`: `DG?product=water_temperature&begin_date=20261107&end_date=20261107&station=8518750&COMMON`
- `coops-8518750-air-temperature-2026-11-07.json`: `DG?product=air_temperature&begin_date=20261107&end_date=20261107&station=8518750&COMMON`
- `coops-8518750-air-pressure-2026-11-07.json`: `DG?product=air_pressure&begin_date=20261107&end_date=20261107&station=8518750&COMMON`
- `coops-8531680-wind-2026-11-07.json`: `DG?product=wind&begin_date=20261107&end_date=20261107&station=8531680&COMMON` (Sandy Hook; The Battery has no wind sensor)
- `coops-n06010-currents-2026-11-07.json`: `DG?product=currents&begin_date=20261107&end_date=20261107&station=n06010&COMMON` (real-time PORTS current meter, Kill Van Kull)

## Data API: error shapes

- `coops-8518750-wind-2026-11-07-error-nodata.json`: `DG?product=wind&begin_date=20261107&end_date=20261107&station=8518750&COMMON` (HTTP 200 with an `error` object: no wind sensor)
- `coops-error-nodata-currents-n03020.json`: `DG?product=currents&begin_date=20261107&end_date=20261107&station=n03020&bin=9&COMMON` (HTTP 200 with an `error` object)
- `coops-error-subordinate-interval-6.json`: `DG?product=predictions&begin_date=20261108&end_date=20261109&datum=MLLW&station=8518919&interval=6&COMMON` (HTTP 200 with an `error` object: subordinate stations only have `hilo`)
- `coops-error-no-datum.json`: `DG?product=predictions&begin_date=20261108&end_date=20261109&station=8518750&interval=hilo&COMMON` (HTTP 400, datum omitted)
- `coops-error-end-before-begin.json`: `DG?product=predictions&begin_date=20261110&end_date=20261108&datum=MLLW&station=8518750&interval=hilo&COMMON` (HTTP 400)
- `coops-error-no-product.txt`: `DG?begin_date=20261108&end_date=20261109&station=8518750&COMMON` (HTTP 400, plain text although `format=json` was sent)
- `coops-error-range-limit-hilo.json`: `DG?product=predictions&begin_date=20270101&end_date=20371231&datum=MLLW&station=8518750&interval=hilo&COMMON` (HTTP 400, range limit message)
- `coops-error-bad-bin.json`: `DG?product=currents_predictions&begin_date=20261108&end_date=20261108&station=NYH1927&bin=99&interval=max_slack&COMMON` (HTTP 400, lists the valid bins)

## Metadata API

- `coops-mdapi-8518750-harcon.json`: `MD/stations/8518750/harcon.json` (default units: feet; 37 constituents)
- `coops-mdapi-8518750-harcon-metric.json`: `MD/stations/8518750/harcon.json?units=metric` (amplitudes in metres)
- `coops-mdapi-8518750-datums.json`: `MD/stations/8518750/datums.json` (feet)
- `coops-mdapi-8518750-datums-metric.json`: `MD/stations/8518750/datums.json?units=metric` (metres)
- `coops-mdapi-8518750-station.json`: `MD/stations/8518750.json?expand=details,sensors,products`
- `coops-mdapi-8518919-station.json`: `MD/stations/8518919.json?expand=tidepredoffsets,details` (Tarrytown, subordinate)
- `coops-mdapi-8518919-tidepredoffsets.json`: `MD/stations/8518919/tidepredoffsets.json`
- `coops-mdapi-8518919-harcon.json`: `MD/stations/8518919/harcon.json` (empty list: subordinate stations have none)
- `coops-mdapi-8518919-datums.json`: `MD/stations/8518919/datums.json` (all fields null)
- `coops-mdapi-NYH1927-station.json`: `MD/stations/NYH1927.json?expand=bins,harcon,deployments`
- `coops-mdapi-NYH1927-harcon.json`: `MD/stations/NYH1927/harcon.json` (all three bins; default units are metres and cm/s for current stations)
- `coops-mdapi-NYH1927-harcon-bin13-english.json`: `MD/stations/NYH1927/harcon.json?units=english&bin=13`
- `coops-mdapi-NYH1927-bins-metric.json`: `MD/stations/NYH1927/bins.json?units=metric`
- `coops-mdapi-ACT3646-station.json`: `MD/stations/ACT3646.json?expand=currentpredictionoffsets` (subordinate current station with its offsets)
- `coops-mdapi-ACT3646_1-currentpredictionoffsets.json`: `MD/stations/ACT3646_1/currentpredictionoffsets.json` (the `_1` suffix is the bin; the unsuffixed path returns 404)
- `coops-mdapi-n03020-bins.json`: `MD/stations/n03020/bins.json`
- `coops-mdapi-stations-currents.json`: `MD/stations.json?type=currents` (85 active current meters, complete)
- `coops-mdapi-stations-waterlevels-newyork-trimmed.json`: `MD/stations.json?type=waterlevels` (trimmed, 3 of 302)
- `coops-mdapi-stations-tidepredictions-newyork-trimmed.json`: `MD/stations.json?type=tidepredictions` (trimmed, 90 of 3502)
- `coops-mdapi-stations-currentpredictions-newyork-trimmed.json`: `MD/stations.json?type=currentpredictions` (trimmed, 195 of 4499)
- `coops-8518750-hilo-2034-06-01.json`: `https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions&application=underbite&time_zone=gmt&units=metric&format=json&begin_date=20340601&end_date=20340607&datum=MLLW&station=8518750&interval=hilo` (2026-11-15; 28 events, seven days eight years after 2026)
- `coops-8518750-predictions-6min-2034-06-01.json`: `https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions&application=underbite&time_zone=gmt&units=metric&format=json&begin_date=20340601&end_date=20340607&datum=MLLW&station=8518750&interval=6` (2026-11-15; 1680 points)
- `coops-n03020-bin7-max-slack-2026-11-08.json`: `https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=currents_predictions&application=underbite&time_zone=gmt&units=metric&format=json&begin_date=20261108&end_date=20261114&station=n03020&bin=7&interval=max_slack` (2026-11-15; 54 events; reference station and bin of ACT3646)
- `coops-mdapi-n03020-harcon-bin7.json`: `https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/n03020/harcon.json?bin=7` (2026-11-15; 29 constituents, metres and cm/s)
- `coops-mdapi-8729840-station.json`: `https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8729840.json` (2026-11-15; Pensacola FL, `tideType` Diurnal)
- `coops-mdapi-8729840-tidepredoffsets.json`: `https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8729840/tidepredoffsets.json` (2026-11-15; `type` `R` (harmonic reference station))
- `coops-mdapi-8729840-harcon-metric.json`: `https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8729840/harcon.json?units=metric` (2026-11-15; 37 constituents, amplitude in metres)
- `coops-mdapi-8729840-datums-metric.json`: `https://api.tidesandcurrents.noaa.gov/mdapi/prod/webapi/stations/8729840/datums.json?units=metric` (2026-11-15)
- `coops-8729840-hilo-2026-11-08.json`: `https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions&application=underbite&time_zone=gmt&units=metric&format=json&begin_date=20261108&end_date=20261114&datum=MLLW&station=8729840&interval=hilo` (2026-11-15; 14 events, one high and one low a day)
- `coops-8729840-predictions-6min-2026-11-08.json`: `https://api.tidesandcurrents.noaa.gov/api/prod/datagetter?product=predictions&application=underbite&time_zone=gmt&units=metric&format=json&begin_date=20261108&end_date=20261114&datum=MLLW&station=8729840&interval=6` (2026-11-15; 1680 points)
