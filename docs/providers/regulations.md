# Regulations and licence links

Verified on **2026-11-16**. Regulations are published as pages and yearly booklets, not as a data
service, so the app links to the official pages and never copies limits or seasons. This note records
which pages were chosen, how each answered, and what to expect when a link is opened from a scripted
client. The link list itself is `tests/fixtures/raw/regulations/us-links.csv`, indexed in
`tests/fixtures/raw/regulations/README.md`.

## What was checked

- 56 jurisdictions: the 50 states, the District of Columbia and five territories (Puerto Rico, US Virgin
  Islands, Guam, American Samoa, Northern Mariana Islands). Of these, 28 are coastal (including Alaska,
  Hawaii and the five territories) and 8 border the Great Lakes (IL, IN, MI, MN, NY, OH, PA, WI; New York is both).
- Per jurisdiction up to three links: the freshwater regulations page, the saltwater regulations page
  (coastal jurisdictions only) and the licence page. Where one agency publishes a single page for both,
  the same URL appears under both kinds. The Great Lakes are treated as fresh water; no state has a
  separate Great Lakes page in the list (Michigan, Ohio, New York and others cover them in the general
  fishing regulations).
- NOAA Fisheries regional recreational pages for federal waters: Greater Atlantic, Southeast (hub, South
  Atlantic, Gulf of America), West Coast, Alaska, Pacific Islands and Atlantic Highly Migratory Species.
- 148 rows in total: 140 for jurisdictions and 8 for NOAA Fisheries.

Every URL was requested once with GET (body discarded), following redirects, at least a second apart per
host, with the User-Agent `underbite-scout/0.1 (provider docs check)` and no cookies or keys. URLs that
failed (403, 404) or had no connection got at most one more request with browser-like headers. Some
URLs were first guessed from the agency's site layout, answered 404, and were replaced by pages found on
the agency's own home page or through its search listing; only the final URLs are in the CSV.

## Result summary

| Outcome | Count (links) |
|---|---|
| 200 with the default User-Agent | 127 |
| 200 only with browser-like headers | 10 |
| 403 with both clients (blocked) | 9 |
| 404 with both clients | 2 |

(Counts are of CSV rows; a URL used for both fresh and salt appears twice.)

### Links that failed

| Jurisdiction | Kind | Status | URL (all https) |
|---|---|---|---|
| US-KS | freshwater, licence | 403 | `www.ksoutdoors.gov/Fishing/Fishing-Regulations`; `www.ksoutdoors.gov/licenses-permits-fees/fishing-licenses-permit-fees` |
| US-MA | freshwater, saltwater, licence | 403 | `www.mass.gov/info-details/freshwater-fishing-regulations`; `.../recreational-saltwater-fishing-regulations`; `www.mass.gov/how-to/buy-your-freshwater-fishing-license` |
| US-NE | licence | 403 | `outdoornebraska.gov/` (the fishing page `outdoornebraska.gov/fish/` answers 200 to browser-like headers) |
| US-NH | freshwater, saltwater, licence | 403 | `www.wildlife.nh.gov/fishing-new-hampshire`; `.../saltwater-fisheries-new-hampshire/recreational-saltwater-fishing`; `.../licenses-and-permits/nh-license-requirements` |
| US-OH | freshwater, licence | 404 | `ohiodnr.gov/buy-and-apply/hunting-fishing-boating/fishing-resources/fishing-regulations`; `.../fishing-licenses` |

Notes on these:

- The 403 hosts (`mass.gov`, `wildlife.nh.gov`, `ksoutdoors.gov`, `outdoornebraska.gov` for the home page)
  refuse scripted clients even with browser-like headers. The URLs come from the agencies' own search
  listings and are **unverified** as to their content; open them in a browser or in the in-app web view
  before shipping them. Other hosts that returned 403 to the default User-Agent (`maine.gov`,
  `tn.gov`, `wlf.louisiana.gov`, `outdoornebraska.gov/fish/`, `mrc.virginia.gov`) answered 200 with
  browser-like headers.
- `ohiodnr.gov` answered 404 to every scripted request, including the site root, so no page of that host
  could be confirmed. The two URLs are the ones the agency's pages list for the regulations booklet and
  the licence page. They are **unverified**.
- Four hosts guessed during the survey did not resolve at all (no DNS record) and were replaced:
  `dawr.guam.gov` (Guam's division is under `doag.guam.gov`), `dfw.gov.mp` (the Northern Mariana Islands
  division is under `dlnr.cnmi.gov`), `fishwildlife.vermont.gov` (now `vtfishandwildlife.com`) and
  `www.wildlife.nm.gov` (now `wildlife.dgf.nm.gov`).

