---
name: flight-search
description: Search flights and fares using the locally installed gflights Rust CLI — a Google Flights client with booking URLs. Use when the user asks to find flights, compare airfare, check flight prices, find the cheapest dates to fly, compare cabins or airports, get booking links, or plan a trip. Triggers include "find flights", "flights from X to Y", "how much is a flight", "cheapest flight", "when should I fly", "airfare", "nonstop flights", "business class price", "booking link".
---

# Flight Search with gflights

`gflights` is installed at `~/.cargo/bin/gflights` — a single-binary Rust CLI that calls
the Google Flights web API directly. No API key, no browser, no Python runtime. Source:
https://github.com/nas-/google-flights-rs. Full background research and ranking of
alternatives lives in `/home/appsmith/txt/flight-mcp.md`.

If missing or broken, reinstall with `cargo install gflights`. It also ships an MCP
stdio server via `gflights mcp` if a client ever needs one.

## Critical Defaults to Override

gflights defaults to **euro** and country **GB**. Always set these explicitly:

```bash
gflights search ... --currency us-dollar --country US
```

Currency values are spelled out: `us-dollar`, `thai-baht`, `japanese-yen`, `euro`.

## Commands

```bash
gflights search --from BKK --to NRT --date 2026-10-09        # flights on a date
gflights cheap  --from BKK --to NRT --date 2026-10-01 --months 1   # cheapest dates
gflights offer  --from BKK --to NRT --date 2026-10-09        # booking URLs + provider prices
gflights graph  --from BKK --to NRT --date 2026-10-09 --months 3   # price per day
gflights dgrid  --from A --to B --dep-start .. --dep-end .. --ret-start .. --ret-end ..
gflights deals  --from BKK --out 2026-10-09 --ret 2026-10-16  # discounted destinations
gflights explore --from BKK --month 10 --budget 300           # cheap destinations by budget
gflights mcity  --leg BKK,NRT,2026-10-09 --leg NRT,BKK,2026-10-16  # multi-city
```

`--from`/`--to` accept IATA codes or city names like "London". Add `--format json` on
`search` for piping to jq. Do not use `gflights select` — it is an interactive picker.

Key `search` flags: `--return YYYY-MM-DD` round trip, `--stops all|no-stop|one-or-less|
two-or-less`, `--class economy|premium-economy|business|first`,
`--sort best|price|duration|departure-time|arrival-time`, `--airline LX` / `--airline
ONEWORLD` include airline or alliance — repeatable, `--exclude-airline` likewise,
`--via CDG` require a connection airport, `--min-layover MIN` / `--max-layover MIN`
rounded up to 30-minute steps, `--adults N --children N`, `--lower-emissions`,
`--show-co2`, `--detail` layover airports and next-day markers.

Key `cheap` flags: `--months N` scan window, `--trip-days N` for round trips of exactly
N nights — omit for one-way date discovery.

## Workflow

1. If dates are flexible, run `cheap` first to find the cheap window, then `search` on
   the best candidates.
2. Bake user constraints into the command — cabin, stops, airlines, layover limits —
   rather than filtering afterward, so prices reflect them.
3. Summarize the top handful of practical options in a table: airline and flight number,
   departure and arrival, duration, stops, price, and caveats. Never dump raw JSON.
4. For international trips, also quote business alongside economy so the upgrade delta
   is visible. Skip that for US domestic unless asked.
5. Once a flight is chosen, run `offer` on that route and date for airline/OTA booking
   URLs with provider prices — these can undercut the Google headline fare, e.g. $179
   direct from HK Express when the metasearch fare said $202.

## Ranking Results

Default to best value, not pure cheapest:

1. Best value — price plus total duration, departure and arrival times, layover
   quality, stops, and baggage assumptions.
2. Cheapest valid — lowest price after excluding self-transfer itineraries, layovers
   under 90 minutes on international connections, and overnight layovers unless the
   user accepts them.
3. Fastest or nonstop as the benchmark to price the comfort premium.

Flag in caveats: self-transfer status, basic economy, separate tickets with checked
bags, and unclear baggage inclusion. Low-cost carriers like HK Express, Vietjet, and
ZIPAIR price bags separately — say so when they top the list.

## Known Quirks

- Prices exclude checked bags and there is no bags-in-price flag. When baggage-inclusive
  pricing matters, cross-check with `fli` — still installed at `~/.local/bin/fli` — which
  supports `--bags 1` and `--carry-on`.
- JSON output serializes a midnight departure hour as null — treat a null hour as 00.
- fli also returns legroom and amenity flags — Wi-Fi, power, video — that gflights does
  not; use it when seat comfort drives the choice.

## After Selecting a Flight

- For seat quality use the airline seat map and https://www.aerolopa.com/ — check window
  alignment, lavatory and galley rows, and exit-row tradeoffs.
- Wi-Fi being listed does not mean it is free — verify on the airline page.
- This data is discovery, not gospel: verify the final fare, fare class, and baggage
  rules on the airline site or a reputable OTA before anyone pays. Prices can differ at
  checkout, and `offer` links go straight to the provider's checkout.

## Risks

gflights calls reverse-engineered Google Flights endpoints, so it can break without
warning if Google changes the interface, and the project is young with a small user
base. Fallbacks in order: `fli` locally, the Kiwi hosted MCP at https://mcp.kiwi.com,
manual Google Flights — see flight-mcp.md for the full ranking.
