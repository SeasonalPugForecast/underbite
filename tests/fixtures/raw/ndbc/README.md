# NDBC raw fixtures

Raw files from the National Data Buoy Center, recorded on 2026-11-08 between 14:20 and 14:30 UTC. No keys or
credentials are involved. Files named `-headNNN` are the first NNN lines of the original (the newest
observations, because the files are sorted newest first, plus the two header lines); nothing inside the kept lines was
changed. All other files are complete, byte for byte.

- `ndbc-44025.txt`: `https://www.ndbc.noaa.gov/data/realtime2/44025.txt` (standard meteorological, 1702 lines, 2026-09-24 01:10 to 2026-11-08 13:40 UTC; wind, pressure and air temperature columns are all `MM` for the whole window)
- `ndbc-44025.spec`: `https://www.ndbc.noaa.gov/data/realtime2/44025.spec` (detailed wave summary, 1716 lines)
- `ndbc-44025.ocean-404.html`: `https://www.ndbc.noaa.gov/data/realtime2/44025.ocean` (HTTP 404: the station has no oceanographic file; the body is the server's 404 page)
- `ndbc-activestations.xml`: `https://www.ndbc.noaa.gov/activestations.xml` (complete, 1357 stations, `created="2026-11-08T14:25:05UTC"`)
- `ndbc-44065-head300.txt`: `https://www.ndbc.noaa.gov/data/realtime2/44065.txt` (New York Harbor Entrance buoy: wind and pressure present, 10-minute steps; first 300 data lines, 2026-11-06 11:40 to 2026-11-08 13:50 UTC)
- `ndbc-44065-spec-head150.spec`: `https://www.ndbc.noaa.gov/data/realtime2/44065.spec` (first 150 data lines; has compass-point values in `SwD` and `WWD`)
- `ndbc-44069-ocean-head300.ocean`: `https://www.ndbc.noaa.gov/data/realtime2/44069.ocean` (Great South Bay buoy, the nearest station with a `.ocean` file; first 300 data lines)
- `ndbc-44091-adcp-head100.adcp`: `https://www.ndbc.noaa.gov/data/realtime2/44091.adcp` (Barnegat buoy, current profile; first 100 data lines)
- `ndbc-BATN6-head300.txt`: `https://www.ndbc.noaa.gov/data/realtime2/BATN6.txt` (C-MAN style file for The Battery, upper-case file name, 6-minute steps; first 300 data lines)
