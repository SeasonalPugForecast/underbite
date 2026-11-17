# Raw fixtures: regulations and licence links

One line per file: file name, how it was made, recording date.

- `us-links.csv` — one row per link (a jurisdiction has one to three rows: freshwater, saltwater, licence;
  NOAA Fisheries regional pages have kind `federal`). Columns: `region` (ISO 3166-2 code, or `NOAA-*`),
  `name`, `agency`, `noaa_region` (NOAA Fisheries region of a coastal jurisdiction, empty otherwise),
  `coastal`, `great_lakes`, `kind`, `url`, `http_status` (final status after redirects; 000 means no
  connection), `final_url` (after redirects), `client` (`default` is the descriptive
  `underbite-scout/0.1 (provider docs check)` User-Agent; `browser-like` is the single retry with browser
  headers), `checked` (date). Each URL was requested once with GET (landing page, body discarded); no
  request carried a key or any personal data. Recorded 2026-11-16 from the agencies' own landing pages.
  This is a link catalogue, not regulation data: no limits or seasons are recorded.