### Links that redirected

No link redirected to a domain that is not the agency's own, with one exception: Illinois
`dnr.illinois.gov/fishing.html` redirects to `ifishillinois.org`, a site run by the Illinois Department
of Natural Resources for its fisheries pages. Other redirects stayed on the same host and only changed
the path or the trailing slash (recorded in `final_url`); the notable ones:

- `gfp.sd.gov/fishing/` redirects to the site root `gfp.sd.gov/`, so the South Dakota freshwater link is
  a landing page, not a fishing page.
- `www.azgfd.com/fishing/regulations/` redirected to `/fishing-2/licenses-and-regulations/`; Arizona uses
  that one page for both regulations and licence.
- `idfg.idaho.gov/fish/rules` redirects to `/rules/fish`; `cpw.state.co.us/activities/fishing-licenses-and-dates`
  to `/activities/fishing/fishing-licenses-and-dates`; `fw.ky.gov/Licenses` to `/Licenses/Pages/default.aspx`;
  `tpwd.texas.gov/licenses/` to `/business/licenses/online_sales/`.
- The NOAA Pacific Islands page moved from `/pacific-islands/recreational-fishing/...` to
  `/pacific-islands/resources-fishing/...`; the CSV keeps the requested URL and records the final one.
- `www.ndow.org/get-outside/rules-regulations/` redirected to the boating rules page and was replaced by
  the Nevada fishing trip page.

## Jurisdictions where one page serves several kinds

- Puerto Rico, US Virgin Islands, Guam, American Samoa and the Northern Mariana Islands: no dedicated
  recreational regulations or licence page was located, so all three kinds point to one agency page
  (the agency home page, or for Guam and the Northern Mariana Islands the division page). They are
  links to an agency, not to rules. American Samoa's federal rules are on the NOAA Pacific Islands
  page; its territorial rules were not located.
- Alabama, Alaska, Delaware, Hawaii, Maryland, Oregon, Rhode Island, Texas, Washington: freshwater and
  saltwater point to one combined agency page, and for several of them the licence kind is the same
  page too (Alabama and Alaska differ only in the licence page). Maryland's three links are the
  Fisheries Service landing page.
- Arizona, Vermont, West Virginia: the licence link is the same page as the freshwater link.
- Hawaii has no general saltwater recreational licence; its licence link is the Division of Aquatic
  Resources licences page.
- Nebraska licence: the site root, because the licence page could not be reached (403).
- Mississippi saltwater: the Department of Marine Resources home page `dmr.ms.gov`, not a regulations
  page.
- Georgia saltwater: the Coastal Resources Division home page `coastalgadnr.org`.

## NOAA Fisheries regions

Federal waters (3 to 200 nautical miles, 9 for Texas and the Gulf coast of Florida) are covered by the
NOAA Fisheries regional pages. Mapping used in the `noaa_region` column:

| Region | Jurisdictions |
|---|---|
| greater-atlantic | ME, NH, MA, RI, CT, NY, NJ, DE, MD, VA |
| southeast | NC, SC, GA, FL, AL, MS, LA, TX, PR, VI |
| west-coast | CA, OR, WA |
| alaska | AK |
| pacific-islands | HI, GU, AS, MP |

North Carolina straddles the Greater Atlantic and Southeast regions (the boundary is at Cape Hatteras, NC);
it is mapped to Southeast, the region whose South Atlantic page covers fishing off most of the state, and
Greater Atlantic rules for some species apply north of the cape. Pennsylvania, the District of Columbia and
the other inland states have no NOAA region. Atlantic highly migratory species (tunas, sharks, billfish)
have their own page and permit rules in all Atlantic and Gulf states; it is listed as `NOAA-HMS`.
The NOAA recreational hub page does not list a West Coast regulations page; the West Coast page used is
`west-coast/recreational-fishing/recreational-fisheries-west-coast`. The Southeast hub page links to
Gulf Council and South Atlantic Council pages on non-NOAA domains (`gulfcouncil.org`, `safmc.net`);
these are not in the CSV, which keeps to NOAA's own domain.

## Yearly PDFs and year-bound URLs

