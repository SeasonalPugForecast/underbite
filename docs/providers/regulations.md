# Regulations and licence links (United States)

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
