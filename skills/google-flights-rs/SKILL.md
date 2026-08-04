---
name: google-flights-rs
description: Search flights and fares through the gflights MCP tools or CLI, including booking offers and URLs. Use when the user asks to find flights, compare airfare, check flight prices, find the cheapest dates to fly, compare cabins or airports, get booking links, or plan a trip. Triggers include "find flights", "flights from X to Y", "how much is a flight", "cheapest flight", "when should I fly", "airfare", "nonstop flights", "business class price", and "booking link".
---

# Flight Search with gflights

`gflights` queries the Google Flights web API directly. It needs no API key or
browser. If the CLI is not on `PATH`, install it with:

```bash
cargo install gflights
```

## Critical Defaults to Override

gflights defaults to currency **euro** and country **GB**. Always set both explicitly:

```bash
gflights search ... --currency us-dollar --country US
```

Currency values are spelled out: `us-dollar`, `thai-baht`, `japanese-yen`, `euro`,
`british-pound`, and so on. Run `gflights search --help` for the full list.

## Commands

```bash
gflights search --from BKK --to NRT --date 2026-10-09          # flights on a date
gflights cheap  --from BKK --to NRT --date 2026-10-01 --months 1   # cheapest dates
gflights offer  --from BKK --to NRT --date 2026-10-09          # booking URLs + provider prices; accepts the search filters; --open launches the cheapest in your browser
gflights graph  --from BKK --to NRT --date 2026-10-09 --months 3   # price per day
gflights dgrid  --from A --to B --dep-start .. --dep-end .. --ret-start .. --ret-end ..
gflights deals  --from BKK --out 2026-10-09 --ret 2026-10-16   # discounted destinations
gflights explore --from BKK --month 10 --budget 300            # cheap destinations by budget
gflights mcity  --leg BKK,NRT,2026-10-09 --leg NRT,BKK,2026-10-16  # multi-city
gflights mcp                                                   # MCP stdio server
```

`--from` and `--to` accept IATA codes or city names such as "London". There is no
separate airport-lookup subcommand. Add `--format json` on `search` when you need to
compare, filter, or tabulate results. Do not use `gflights select`. It is an
interactive picker and will hang a non-interactive session.

## Choose a Transport

- Use MCP when the client has a connected gflights server. MCP exposes every
  non-interactive CLI action and returns structured JSON.
- Use the CLI when MCP is unavailable. Both MCP `offer` and CLI `offer` resolve
  each priced offer's click token into a provider booking URL; CLI `offer` also
  prints the table and `--open` launches the cheapest in a browser.
- Do not start an MCP server for a one-off shell request. Call the CLI directly.
- Check currency and country before searching. MCP fixes both values when the
  server starts. The tool arguments cannot override them. Use the CLI when the
  running server has the wrong locale.

## MCP Server

The same binary runs an MCP stdio server. It exposes `search`, `price_graph`,
`cheapest_dates`, `explore`, `deals`, `date_grid`, `offer`, and `multi_city`.
The `search` and `offer` tools accept the full search filter set. `search` also
takes `priced_only` (drop itineraries without a bookable price) and `web_url`
(return only the browsable Google Flights URL, no API call); `offer` also
takes `open` (resolve the cheapest booking URL and launch it in the host's
default browser). Give MCP users this configuration and set their locale at
startup:

```json
{
  "mcpServers": {
    "gflights": {
      "command": "gflights",
      "args": ["--currency", "us-dollar", "--country", "US", "mcp"]
    }
  }
}
```

Map CLI commands to MCP tools as follows: `cheap` to `cheapest_dates`, `graph`
to `price_graph`, `dgrid` to `date_grid`, and `mcity` to `multi_city`.

Key CLI `search` flags: `--return YYYY-MM-DD` for round trips,
`--stops all|no-stop|one-or-less|two-or-less`,
`--class economy|premium-economy|business|first`,
`--sort top-flights|best|price|departure-time|arrival-time|duration|emissions`,
`--time 6-20` outbound departure window on the 24h clock, `--arr-time`, `--ret-time`
and `--ret-arr-time` for the other windows, `--bags 0-2` checked bags folded into the
displayed price, `--carry-on 0-2` likewise, `--max-price N`, `--exclude-basic` to drop
basic-economy fares, `--priced-only` to drop itineraries Google returned without a bookable
price, `--web-url` to print the browsable Google Flights URL for the search and exit (no API
call), `--airline LX` or `--airline ONEWORLD` to include an airline or
alliance. Repeat `--airline` and `--exclude-airline` as needed. Use `--via CDG` to require a
connection airport, `--min-layover MIN` and `--max-layover MIN` rounded up to
30-minute steps, `--lower-emissions`, `--show-co2`, and `--detail` for layover
airports and next-day markers.

Key CLI `cheap` flags: `--months N` scan window, `--trip-days N` for round trips of
exactly N nights, `--weekday mon..sun` (or 1..7) to keep only departures on that weekday.
Omit `--trip-days` for one-way date discovery.

## Workflow

1. If dates are flexible, call MCP `cheapest_dates` or CLI `cheap` first. Then
   search the best candidate dates.
2. Put cabin, stops, bags, airlines, and time windows into the request. Prices
   must reflect the constraints.
3. Summarize the top handful of practical options in a table: airline and flight
   number, departure and arrival, duration, stops, price, and caveats. Never dump
   raw JSON at the user.
4. For international trips, also quote business alongside economy so the upgrade
   delta is visible. Skip that for short domestic hops unless asked.
5. Once the user chooses a flight, call `offer` for airline and OTA prices. Use
   CLI `offer` when the user needs resolved booking URLs. Add `--open` to
   launch the cheapest offer's booking page in the default browser — no
   copy-paste. It works with both table and `--format json` output.

## Ranking Results

Default to best value, not pure cheapest:

1. Best value: price plus total duration, departure and arrival times, layover
   quality, stops, and baggage assumptions.
2. Cheapest valid: lowest price after excluding self-transfer itineraries, layovers
   under 90 minutes on international connections, and overnight layovers unless the
   user accepts them.
3. Fastest or nonstop as the benchmark to price the comfort premium.

Flag in caveats: `self_transfer` and `mixed_cabin` markers, basic-economy fares, and
low-cost carriers whose headline price excludes bags. All of these, plus legroom and
wifi, power, and video flags per leg, are available in `search --format json`.

## Known Quirks

- A midnight departure hour serializes as null in JSON output. Treat a null hour
  as 00.
- Some itineraries have no bookable price: Google published the schedule but not
  a fare. They render as `—` in the table and serialize as `trip_cost: null` in
  JSON. Add `--priced-only` to drop them when you need a clean, fully-priced list
  (for example before sorting or tabulating with jq).
- `cheap`/`cheapest_dates` filter weekdays client-side with `--weekday`/`weekday`
  (mon..sun or 1..7); the underlying endpoint has no day-of-week parameter.

## After Selecting a Flight

- For seat quality, use the airline seat map and https://www.aerolopa.com/. Check
  window alignment, lavatory and galley rows, and exit-row tradeoffs.
- A wifi flag does not mean wifi is free. Verify on the airline page.
- This data is discovery, not gospel: verify the final fare, fare class, and baggage
  rules at the provider before anyone pays. CLI `offer` links go straight to the
  provider's checkout, where the price can differ.

## Risks

gflights talks to reverse-engineered Google Flights endpoints. Google can change
them without notice. If searches fail, check the repository for updates and search
Google Flights manually.
