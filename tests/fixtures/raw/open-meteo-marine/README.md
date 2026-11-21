# Open-Meteo Marine: recorded responses

All files are response bodies exactly as served by `https://marine-api.open-meteo.com/v1/marine`, recorded
on 2026-11-08 with a descriptive `User-Agent` and no API key. Times are UTC unless noted.
`B` is `https://marine-api.open-meteo.com/v1/marine`; `ALL` is the comma-separated list
`wave_height,wave_direction,wave_period,wave_peak_period,wind_wave_height,wind_wave_direction,wind_wave_period,wind_wave_peak_period,swell_wave_height,swell_wave_direction,swell_wave_period,swell_wave_peak_period,secondary_swell_wave_height,secondary_swell_wave_period,secondary_swell_wave_direction,tertiary_swell_wave_height,tertiary_swell_wave_period,tertiary_swell_wave_direction,sea_surface_temperature,ocean_current_velocity,ocean_current_direction,sea_level_height_msl`.

| File | URL | Date |
|---|---|---|
| `marine-40.70_-74.02-hourly-2d.json` | `B?latitude=40.70&longitude=-74.02&hourly=ALL&forecast_days=2&timezone=UTC` (snaps to 40.625, -74.04166) | 2026-11-08 |
| `marine-40.25_-73.16-hourly-2d.json` | `B?latitude=40.25&longitude=-73.16&hourly=ALL&forecast_days=2&timezone=UTC` (snaps to 40.291664, -73.12499) | 2026-11-08 |
| `marine-40.70_-74.02-minutely15-1d.json` | `B?latitude=40.70&longitude=-74.02&minutely_15=ocean_current_velocity,ocean_current_direction,sea_level_height_msl&forecast_days=1&timezone=UTC` | 2026-11-08 |
| `marine-40.25_-73.16-current.json` | `B?latitude=40.25&longitude=-73.16&current=wave_height,wave_period,wave_direction,sea_surface_temperature,ocean_current_velocity,ocean_current_direction,sea_level_height_msl&timezone=UTC` | 2026-11-08 |
| `marine-40.25_-73.16-daily-3d.json` | `B?latitude=40.25&longitude=-73.16&daily=wave_height_max,wave_direction_dominant,wave_period_max,swell_wave_height_max&forecast_days=3&timezone=UTC` | 2026-11-08 |
| `marine-40.25_-73.16-imperial-knots-1d.json` | `B?latitude=40.25&longitude=-73.16&hourly=wave_height,ocean_current_velocity,sea_surface_temperature&forecast_days=1&timezone=UTC&length_unit=imperial&wind_speed_unit=kn&temperature_unit=fahrenheit` | 2026-11-08 |
| `marine-40.25_-73.16-gfswave025-1d.json` | `B?latitude=40.25&longitude=-73.16&hourly=wave_height,wave_period,swell_wave_height,secondary_swell_wave_height,tertiary_swell_wave_height,tertiary_swell_wave_period,tertiary_swell_wave_direction&models=ncep_gfswave025&forecast_days=1&timezone=UTC` (the only model that fills the tertiary train; values are 0 when absent) | 2026-11-08 |
| `marine-40.25_-73.16-history-2024-07-01.json` | `B?latitude=40.25&longitude=-73.16&hourly=wave_height,sea_surface_temperature,ocean_current_velocity,sea_level_height_msl&start_date=2024-07-01&end_date=2024-07-01&timezone=UTC` | 2026-11-08 |
| `marine-40.25_-73.16-history-2021-12-15-nulls.json` | `B?latitude=40.25&longitude=-73.16&hourly=wave_height,sea_surface_temperature,sea_level_height_msl&start_date=2021-12-15&end_date=2021-12-15&timezone=UTC` (before SST and sea level exist: all null) | 2026-11-08 |
| `marine-40.25_-73.16-era5-ocean-1960-07-01.json` | `B?latitude=40.25&longitude=-73.16&hourly=wave_height,wave_period,wave_direction,sea_surface_temperature&models=era5_ocean&start_date=1960-07-01&end_date=1960-07-01&timezone=UTC` (0.5 degree cell 40.5, -73.0; SST all null) | 2026-11-08 |
| `marine-39.0_-98.0-land-hourly-1d.json` | `B?latitude=39.0&longitude=-98.0&hourly=wave_height,sea_surface_temperature,ocean_current_velocity,sea_level_height_msl&forecast_days=1&timezone=UTC` (inland land point: HTTP 200, `elevation` 402, every value null) | 2026-11-08 |
| `marine-42.0_-81.5-lake-erie-hourly-1d.json` | `B?latitude=42.0&longitude=-81.5&hourly=wave_height,wave_period,sea_surface_temperature,ocean_current_velocity,sea_level_height_msl&forecast_days=1&timezone=UTC` (open Lake Erie: waves present, SST, current and sea level null) | 2026-11-08 |
| `marine-error-invalid-variable.json` | `B?latitude=40.70&longitude=-74.02&hourly=bogus&forecast_days=1` (HTTP 400) | 2026-11-08 |
| `marine-error-forecast-days.json` | `B?latitude=40.25&longitude=-73.16&hourly=wave_height&forecast_days=20` (HTTP 400) | 2026-11-08 |

Weather data by Open-Meteo.com (<https://open-meteo.com/>), licensed CC BY 4.0 (<https://open-meteo.com/en/licence>).
Some marine variables are generated using E.U. Copernicus Marine Service Information (https://doi.org/10.48670/moi-00016, https://doi.org/10.48670/moi-00017).
