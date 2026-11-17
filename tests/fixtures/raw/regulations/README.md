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
- `ca-links.csv` — Canadian links, one row per link: 10 provinces and 3 territories (regulations, licence, and a
  French row where a French page exists) and the six Fisheries and Oceans Canada (DFO) regional recreational
  pages for tidal waters (`DFO-PAC`, `DFO-MAR`, `DFO-GLF`, `DFO-QC`, `DFO-NL`, `DFO-ARC`; kind `tidal`, English and
  French). Columns: `region` (ISO 3166-2 code or `DFO-*`), `name`, `agency`, `kind` (`regulations`, `licence`,
  `tidal`), `language` (`en`, `fr`), `url`, `http_status` (final status after redirects; 000 means no connection),
  `final_url` (after redirects), `checked` (date). Each URL was requested once with GET (body discarded) with the
  `underbite-scout/0.1 (provider docs check)` User-Agent, no cookies and no keys. Recorded 2026-11-17 from the
  agencies' own pages and their search listings. Links only: no limits or seasons are recorded.
