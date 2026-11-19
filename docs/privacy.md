# Privacy

Underbite has no accounts, no server of its own and no analytics. Nothing in it identifies you, and
nothing about you is collected by the people who make it. The app shows the same text on its Privacy
screen, with the rounding you have chosen.

## What stays on your device

- Your spots, trips, catches, photos, notes, alert rules and settings. They live in the app's own
  storage on the phone and are not uploaded anywhere.
- Where you are. The phone's location is used only on the phone, to centre the map, to start a new
  spot or to search near you. Location access is optional, and asking for it once does not
  keep it running.
- The spots' exact coordinates. Stations are chosen on the phone from lists fetched for the
  rounded place.
- Optional keys, such as a USGS API key, are kept in the iOS Keychain. They are never written to
  settings, logs or exports.

## What leaves the device

The app contacts public data services to get tides, currents, river flow, water temperature,
weather, waves, charts, access, stocking and species information. They are named on the Sources
screen, with their licences. To ask them about a spot, the app sends:

- **The spot's place, rounded.** Latitude and longitude are rounded before they leave the phone to
  the number of decimals you choose under Settings, Privacy: two (about 1 km, the default and the
  finest allowed), one (about 10 km) or none (whole degrees). A coarser place may pick a station
  farther from the water you fish. Station requests use the station's own identifier, which is public.
- **The area on the map.** Map layers, depth shading, access areas and nearby species ask the
  services for the part of the world on screen, by tile, by grid block or by box. These are areas
  you are looking at, not a spot you saved.
- **Place search.** The words you type in the new-spot search go to Apple Maps, with an
  approximate area around you when your location is known. The app keeps no list of past searches.

The services see your IP address, as any website does. The app sends an identifier of itself and its
version, `underbite/<version>`, and no device or advertising identifier. Requests are never logged
with keys. The app does not track you, does not use or ask for an advertising identifier, and has no
third-party code that collects data.

## What you choose to send

- **Shares.** When you share a spot or a trip, the file or link holds what you chose to share,
  including its exact place unless you round it or share only the area. You send it, to whom you
  choose, by the iOS share sheet.
- **iCloud.** Sharing through iCloud happens only when you share. Underbite does not sync your
  data to iCloud on its own.
- **Exports.** A catch export (CSV or GPX) holds exact locations, since a log of where you caught
  fish is of little use otherwise. Catch photos are saved without their location and camera
  metadata, so a shared photo carries none.

## Apple's privacy manifest

The app and the widget extension declare the data a privacy report asks for: coarse location (the
rounded place) and search history (the place search text), both for app functionality, not linked to
you and not used for tracking. They also declare the system calls Apple asks about: user defaults
for the app's own settings and the colour theme shared with the widget, and file timestamps in the
app's own container.

## Questions

Open an issue in the project's repository. The source is public, so what is written here can be
checked against the code.
