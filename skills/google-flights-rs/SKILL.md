---
name: google-flights-rs
description: Search flights and fares with the gflights CLI, a Google Flights client with booking URLs. Use when the user asks to find flights, compare airfare, check flight prices, find the cheapest dates to fly, compare cabins or airports, get booking links, or plan a trip. Triggers include "find flights", "flights from X to Y", "how much is a flight", "cheapest flight", "when should I fly", "airfare", "nonstop flights", "business class price", "booking link".
---

# Flight Search with gflights

`gflights` is a single-binary CLI that queries the Google Flights web API directly —
no API key, no browser. If it is not on PATH, install it with:

```bash
cargo install gflights
```

## Critical Defaults to Override

gflights defaults to currency **euro** and country **GB**. Always set both explicitly:

```bash
gflights search ... --currency us-dollar --country US
```

Currency values are spelled out: `us-dollar`, `thai-baht`, `japanese-yen`, `euro`,
`british-pound`, and so on — run `gflights search --help` for the full list.

## Commands

```bash
gflights search --from BKK --to NRT --date 2026-10-09          # flights on a date
gflights cheap  --from BKK --to NRT --date 2026-10-01 --months 1   # cheapest dates
gflights offer  --from BKK --to NRT --date 2026-10-09          # booking URLs + provider prices
gflights graph  --from BKK --to NRT --date 2026-10-09 --months 3   # price per day
gflights dgrid  --from A --to B --dep-start .. --dep-end .. --ret-start .. --ret-end ..
gflights deals  --from BKK --out 2026-10-09 --ret 2026-10-16   # discounted destinations
gflights explore --from BKK --month 10 --budget 300            # cheap destinations by budget
gflights mcity  --leg BKK,NRT,2026-10-09 --leg NRT,BKK,2026-10-16  # multi-city
gflights mcp                                                   # MCP stdio server
```

`--from` and `--to` accept IATA codes or city names such as "London" — there is no
separate airport-lookup subcommand. Add `--format json` on `search` when you need to
compare, filter, or tabulate results. Do not use `gflights select` — it is an
interactive picker and will hang a non-interactive session.

Always use the CLI when you have shell access, even if a gflights MCP server is
connected: the MCP tools are a reduced subset — no `offer` booking URLs and fewer
filters — and the CLI needs no standing server process.

## MCP Server

The same binary doubles as an MCP stdio server for clients without shell access,
such as Claude Desktop. It exposes `search`, `price_graph`, `cheapest_dates`,
`explore`, and `deals` tools. To offer it when a user asks how to use gflights from
an MCP client, give them this config:

```json
{
  "mcpServers": {
    "gflights": {
      "command": "gflights",
      "args": ["mcp"]
    }
  }
}
```

Booking URLs still require the CLI `offer` subcommand — the MCP server does not
expose it.

Key `search` flags: `--return YYYY-MM-DD` for round trips,
`--stops all|no-stop|one-or-less|two-or-less`,
`--class economy|premium-economy|business|first`,
`--sort top-flights|best|price|departure-time|arrival-time|duration|emissions`,
`--time 6-20` outbound departure window on the 24h clock, `--arr-time`, `--ret-time`
and `--ret-arr-time` for the other windows, `--bags 0-2` checked bags folded into the
displayed price, `--carry-on 0-2` likewise, `--max-price N`, `--exclude-basic` to drop
basic-economy fares, `--airline LX` or `--airline ONEWORLD` to include an airline or
alliance — repeatable, `--exclude-airline` likewise, `--via CDG` to require a
connection airport, `--min-layover MIN` and `--max-layover MIN` rounded up to
30-minute steps, `--lower-emissions`, `--show-co2`, and `--detail` for layover
airports and next-day markers.

Key `cheap` flags: `--months N` scan window, `--trip-days N` for round trips of
exactly N nights — omit it for one-way date discovery.

## Workflow

1. If dates are flexible, run `cheap` first to find the cheap window, then `search`
   on the best candidate dates.
2. Bake user constraints into the command — cabin, stops, bags, airlines, time
   windows — rather than filtering afterward, so prices reflect them.
3. Summarize the top handful of practical options in a table: airline and flight
   number, departure and arrival, duration, stops, price, and caveats. Never dump
   raw JSON at the user.
4. For international trips, also quote business alongside economy so the upgrade
   delta is visible. Skip that for short domestic hops unless asked.
5. Once a flight is chosen, run `offer` on that route and date for airline and OTA
   booking URLs with real provider prices — these can undercut the metasearch
   headline fare.

## Ranking Results

Default to best value, not pure cheapest:

1. Best value — price plus total duration, departure and arrival times, layover
   quality, stops, and baggage assumptions.
2. Cheapest valid — lowest price after excluding self-transfer itineraries, layovers
   under 90 minutes on international connections, and overnight layovers unless the
   user accepts them.
3. Fastest or nonstop as the benchmark to price the comfort premium.

Flag in caveats: `self_transfer` and `mixed_cabin` markers, basic-economy fares, and
low-cost carriers whose headline price excludes bags. All of these, plus legroom and
wifi, power, and video flags per leg, are available in `search --format json`.

## Known Quirks

- A midnight departure hour serializes as null in JSON output — treat a null hour
  as 00.
- `cheap` has no day-of-week filter — filter its output by weekday yourself when the
  user wants, say, Friday departures only.

## After Selecting a Flight

- For seat quality use the airline seat map and https://www.aerolopa.com/ — check
  window alignment, lavatory and galley rows, and exit-row tradeoffs.
- A wifi flag does not mean wifi is free — verify on the airline page.
- This data is discovery, not gospel: verify the final fare, fare class, and baggage
  rules at the provider before anyone pays. `offer` links go straight to the
  provider's checkout, where the price can differ.

## Risks

gflights talks to reverse-engineered Google Flights endpoints, which can change
without notice. If searches start failing, check the repository for updates and fall
back to searching Google Flights manually.