Many agencies publish the regulations as a yearly booklet in addition to a web page. Pages that were
confirmed to exist all year are linked; URLs containing a year were avoided. Ohio's regulations page is a
pointer to the yearly booklet (PDF); Louisiana's pages sit beside a yearly PDF; Kansas publishes a
yearly summary PDF as well. No state was found that publishes only a PDF, but page bodies were not read
(only home pages were scanned for links), and for the blocked hosts (MA, NH, KS, NE, OH) page content is
**unverified**. No URL in the CSV contains a year.

## Terms of use

- No agency page checked offered an API, and none was used. Linking to public agency pages needs no
  licence or attribution; the app credits each agency by name in the link label and shows the check date.
- A few sites (ksoutdoors.gov, mass.gov, wildlife.nh.gov, outdoornebraska.gov, ohiodnr.gov) refuse
  scripted clients. Scripted checks of those links should be done sparingly or by hand, and the app must
  open them in a browser view, not fetch them.
- Fees, seasons and limits change during the year (several agencies post in-season changes and
  emergency rules); the app text must say regulations may have changed and must link, not quote.

## Unverified

- Page content for the 403 and 404 links listed above.
- Whether each landing page is the best current entry point (agencies reorganise their sites yearly);
  only the HTTP status and final URL were observed, and page bodies were not read except to find links
  on agency home pages.
- Territorial regulations (PR, VI, GU, AS, MP) beyond the agency landing pages.
- The recorded status reflects 2026-11-16 only; the catalogue needs a recheck at least once a year.

# Canada

Verified on **2026-11-17**. As for the United States, regulations are pages and yearly guides, not a data
service, so the app links to the official pages and never copies limits, seasons or species rules. The link
list is `tests/fixtures/raw/regulations/ca-links.csv` (51 rows), indexed in
`tests/fixtures/raw/regulations/README.md`.

## Who regulates what

- Freshwater angling is managed by each province or territory, in several of them under federal fishery
  regulations that the province administers (the Maritime Provinces, Newfoundland and Labrador, Yukon and
  Northwest Territories fishery regulations). Nunavut is covered by the Northwest Territories Fishery
  Regulations; Fisheries and Oceans Canada (DFO) is consulting on separate Nunavut regulations (DFO search
  summary, 2026-11-17).
- Tidal waters and the sea are federal. DFO publishes a recreational fishing page per region: Pacific,
  Maritimes, Gulf, Quebec, Newfoundland and Labrador, and Arctic. A search summary of DFO's Maritimes pages says
  no licence is needed for angling in tidal waters of the Maritime provinces while a provincial licence is
  needed inland (**unverified on the page itself**; page bodies were not read).
- In Newfoundland and Labrador, DFO publishes the Anglers' Guide for inland waters. The CSV uses the DFO guide
  page as the regulations link and the province's angling licence page as the licence link.
- National parks have their own Parks Canada fishing licence, and provincial and territorial licences are not
  valid there (stated in several provincial guides). No Parks Canada link was checked.

## What was checked

- 39 rows for the 10 provinces and 3 territories: a regulations link and a licence link each, with an English
  and a French row where the agency has both; French rows exist for MB, ON, QC, NB, PE, NL, YT, NT and NU.
- 12 rows for DFO's six regions, English and French.
- Every URL was requested once with GET, following redirects, at least a second apart per host, with the
  User-Agent `underbite-scout/0.1 (provider docs check)` and no cookies or keys. No banner or consent form was
  accepted and no browser-like retry was made. Guessed URLs that answered 404 were replaced by pages found on
  the agencies' own sites or in their search listings; only the final choices are in the CSV.

## Result summary

| Outcome | Rows |
|---|---|
| 200 | 45 |
| 403 (Yukon and Nunavut, all six rows) | 6 |

`www.gov.nt.ca` is slow and occasionally fails: the English sport fishing page returned 504 and then timed out
once at 25 s, and answered 200 on a repeat with a 40 s timeout, which is the recorded result. The app should
treat that host as flaky.

## Links by jurisdiction

Paths are abbreviated; the CSV has every full URL.

