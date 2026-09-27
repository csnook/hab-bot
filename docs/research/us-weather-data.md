# US weather data for severe-weather triggers

Research for [csnook/hab-bot#4](https://github.com/csnook/hab-bot/issues/4): which data sources can supply US weather for reminders, and on what terms.

- **Read on:** 2026-09-27. Versions and dates of what was read are given per source below.
- **Terminology:** in `CONTEXT.md`, an **alert** is how an *occurrence* gets someone's attention. Weather providers use "alert" for something else: a watch, warning or advisory issued by a weather agency. To avoid the clash, this document calls those **warning messages** (or CAP messages), except when quoting an endpoint or field name such as `/alerts` or `WeatherAlert`.
- **Unverified claims:** this environment's proxy blocked `weather.gov`, `api.weather.gov`, `vlab.noaa.gov`, `gml.noaa.gov`, `fema.gov`, `open-meteo.com` and others (full list in [Access notes](#access-notes)). Where a claim rests only on a search-engine excerpt of a blocked page, it is marked **[unverified: search excerpt]**. Where it rests on a community post rather than on NWS staff, it says so.

## Summary

- **The NWS API (`api.weather.gov`) is the primary US source for warning messages** such as Severe Thunderstorm Warnings, and for forecasts. It is free and needs no key: it asks for a `User-Agent` header that identifies the app and gives a contact. Its data is public domain. The rate limit is not published. Warning messages follow the CAP v1.2 fields (event, severity, urgency, onset, expires, and the message type Alert, Update or Cancel), and the API can filter them by point, zone or state.
- **The NWS API can only be polled; it has no push.** NWS's own new website polls `/alerts/active` every 30 seconds, and the endpoint's cache lifetime is reported to be 30 seconds. No published figure says how long a warning takes to reach the API after it is issued. NWS does push warnings through NWWS-OI (XMPP, account by request) and FEMA offers the IPAWS feed, but neither could be verified here. The open-source FOSS Public Alert Server polls the NWS feed every 60 seconds by default and pushes over UnifiedPush, but it describes itself as not ready for production.
- **Open-Meteo has forecasts but no warning messages.** It is free for non-commercial use (600 calls a minute, 5,000 an hour, 10,000 a day), needs no key, licenses its data CC BY 4.0, and can be self-hosted (AGPLv3). It includes daily sunrise and sunset. Its thunderstorm weather codes are derived from models; they are not official warnings.
- **Other sources:** Pirate Weather (free key, 10,000 calls a month) passes NWS warning messages through, polling them every 30 minutes in its self-hosting setup. Apple WeatherKit REST needs a paid Apple Developer membership and a token signed on a server, and has strict display rules for warnings. No other provider could be checked here.
- **Sunrise and sunset need no network.** They can be computed on the device with Meeus/NOAA-style algorithms, for example the Rust `sunrise` crate or the JS `suncalc` library, and this works the same in every region. The NWS API does not provide them.
- **A region's weather module** would need to expose at least: whether it covers a location; the active warning messages for that location, normalized to CAP fields and with their update and cancel lifecycle; a mapping from the region's event names or codes to kinds the app understands; forecast values; a polling or push schedule; and its terms (attribution, key, limits).

## Access notes

| Source | How it was read | Version / date |
|---|---|---|
| NWS API documentation (GitHub Pages source) | `raw.githubusercontent.com/weather-gov/api/master/*.md` | read 2026-09-27; files undated |
| NWS OpenAPI spec | Official copy committed to [weather-gov/api `assets/openapi.yaml`](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml) | **v1.9.0, committed 2021-09-22** ([history](https://github.com/weather-gov/api/commits/master/assets/openapi.yaml)). The live spec at `https://api.weather.gov/openapi.json` was blocked. The live API is at 3.7 ([release notes, 2026-03-19](https://github.com/weather-gov/api/discussions/855)), so details below come from 1.9 plus the later release notes. |
| NWS API release notes and Q&A | [weather-gov/api Discussions](https://github.com/weather-gov/api/discussions), via WebFetch | posts dated per citation |
| weather.gov 2.0 source code (NWS's new site) | [weather-gov/weather.gov](https://github.com/weather-gov/weather.gov), `main` branch | read 2026-09-27 |
| www.weather.gov documentation pages | **Blocked**; only search excerpts seen | — |
| Open-Meteo terms, pricing and docs | Page sources in [open-meteo/open-meteo-website](https://github.com/open-meteo/open-meteo-website), `main` | terms page last changed 2026-07-16 ([history](https://github.com/open-meteo/open-meteo-website/commits/main/src/routes/en/terms/%2Bpage.svelte)) |
| Open-Meteo README | [open-meteo/open-meteo](https://github.com/open-meteo/open-meteo/blob/main/README.md) | read 2026-09-27 |
| Pirate Weather docs and code | [Pirate-Weather/pirateweather](https://github.com/Pirate-Weather/pirateweather) docs, [pirate-weather-code](https://github.com/Pirate-Weather/pirate-weather-code) | API v2.10.2 (changelog 2026-09-23) |
| Apple WeatherKit | developer.apple.com (reachable) | read 2026-09-27 |
| FOSS Public Alert Server | GitHub mirror [KDE/foss-public-alert-server](https://github.com/KDE/foss-public-alert-server) (invent.kde.org blocked) | `master`, read 2026-09-27; API v1.0.0 "early testing" |

Blocked hosts: `api.weather.gov`, `www.weather.gov`, `weather-gov.github.io`, `vlab.noaa.gov`, `gml.noaa.gov`, `spc.noaa.gov`, `mapservices.weather.noaa.gov`, `tgftp.nws.noaa.gov`, `nwws-oi.weather.gov`, `fema.gov`, `docs.oasis-open.org`, `open-meteo.com`, `api.open-meteo.com`, `docs.pirateweather.net`, `pirate-weather.apiable.io`, `openweathermap.org`, `tomorrow.io`, `api.met.no`, `invent.kde.org`, `alerts.kde.org`, `aa.usno.navy.mil`, `web.archive.org`. No live API responses could be fetched, so nothing below was checked against live data.

## 1. National Weather Service API (`api.weather.gov`)

### Terms, keys and rate limits

- **No key; a `User-Agent` is required.** Requests without a `User-Agent` get 403. NWS recommends "something that identifies your application and includes a contact email", and says "In the future we will replace the User-Agent requirement with a more typical API key system." ([General FAQs](https://github.com/weather-gov/api/blob/master/general-faqs.md)). The OpenAPI security scheme adds: "The API remains open and free to use and there are no limits imposed based on the User-Agent string" ([openapi.yaml v1.9](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml), `securitySchemes.userAgent`). The suggested format is `(myweatherapp.com, contact@myweatherapp.com)` **[unverified: search excerpt of https://www.weather.gov/documentation/services-web-api]**.
- **An API key is coming.** API 3.3 (2025-11-27): "API-Key has been added to the security schemes in OpenAPI. We are not ready to flip that switch yet (there will be plenty of warning when we do)." ([Discussion #846](https://github.com/weather-gov/api/discussions/846))
- **The rate limit is not published.** The documentation page reportedly says "The rate limit is not public information, but allows a generous amount for typical use. If the rate limit is exceeded a request will return with an error, and may be retried after the limit clears (typically within 5 seconds). Proxies are more likely to reach the limit, whereas requests directly from clients are not likely." **[unverified: search excerpt of https://www.weather.gov/documentation/services-web-api]**
- **Hitting the limit returns 403, not 429.** "Expect 403 response status error with a Reference ID # in the body." (sullynole, 2024-11-07, [Discussion #772](https://github.com/weather-gov/api/discussions/772)). A missing `User-Agent` also gives 403 ([General FAQs](https://github.com/weather-gov/api/blob/master/general-faqs.md)).
- **One user polling one endpoint is fine.** "If you are a single user requesting a single endpoint, it is unlikely there would be an issue with the rate limit." (sullynole, 2026-05-13, [Discussion #865](https://github.com/weather-gov/api/discussions/865))
- **The data is public domain.** "The information on National Weather Service Web servers and Web sites is in the public domain, unless specifically annotated otherwise". The NWS name and logo are protected, and users may not imply NWS endorsement **[unverified: search excerpt of https://www.weather.gov/disclaimer]**. The API docs also disclaim endorsement of any third-party services they link to ([index.md](https://github.com/weather-gov/api/blob/master/index.md)).
- **Outages** go to NCO/OMB Tech Control (nco.ops@noaa.gov), not to GitHub ([Reporting Issues](https://github.com/weather-gov/api/blob/master/reporting-issues.md)).

### Warning messages (alerts)

**Endpoints.** These are from spec v1.9 ([openapi.yaml](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml)):

- `/alerts/active` returns all active warning messages. It filters by `status`, `message_type`, `event` (name), `code`, `area` (state or territory), `point` (lat,lon), `region`, `region_type`, `zone`, `urgency`, `severity` and `certainty`. `area`, `point`, `region`, `region_type` and `zone` exclude one another.
- `/alerts/active/zone/{zoneId}` and `/alerts/active/area/{area}` fetch by zone or by state.
- `/alerts/active/count` returns counts by area and zone.
- `/alerts/types` lists the recognized event names.
- `/alerts/{id}` fetches one message. Besides GeoJSON and JSON-LD, it can be returned as `application/cap+xml`.
- `/alerts` covers past messages, with `start` and `end`.
- `limit` is shown on `/alerts/active` in v1.9, but "It was formally removed in 2.2 in January" 2025 (StephenClouse, [Discussion #821](https://github.com/weather-gov/api/discussions/821)).

**Formats.** Collections come as `application/geo+json`, `application/ld+json` and `application/atom+xml` ([openapi.yaml](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml)). `https://api.weather.gov/alerts/active.atom` replaced the ATOM feeds at `alerts.weather.gov` under Service Change Notice 23-121, as quoted in [Discussion #685](https://github.com/weather-gov/api/discussions/685) (2023-12-21).

**Fields.** Per the spec, the `Alert` object follows "the National Weather Service CAP v1.2 specification, which extends the OASIS Common Alerting Protocol (CAP) v1.2 specification and USA Integrated Public Alert and Warning System (IPAWS) Profile v1.0" ([openapi.yaml](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml), `components.schemas.Alert`). The fields include:

- `event` (free text, such as "Severe Thunderstorm Warning")
- `severity` (Extreme / Severe / Moderate / Minor / Unknown), `urgency` (Immediate / Expected / Future / Past / Unknown) and `certainty` (Observed / Likely / Possible / Unlikely / Unknown)
- `status` (Actual / Exercise / System / Test / Draft)
- `messageType` (Alert / Update / Cancel / Ack / Error) and `references` (earlier messages that this one updates or replaces)
- `sent`, `effective`, `onset`, `expires` and `ends`
- `category` (Met, Geo, …) and `response` (Shelter, Evacuate, Prepare, …)
- `headline`, `description` and `instruction`
- `areaDesc`, `geocode.UGC`, `geocode.SAME` and `affectedZones`
- `parameters`, free-form keys from the NWS CAP spec
- a GeoJSON geometry

API 2.5 (2025-05-22) added "missing CAP fields `scope`, `code`, `language`, `eventCode` and `web`", and changed `eventEndingTime` to use the local time-zone offset ([Discussion #821](https://github.com/weather-gov/api/discussions/821)).

**Event codes.** The `code` filter takes "a concatenation of the two character Phenomena (PP) and one character Significance (S) code from the NWS Valid Time Event Code (VTEC)" (scadergit, maintainer, 2023-07-07, [Discussion #644](https://github.com/weather-gov/api/discussions/644)). The event-code list is in NWS CAP documentation on `vlab.noaa.gov`, which was blocked.

**Event names change.** On 2025-03-04, "Excessive Heat Watch/Warning" became "Extreme Heat Watch/Warning", and the VTEC code changed from EH to XH ([Discussion #800](https://github.com/weather-gov/api/discussions/800), flagged as a breaking change). A September 2026 Public Information Statement (PNS26-62) asks for comment on moving text products from VTEC to a CAP-based format. A community post notes that this "may require changes for API users who consume text products" ([Discussion #871](https://github.com/weather-gov/api/discussions/871), 2026-09-12). The PNS PDF itself (`weather.gov/media/notification/pdf_2026/PNS26-62_CAP_Transition.pdf`) was blocked.

**Where the data comes from, and what it leaves out.**

- The API "is fed from upstream, which now does the translation from text product to CAP" (scadergit, 2021-10-06, [Discussion #469](https://github.com/weather-gov/api/discussions/469)).
- Hazardous Weather Outlooks "were purposefully dropped from the upstream source of data for the API. They generally are seen in the Weather Service as forecasts and not alerts" (jah1007, NWS, 2021-10-01, same thread).
- The `hazards` layer in the gridpoint forecast "will not contain important alerts such as Tornado or Flash Flood Warnings. I would recommend against gathering alerts from the forecast grids." (same thread)

**Quirks worth knowing.**

- In 2022, "alerts that have been updated sometimes remain on the active endpoint too long". The suggested workaround was to "cross-check the expiredReferences of all alerts in the endpoint" (sullynole, 2022-01-15, [Discussion #504](https://github.com/weather-gov/api/discussions/504)). Whether this was later fixed is not stated.
- NWS's own weather.gov 2.0 code notes: "Alert ID URNs don't appear to be globally unique". It hashes each message's properties instead ([backgroundUpdateTask.js](https://github.com/weather-gov/weather.gov/blob/main/api-interop-layer/data/alerts/backgroundUpdateTask.js)).
- Some messages have no geometry. weather.gov 2.0 then builds the shape from `affectedZones`, and falls back to county SAME codes ([geometry.js](https://github.com/weather-gov/weather.gov/blob/main/api-interop-layer/data/alerts/geometry.js)). A client matching a point to a message may need zone and county shapes, or it can rely on the API's own `point` filter.
- weather.gov 2.0 requests `/alerts/active?status=actual`, which excludes test and exercise messages. It ranks events such as "severe thunderstorm warning" above "severe thunderstorm watch" ([kinds.js](https://github.com/weather-gov/weather.gov/blob/main/api-interop-layer/data/alerts/kinds.js)).

### Forecasts

- **Two-step lookup.** First `GET /points/{lat},{lon}`, then follow `forecast`, `forecastHourly` or `forecastGridData`. Forecasts are on a 2.5 km grid. The API accepts at most four decimal places in coordinates. The `/points` result "don't change very often, so you can cache the result". The API offers no geocoding ([General FAQs](https://github.com/weather-gov/api/blob/master/general-faqs.md)). The `/points` response also gives `forecastZone`, `county`, `fireWeatherZone`, `timeZone` and `radarStation` ([openapi.yaml](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml), `Point`), which are the zone codes that zone-based warning queries need.
- **Raw grid layers** from `/gridpoints/{wfo}/{x},{y}` include `probabilityOfThunder`, `windGust`, `windSpeed`, `probabilityOfPrecipitation`, `quantitativePrecipitation`, `weather` (decoded NDFD weather strings), `hazards` and many more. Each is a time series whose `validTime` is an ISO 8601 interval ([Gridpoints FAQ](https://github.com/weather-gov/api/blob/master/gridpoints.md)).
- **Freshness.** Gridpoint endpoints send `Last-Modified` and support `If-Modified-Since` (304 when nothing has changed). The body also carries `updateTime` ([Gridpoints FAQ](https://github.com/weather-gov/api/blob/master/gridpoints.md)). The API sets `Cache-Control` and asks clients not to cache-bust; unknown query parameters get a 400 ([General FAQs](https://github.com/weather-gov/api/blob/master/general-faqs.md)). A community member reports `max-age` of "1 hour" for `/gridpoints/.../forecast` (dwhitemv25, 2024-05-04, [Discussion #719](https://github.com/weather-gov/api/discussions/719)).
- **Horizon.** Breezy Weather's source comparison lists NWS daily and hourly forecasts as "Up to 7 days", covering the US, Puerto Rico, the US Virgin Islands, Guam and the Northern Mariana Islands ([Breezy Weather SOURCES.md](https://github.com/breezy-weather/breezy-weather/blob/main/docs/SOURCES.md#national-weather-service)). That is a third-party summary, not an NWS statement.
- **Storm outlooks before any watch or warning.** The Storm Prediction Center's Day 1–3 convective outlooks have categories TSTM, MRGL, SLGT, ENH, MDT and HIGH. They are reportedly published as polygons through an ArcGIS REST service that can return GeoJSON, at `https://mapservices.weather.noaa.gov/vector/rest/services/outlooks/SPC_wx_outlks/MapServer` **[unverified: search excerpt; host blocked]**. They are not part of `/alerts`.

## 2. Alternatives

### Open-Meteo

- **Terms (free API).** "Less than 10'000 API calls per day, 5'000 per hour and 600 per minute", with a monthly cap of 300,000 in the plan table. Use must be non-commercial. "private or non-profit websites or apps that do not have subscriptions or advertising" and "personal home automation purposes" count as non-commercial. Open-Meteo reserves "the right to block applications and IP addresses that misuse our service without prior notice." ([terms page source](https://github.com/open-meteo/open-meteo-website/blob/main/src/routes/en/terms/%2Bpage.svelte), last changed 2026-07-16). A request for more than 10 variables or more than 2 weeks counts as several calls, in fractions ([pricing page source](https://github.com/open-meteo/open-meteo-website/blob/main/src/routes/en/pricing/%2Bpage.svelte)).
- **Keys, licence and privacy.** The free API needs no key. The data is CC BY 4.0, and each place that shows it needs an attribution link. The code is AGPLv3. Open-Meteo says it collects no personal data; per the terms, webserver logs "may contain sensitive information such as geographical coordinates" and are deleted after 90 days ([README](https://github.com/open-meteo/open-meteo/blob/main/README.md); [terms](https://github.com/open-meteo/open-meteo-website/blob/main/src/routes/en/terms/%2Bpage.svelte)).
- **Commercial plans.** These give a key for `customer-api.open-meteo.com`, at 1M, 5M or 50M+ calls a month ([pricing](https://github.com/open-meteo/open-meteo-website/blob/main/src/routes/en/pricing/%2Bpage.svelte)). The prices are in an embedded Stripe table and could not be read.
- **Self-hosting** works with Docker or an Ubuntu package. A self-hosted instance "can access the public Open-Meteo database remotely and cache requested data locally" ([README](https://github.com/open-meteo/open-meteo/blob/main/README.md)).
- **US data.** Forecasts come from NOAA GFS and HRRR, at 3–25 km resolution, out to 16 days, updated every hour ([docs page source](https://github.com/open-meteo/open-meteo-website/blob/main/src/routes/en/docs/%2Bpage.svelte)). The variables include `weather_code`, `wind_gusts_10m`, `cape` and `lightning_potential`, plus daily `sunrise`, `sunset` and `daylight_duration` ([options.ts](https://github.com/open-meteo/open-meteo-website/blob/main/src/routes/en/docs/options.ts)). Daily variables need a `timezone` ([docs](https://github.com/open-meteo/open-meteo-website/blob/main/src/routes/en/docs/%2Bpage.svelte)).
- **Thunderstorm codes.** Code 95 is "Thunderstorm" and 97 is "Heavy thunderstorm". For models without an explicit hail forecast, "All other models derive thunderstorms from instability parameters and report codes 95 and 97" ([wmo-codes-table.svelte](https://github.com/open-meteo/open-meteo-website/blob/main/src/lib/components/variables/wmo-codes-table.svelte)). These are model output, not official warnings.
- **No warning messages.** The feature request "Alerts" ([open-meteo#351](https://github.com/open-meteo/open-meteo/issues/351), opened 2023-06-05) is still open. Breezy Weather's README lists alerts among the features for which "we currently lack a libre and gratis worldwide alternative" ([README](https://github.com/breezy-weather/breezy-weather/blob/main/README.md)).

### Pirate Weather

- **Keys, limits and licence.** A key is required (registration at `pirate-weather.apiable.io`, which was blocked). "a $2 monthly donation lets me raise your API limit from 10,000 calls/ month to 20,000 calls per month" ([docs index](https://github.com/Pirate-Weather/pirateweather/blob/main/docs/index.md)). Responses carry `Ratelimit-Limit`, `Ratelimit-Remaining` and `Ratelimit-Reset` headers ([alerts-flags-errors.md](https://github.com/Pirate-Weather/pirateweather/blob/main/docs/API/alerts-flags-errors.md)). The code is AGPL and can be self-hosted, which needs at least 32 GB of free memory and 200 GB of disk ([pirate-weather-code README](https://github.com/Pirate-Weather/pirate-weather-code/blob/main/README.md)). The terms page (`pirate-weather.apiable.io/terms`) was blocked.
- **Warning messages.** Responses include an `alerts` block with `title`, `regions`, `severity`, `time`, `expires`, `description` and `uri` ([alerts-flags-errors.md](https://github.com/Pirate-Weather/pirateweather/blob/main/docs/API/alerts-flags-errors.md)). The block has no event code or message type. US messages are ingested from the NWS WWA shapefile (`tgftp.nws.noaa.gov/.../current_all.tar.gz`), merged with `api.weather.gov` active alerts ([NWS_Alerts_Local.py](https://github.com/Pirate-Weather/pirate-weather-code/blob/main/API/NWS_Alerts_Local.py)). The self-hosting compose file schedules that ingest at `"0 0,30 * * * *"`, which is every 30 minutes ([pirate-compose_oph](https://github.com/Pirate-Weather/pirate-weather-code/blob/main/pirate-compose_oph)). The hosted service's schedule is not documented in anything read. WMO messages were added for worldwide coverage ([changelog](https://github.com/Pirate-Weather/pirateweather/blob/main/docs/changelog.md)).
- **Forecasts** use NBM, HRRR, RTMA-RU and GFS for the US ([DataSources.md](https://github.com/Pirate-Weather/pirateweather/blob/main/docs/DataSources.md)). The daily data includes `sunriseTime` and `sunsetTime` ([data-blocks.md](https://github.com/Pirate-Weather/pirateweather/blob/main/docs/API/data-blocks.md)).

### Apple WeatherKit REST API

- **Access.** WeatherKit requires Apple Developer Program membership ([WeatherKit](https://developer.apple.com/weatherkit/)), which costs "99 USD per membership year" ([What's included](https://developer.apple.com/programs/whats-included/)). Each request carries a JWT signed with ES256 using a WeatherKit private key. Apple says: "Never distribute your private key. If you need to create tokens for apps or websites, create an authenticated service to create and sign your own tokens." ([Request authentication](https://developer.apple.com/documentation/weatherkitrestapi/request-authentication-for-weatherkit-rest-api)). An Android or Linux client would therefore need a server to issue tokens.
- **Limits.** 500,000 calls a month are included, with paid tiers from US$49.99 for 1M a month ([WeatherKit](https://developer.apple.com/weatherkit/)).
- **Warning messages.** These are "available for select regions" ([WeatherKit](https://developer.apple.com/weatherkit/)). `countryCode` "is necessary for weather alerts" ([GET /api/v1/weather](https://developer.apple.com/documentation/weatherkitrestapi/get-api-v1-weather-_language_-_latitude_-_longitude_)). A `WeatherAlertSummary` has `source`, `severity`, `urgency`, `certainty`, `effectiveTime`, `eventOnsetTime`, `eventEndTime`, `expireTime`, `issuedTime`, `responses` and `detailsUrl` ([WeatherAlertSummary](https://developer.apple.com/documentation/weatherkitrestapi/weatheralertsummary)).
- **Display rules for warnings.** Each must link to Apple's details page and name the issuing agency, and "You must not modify, change, alter, or obscure the text of a severe weather alert in any way" ([WeatherKit](https://developer.apple.com/weatherkit/), attribution section).
- **Sunrise and sunset.** Daily data has `sunrise` and `sunset`, plus civil, nautical and astronomical variants ([DayWeatherConditions](https://developer.apple.com/documentation/weatherkitrestapi/dayweatherconditions)).
- **Push.** None is documented for the REST API in the pages read.

### FOSS Public Alert Server (warning messages only)

- **What it is.** An open-source server (AGPL-3.0-or-later) that "aggregates hundreds of CAP Feeds published by alerting authorities worldwide" and "lets clients receive push notifications via UnifiedPush" ([readme](https://github.com/KDE/foss-public-alert-server/blob/master/readme.md)). It is a KDE / FOSS Warn project.
- **Status.** The readme says "This project is still in development and not yet ready for production!", and the API describes itself as "in an early testing phase, and the API may change" ([openAPI-docu.yaml](https://github.com/KDE/foss-public-alert-server/blob/master/openAPI-docu.yaml)).
- **How subscriptions work.** A client subscribes a UnifiedPush endpoint for a bounding box. The server pushes `added` and `update` notifications. Subscriptions "need to be regularly updated to not expire" ([openAPI-docu.yaml](https://github.com/KDE/foss-public-alert-server/blob/master/openAPI-docu.yaml), `POST /subscription/`). A public instance is at `https://alerts.kde.org` (same file).
- **US coverage.** The US source is `https://api.weather.gov/alerts/active.atom` ([custom_feeds.json](https://github.com/KDE/foss-public-alert-server/blob/master/foss_public_alert_server/sourceFeedHandler/custom_feeds.json)). By default the server polls its feeds every 60 seconds (`DEFAULT_UPDATE_PERIOD_FOR_CAP_FEEDS = 60`, [settings.py](https://github.com/KDE/foss-public-alert-server/blob/master/foss_public_alert_server/foss_public_alert_server/settings.py)).
- **Relevance.** It uses no Google services and can be self-hosted, but it adds a push hop and a dependency on a project that is still in testing.

### Not verified

OpenWeatherMap, Tomorrow.io, AccuWeather/Xweather, MET Norway and the USNO astronomical API were all blocked, so their terms, and whether they offer warning messages or webhooks, are unknown here.

## 3. Rate limits, API keys and terms at a glance

| Source | Key | Free limit | Licence / use terms | Warning messages | Sunrise/sunset | Self-host |
|---|---|---|---|---|---|---|
| NWS API | None; `User-Agent` with contact. An API key is planned. | Unpublished, "generous"; 403 when exceeded | Public domain; no implied endorsement [excerpt] | Yes (CAP v1.2 fields) | No | No (it is the upstream) |
| Open-Meteo | None (free tier) | 600/min, 5,000/h, 10,000/day, 300,000/month | Non-commercial only; CC BY 4.0 attribution | No | Yes (daily) | Yes (AGPLv3) |
| Pirate Weather | Required (free) | 10,000/month | Terms page not reachable; code AGPL | Yes (NWS, WMO); reduced field set | Yes (daily) | Yes (heavy) |
| Apple WeatherKit | Signed JWT; paid developer membership | 500,000/month included | Apple attribution and display rules | Yes, "select regions" | Yes (daily, with twilights) | No |
| FOSS Public Alert Server | None stated | Not stated | AGPL; terms on a KDE wiki page (not read) | Yes (CAP, from NWS ATOM) | No | Yes |
| On-device calculation | — | — | Libraries: MIT (`sunrise` crate); `suncalc` licence not read | — | Yes | n/a |

Sources are as cited in sections 1, 2 and 5.

## 4. How quickly warning messages appear, and push versus polling

**Polling the NWS API**

- **Suggested interval.** NWS reportedly recommends "making requests of the server no more than every 30 seconds" **[unverified: search excerpt of https://www.weather.gov/documentation/services-web-alerts]**.
- **Cache lifetime.** A community member reports "`/alerts/active` max-age is 30 seconds". Re-requesting before then "will simply return" cached content, and "Once or twice a minute isn't going to run into any issues" (dwhitemv25, community, 2024-05-04, [Discussion #719](https://github.com/weather-gov/api/discussions/719)).
- **What NWS's own site does.** weather.gov 2.0 fetches `/alerts/active?status=actual` on a `setTimeout(..., 30_000)` loop, so every 30 seconds ([backgroundUpdateTask.js](https://github.com/weather-gov/weather.gov/blob/main/api-interop-layer/data/alerts/backgroundUpdateTask.js)).
- **No conditional requests.** API 3.3: "`/alerts` endpoints no longer use the Last-Modified header due to caching issues" ([Discussion #846](https://github.com/weather-gov/api/discussions/846)). `If-Modified-Since` therefore no longer applies to warning messages.
- **No published latency figure.** API 2.5 added "`/health`: added CAP product latency" ([Discussion #821](https://github.com/weather-gov/api/discussions/821)). That means the API tracks how long CAP products take to arrive, but no published figure was found for the time from a forecaster issuing a warning to it appearing in `/alerts/active`.

**Push options**

- **The NWS API itself.** It has no webhooks or push. A user's 2025 request for a webhook drew replies only from community members: "there are no webhooks" (wsamoht) and "NWS has a 'push' service in NWS Weather Wire, a stream of text products and CAP-formatted alerts" (dwhitemv25) ([Discussion #808](https://github.com/weather-gov/api/discussions/808)). The API 3.3 notes describe the new radio endpoint as "not intended to replace NOAA Weather Radio (there is no real-time alerting, for starters)" ([Discussion #846](https://github.com/weather-gov/api/discussions/846)).
- **NWWS-OI** (NOAA Weather Wire Service Open Interface) is an XMPP stream. An account is requested by emailing NWWS.Issue@noaa.gov, with "a wait of up to 30+ days". The same user ID and password cannot be used on several machines **[unverified: search excerpts of https://www.weather.gov/nwws/nwws_oi_request and https://www.weather.gov/nwws/OISetup]**. The eligibility rules could not be read. A community member notes that NWWS-OI carries both VTEC and CAP messages ([Discussion #871](https://github.com/weather-gov/api/discussions/871)).
- **IPAWS All-Hazards Information Feed** (FEMA) is described as free, CAP over "a simple to implement, HTTP interface", with registration through the IPAWS User Portal. Whether it pushes or is polled, and which NWS products it carries, could not be confirmed **[unverified: search excerpt of https://www.fema.gov/emergency-managers/practitioners/integrated-public-alert-warning-system/technology-developers/all-hazards-information-feed]**.
- **EMWIN.** A community member states "The NWS no longer provides the push EMWIN data stream", and that a private group now runs it ([Discussion #808](https://github.com/weather-gov/api/discussions/808), 2026-05-20). This is not from NWS.
- **FOSS Public Alert Server** pushes over UnifiedPush after polling the NWS ATOM feed every 60 seconds (see §2). Its latency is at least that polling interval plus push delivery.
- **Pirate Weather** re-polls every 30 minutes in its self-hosting configuration (see §2). The hosted service is unknown.
- **Wireless Emergency Alerts (phones).** NWS sends Severe Thunderstorm Warnings through WEA "only when the damage threat is destructive": winds of at least 80 mph or hail of 2.75 inches or more **[unverified: search excerpt of https://www.weather.gov/news/072221-svr-wea]**. So most severe thunderstorm warnings never reach phones this way. Whether an Android app can read WEA broadcasts was not researched.

**What this means for where weather is evaluated** (facts only):

- A device polling `/alerts/active?point=…` makes one request per location per interval. A server can poll the whole national feed once and match it against every reminder's source.
- The NWS says proxies are more likely than clients to hit the rate limit **[unverified: search excerpt]**.
- Getting a warning message from a server to an Android device quickly requires a push channel. UnifiedPush is what the FOSS Public Alert Server uses; its push path was not otherwise researched here.

## 5. Sunrise and sunset

- **NWS API: not provided.** Spec v1.9 has no sun or twilight fields ([openapi.yaml](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml)). The current 3.x spec could not be checked.
- **Open-Meteo, Pirate Weather and WeatherKit** all return daily sunrise and sunset (see §2). WeatherKit also returns civil, nautical and astronomical twilight.
- **Offline calculation (NOAA method).** NOAA's Solar Calculator reportedly uses equations from Jean Meeus's *Astronomical Algorithms*. It is "theoretically accurate to within a minute for locations between +/- 72° latitude, and within 10 minutes outside of those latitudes", and is "no longer actively supported or maintained" **[unverified: search excerpt of https://gml.noaa.gov/grad/solcalc/calcdetails.html; page blocked]**.
- **Rust library.** The [`sunrise` crate](https://github.com/nathan-osman/rust-sunrise) (v3.0.0 on the crates.io index, MIT) computes sunrise, sunset and civil, nautical and astronomical dawn and dusk from date and coordinates. It uses the Wikipedia "sunrise equation" method, and supports `no_std` with `libm` ([README](https://github.com/nathan-osman/rust-sunrise/blob/master/README.md), [Cargo.toml](https://github.com/nathan-osman/rust-sunrise/blob/master/Cargo.toml), [crates index](https://index.crates.io/su/nr/sunrise)).
- **JavaScript library.** [`suncalc`](https://github.com/mourner/suncalc) (v2.0.2 on npm, published 2026-09-02) is "based on the formulas from Jean Meeus' *Astronomical Algorithms*" and says it matches USNO and timeanddate.com conventions ([README](https://github.com/mourner/suncalc/blob/master/README.md), [npm registry](https://registry.npmjs.org/suncalc)).
- **Offline calculation needs only coordinates, a date and a time zone.** It works the same in any region, and on the device or on the server.

## 6. What a region's weather module would need to expose

These are the capabilities the sources above differ on, and which a module interface would have to account for if other regions are added later. This section lists facts and does not propose a design.

1. **Coverage.** Whether the module serves a given coordinate. NWS covers the US and its territories ([Breezy SOURCES.md](https://github.com/breezy-weather/breezy-weather/blob/main/docs/SOURCES.md#national-weather-service)). WeatherKit covers warnings only for "select regions" and needs a country code.
2. **Location resolution and caching.** NWS needs a `/points` lookup to get a grid cell, zones, county and time zone, and recommends caching it. It limits coordinates to four decimals and has no geocoding ([General FAQs](https://github.com/weather-gov/api/blob/master/general-faqs.md)). Other providers take coordinates directly.
3. **Active warning messages for a location**, normalized to a common shape. CAP v1.2 is the common denominator: NWS ([openapi.yaml](https://github.com/weather-gov/api/blob/master/assets/openapi.yaml)), the FOSS Public Alert Server's worldwide sources ([readme](https://github.com/KDE/foss-public-alert-server/blob/master/readme.md)) and WeatherKit's summary fields ([WeatherAlertSummary](https://developer.apple.com/documentation/weatherkitrestapi/weatheralertsummary)) all use its vocabulary. The fields that matter for a *trigger* are:
   - identity: an id, or a content hash where ids are not unique ([backgroundUpdateTask.js](https://github.com/weather-gov/weather.gov/blob/main/api-interop-layer/data/alerts/backgroundUpdateTask.js))
   - lifecycle: `messageType` (Alert, Update or Cancel) and `references`, so that an update is not taken for a new warning
   - timing: `sent`, `effective`, `onset`, `expires` and `ends`
   - area: a polygon, or zone and county codes that need their own shapes
   - `status`, so that Test and Exercise messages can be filtered out
   - `severity`, `urgency` and `certainty`
   - the text: `headline`, `description` and `instruction`
   - a link, and the name of the issuing agency. WeatherKit requires the agency name ([WeatherKit](https://developer.apple.com/weatherkit/)).
4. **Event vocabulary.** CAP's `event` is free text, and each region has its own codes (NWS uses VTEC-derived PP+S codes, [Discussion #644](https://github.com/weather-gov/api/discussions/644)). Names can change (EH to XH, [Discussion #800](https://github.com/weather-gov/api/discussions/800)), and the NWS text-product format may change too ([Discussion #871](https://github.com/weather-gov/api/discussions/871)). A reminder written as "when a severe storm is coming" would need each region's events mapped to kinds the app understands. It would also need to decide which of these counts as "coming": an SPC outlook, a watch or a warning.
5. **Forecast values.** These vary by source: NWS grid layers such as `probabilityOfThunder` and `windGust`; Open-Meteo's `weather_code`, `cape` and `wind_gusts_10m`; and so on. Each has its own horizon, resolution and update time (`updateTime` / `Last-Modified` on NWS gridpoints). A forecast is a separate kind of data from warning messages, and some sources offer only one of the two (Open-Meteo has no warning messages).
6. **Delivery mode and freshness.** Whether the module polls or receives pushes; the minimum polling interval and cache hints (30 seconds for NWS warning messages); whether conditional requests work (not for NWS `/alerts` since 3.3); and how rate limiting shows up (a 403 on NWS). These decide how quickly a *trigger* can fire, and whether it can run on the device or needs the server.
7. **Terms and credentials.** For each source: whether a key is needed, and whether it can live on the device (a WeatherKit private key cannot); identification requirements (the NWS `User-Agent`); limits; commercial-use restrictions (Open-Meteo); and attribution text or links to show wherever the data appears (Open-Meteo CC BY; Apple's rules).
8. **Sunrise and sunset** are astronomical rather than regional (§5), so they need not belong to a region's module.

## Gaps and open questions

- **Live NWS spec not read.** `api.weather.gov/openapi.json` was blocked. The spec used here is v1.9 (2021), and the live API is 3.7. Parameter lists, pagination, and new fields after 2.5 (such as `eventCode`) should be checked against the live spec.
- **NWS documentation pages not read.** The rate-limit wording, the "no more than every 30 seconds" guidance, the public-domain disclaimer and the `User-Agent` format come from search excerpts only.
- **No end-to-end latency figure.** Nothing found says how long a Severe Thunderstorm Warning takes to reach `/alerts/active` after issuance. The `/health` endpoint's "CAP product latency" could be sampled once the host can be reached.
- **NWWS-OI eligibility and terms.** Who may get an account, whether a personal or self-hosted server qualifies, and any redistribution limits were not readable.
- **IPAWS feed.** Its push or poll model, which NWS products it carries, and the registration terms were not verified.
- **Effect of PNS26-62 (VTEC to CAP transition).** It is unclear whether the proposal changes `/alerts` event names, codes or `parameters`, and when. The PDF was blocked.
- **Open-Meteo "non-commercial" and limit scope.** Whether hab-bot's use would count as non-commercial depends on how it is distributed. Whether the daily limit counts per IP, per app or for all users of one app is not stated in the terms.
- **Terms for Pirate Weather and the FOSS Public Alert Server.** Neither terms page could be read. Pirate Weather's hosted alert-ingest interval is also undocumented in the sources read.
- **Point matching accuracy.** How NWS's `point=` filter decides whether a point is inside a message's area (polygon, zone or county) is not documented in the sources read.
- **Other providers.** OpenWeatherMap, Tomorrow.io, Xweather and MET Norway were blocked. Whether any offers webhooks for US warning messages on acceptable terms is unknown.
- **Push to Android without Google.** Server-side evaluation needs some way to reach the device quickly. UnifiedPush is used by the FOSS Public Alert Server, but its delivery characteristics on Android were not researched here.
- **Glossary clash.** Providers' "alerts" and `CONTEXT.md`'s **Alert** mean different things. The domain docs may want a term for an agency's watch, warning or advisory.