| Code | Regulations (en) | Licence (en) | French |
|---|---|---|---|
| CA-BC | `www2.gov.bc.ca/gov/content/sports-culture/recreation/fishing-hunting/fishing/fishing-regulations` | `.../fishing/recreational-freshwater-fishing-licence` | none found |
| CA-AB | `albertaregulations.ca/fishingregs/` | `mywildalberta.ca/buy-licences/fishing-licenses-fees` | none found |
| CA-SK | `saskatchewan.ca/residents/parks-culture-heritage-and-sport/hunting-trapping-and-angling/angling` | `.../angling/angling-licences` | none found |
| CA-MB | `gov.mb.ca/nrnd/fish-wildlife/fish/recreational_fishing/index.html` | `www.manitobaelicensing.ca` | regulations: `.../index.fr.html` |
| CA-ON | `ontario.ca/document/ontario-fishing-regulations-summary` | `.../recreational-fishing-licences-and-fees` | both: `ontario.ca/fr/document/resume-des-reglements-de-la-peche`, `.../permis-de-peche-recreative-et-frais` |
| CA-QC | `quebec.ca/en/tourism-recreation-sport/sporting-and-outdoor-activities/sport-fishing` | `.../sport-fishing/fishing-licence` | both: `quebec.ca/tourisme-loisirs-sport/activites-sportives-et-de-plein-air/peche-sportive`, `.../peche-sportive/permis` |
| CA-NB | `gnb.ca/en/topic/tourism-recreation/hunting-fishing.html` | `www2.gnb.ca/content/gnb/en/services/services_renderer.1115.Fishing_Licence.html` | both: `gnb.ca/fr/sujet/tourisme-loisirs/chasse-peche.html`, `www2.gnb.ca/content/gnb/fr/services/services_renderer.1115.Fishing_Licence.html` |
| CA-NS | `novascotia.ca/fish/laws-and-regulations/` | `novascotia.ca/programs-and-services/sportfishing-licensing` | none found |
| CA-PE | `princeedwardisland.ca/en/information/communities-land-and-environment/angling-resources-and-information-centre` | `.../en/service/online-fishing-licences` | both: `.../fr/information/terres-et-environnement/guide-de-la-peche-sportive`, `.../fr/service/permis-de-peche-en-ligne` |
| CA-NL | `nfl.dfo-mpo.gc.ca/en/NL/AG/Anglersguide` (DFO) | `gov.nl.ca/gs/licences/hunting/angling/` | regulations (DFO): `nfl.dfo-mpo.gc.ca/fr/TNL/GPL/Guidedupecheuralaligne` |
| CA-YT | `yukon.ca/en/fishing-regulations` (403) | `yukon.ca/en/yukon-fishing-licence` (403) | `yukon.ca/fr/reglements-de-peche` (403) |
| CA-NT | `gov.nt.ca/ecc/en/services/sport-fishing` | `gov.nt.ca/ecc/en/services/get-fishing-licence` | regulations: `gov.nt.ca/ecc/fr/services/peche-sportive` |
| CA-NU | `gov.nu.ca/en/environment` (403) | `gov.nu.ca/how-do-i-get-fishing-license` (403) | `gov.nu.ca/fr/environnement` (403) |

Where a jurisdiction has no French licence row, the French regulations row (if any) is the only French link.
The French rows were checked for status only; whether they mirror the English pages is **unverified**.

## DFO tidal regions

| Code | Region | English page (French: replace `eng` with `fra`) |
|---|---|---|
| DFO-PAC | Pacific | `www.pac.dfo-mpo.gc.ca/fm-gp/rec/index-eng.html` |
| DFO-MAR | Maritimes | `www.dfo-mpo.gc.ca/fisheries-peches/recreational-recreative/maritimes/index-eng.html` |
| DFO-GLF | Gulf | `.../recreational-recreative/gulf/index-eng.html` |
| DFO-QC | Quebec | `.../recreational-recreative/quebec/index-eng.html` |
| DFO-NL | Newfoundland and Labrador | `.../recreational-recreative/newfoundland-labrador/index-eng.html` |
| DFO-ARC | Arctic | `.../recreational-recreative/arctic-arctique/index-eng.html` |

All twelve returned 200 without a redirect. The Pacific page is on its own subdomain. Which region applies to a
point of tidal water is a mapping the app must make; DFO's region descriptions (as read in search summaries)
put the Northumberland Strait and the Gulf of St. Lawrence coasts of New Brunswick and Prince Edward Island in
the Gulf region, the Atlantic coast of Nova Scotia and the Bay of Fundy in the Maritimes, the St. Lawrence
estuary in Quebec, Hudson Bay, James Bay and the northern territories in the Arctic. That mapping is
**unverified** against a boundary document. The Great Lakes have no DFO recreational page in the list and are
treated as fresh water under the provincial page.

## Links that failed

| Jurisdiction | Kind | Status | URL |
|---|---|---|---|
| CA-YT | regulations (en, fr), licence | 403 | `yukon.ca/en/fishing-regulations`, `yukon.ca/fr/reglements-de-peche`, `yukon.ca/en/yukon-fishing-licence` |
| CA-NU | regulations (en, fr), licence | 403 | `gov.nu.ca/en/environment`, `gov.nu.ca/fr/environnement`, `gov.nu.ca/how-do-i-get-fishing-license` |

Both hosts refused the descriptive User-Agent, including their home pages (`yukon.ca/en`, `www.gov.nu.ca/`). The
URLs come from the agencies' own search listings and are **unverified** as to their content. Whether a
browser-like client is served is **unverified**: none was tried. The app should open these in a browser view
and not fetch them. For Nunavut only yearly guide PDFs were found in search results (2021-22 the newest), so
the Department of Environment landing page is used.

## Links that redirected

- `mywildalberta.ca/buy-licences/fishing-licenses-fees` redirects to `.../default.aspx` on the same host.
- `manitobaelicensing.ca` without `www` fails TLS name verification (the certificate does not match the host);
  `www.manitobaelicensing.ca` redirects to `/public`. The CSV uses the `www` host. Manitoba's licences are sold
  on this separate e-licensing site.
- `ontario.ca/fr/document/resume-des-reglements-de-la-peche-en-ontario` redirects to
  `/fr/document/resume-des-reglements-de-la-peche`, and Ontario appends `redirect_id` and `redirect_year`
  parameters to moved pages; the CSV holds the clean current path, which answered 200.
- `gov.nu.ca` redirects to `www.gov.nu.ca` (the status stays 403).
- `princeedwardisland.ca/angling`, a short link, redirects to the resource centre page; the full path is used.

## Pages that did not exist

Several guessed paths answered 404 and were replaced; they show how agencies reorganise: BC's
`outdoor-recreation/fishing-and-hunting/...` paths (the live path is `sports-culture/recreation/fishing-hunting/...`);
Alberta's `alberta.ca` fishing pages (licences are on `mywildalberta.ca` and `eservices.alberta.ca`); Ontario's
`/page/fishing-ontario`; Quebec's `tourism-and-recreation/...` English paths (now
`tourism-recreation-sport/sporting-and-outdoor-activities/...`); Newfoundland and Labrador's `gov.nl.ca/ecc/...`
(the department's path segment changes with each reorganisation: `fal`, `ffa` and `gs` were all seen);
Nova Scotia has no French pages for these topics. **A 200 does not prove a page exists**: `gov.mb.ca/fish-wildlife/fishing/index.html`
(a guessed path) answered 200 after redirecting to `manitoba.ca/404.html`, the province's own not-found page. Every
final URL in the CSV was compared with its request to catch this.

## Pages that serve more than one purpose

- Nunavut: the regulations link is the Department of Environment landing page, not a regulations page.
- New Brunswick: the regulations link is the hunting and fishing topic hub; the yearly regulations summary is
  linked from it but not recorded because it is a yearly document.
- Newfoundland and Labrador: the regulations link is DFO's guide, the licence link is the province's page.

## Yearly PDFs and year-bound URLs

Alberta, Saskatchewan, Ontario, New Brunswick, Northwest Territories and Yukon publish yearly guides; Quebec and
BC publish two-year guides (Quebec changes on 1 April of even-numbered years); Manitoba publishes an anglers'
guide as a PDF. The CSV links the stable landing page, not the PDF. The NWT page `/ecc/en/content/nwt-sport-fishing-guide`
redirects directly to a PDF whose name carries the year and is not used. No URL in the CSV contains a year.

## Language

English only (none found): BC, AB, SK, NS. French rows exist for MB, ON, QC, NB, PE, NL, NT, YT and NU (YT and NU
unverified). Quebec's pages are French first; its English pages are translations. The app should choose the link
language from its own language setting and fall back to English when a jurisdiction has no French row.

## Terms of use

- No page offered an API and none was used. Linking to public agency pages needs no licence or attribution; the
  app credits each agency by name in the link label and shows the check date. Agency copyright notices were not
  read page by page and no page content was copied.
- Provincial guides describe themselves as summaries, not legal documents; the app text must say regulations
  may have changed and link, not quote.
- Closures and rule changes happen during the season (for example short-notice stream closures in Alberta and the
  rules that took effect in New Brunswick in March 2026, as announced on gnb.ca).

## Unverified

- Page content for every link: only HTTP status and final URL were observed.
- The Yukon and Nunavut links (403 to the descriptive User-Agent; no other client tried).
- That the French pages mirror the English ones.
- The Maritimes tidal-licence statement and the region boundaries above.
- Whether each landing page is the best entry point; the status reflects 2026-11-17 only, and the catalogue needs a
  recheck at least once a year.
