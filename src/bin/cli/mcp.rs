//! MCP (Model Context Protocol) server over stdio.
//!
//! Speaks JSON-RPC 2.0 with newline-delimited messages on stdin/stdout — the
//! MCP stdio transport — so it works with any MCP client (e.g. Claude Desktop).
//! Each tool is a thin adapter: it parses JSON arguments, builds the existing
//! `Config`/`ExploreConfig`, calls the corresponding `ApiClient` method, and
//! returns the serialized result. No new business logic lives here.
//!
//! Supported tools: `search`, `price_graph`, `cheapest_dates`, `explore`,
//! `deals`, `date_grid`, `offer`, `multi_city`.

use anyhow::Result;
use chrono::{Months, NaiveDate};
use clap::ValueEnum;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Stdout};

use gflights::parsers::common::{
    AirlineFilter, FlightTimes, Location, PlaceType, SortOrder, StopOptions, StopoverDuration,
    TravelClass, Travelers,
};
use gflights::requests::api::ApiClient;
use gflights::requests::config::explore::resolve_interest;
use gflights::requests::config::{
    Config, DealConfig, ExploreConfig, ExploreDate, ExploreDuration, MultiCityConfig,
};

/// MCP protocol revision this server implements.
const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "gflights";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ---------------------------------------------------------------------------
// Server loop
// ---------------------------------------------------------------------------

pub async fn run_mcp(client: &ApiClient) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                send_error(&mut out, Value::Null, -32700, &format!("parse error: {e}")).await?;
                continue;
            }
        };

        let id = msg.get("id").cloned(); // absent => notification (no response)
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => {
                send_result(&mut out, id, initialize_result()).await?;
            }
            "ping" => {
                send_result(&mut out, id, json!({})).await?;
            }
            "tools/list" => {
                send_result(&mut out, id, json!({ "tools": tool_catalog() })).await?;
            }
            "tools/call" => {
                let result = handle_tool_call(&params, client).await;
                send_result(&mut out, id, tool_result(result)).await?;
            }
            // Notifications (initialized, cancelled, …) require no response.
            m if m.starts_with("notifications/") => {}
            _ => {
                // Only requests (those carrying an id) get an error reply.
                if let Some(id) = id {
                    send_error(&mut out, id, -32601, &format!("method not found: {method}"))
                        .await?;
                }
            }
        }
    }
    Ok(())
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
    })
}

/// Wrap a tool outcome into an MCP `tools/call` result object.
fn tool_result(outcome: std::result::Result<String, String>) -> Value {
    match outcome {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }], "isError": false }),
        Err(msg) => json!({ "content": [{ "type": "text", "text": msg }], "isError": true }),
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC framing helpers
// ---------------------------------------------------------------------------

async fn write_line(out: &mut Stdout, v: &Value) -> Result<()> {
    let mut s = serde_json::to_string(v)?;
    s.push('\n');
    out.write_all(s.as_bytes()).await?;
    out.flush().await?;
    Ok(())
}

async fn send_result(out: &mut Stdout, id: Option<Value>, result: Value) -> Result<()> {
    // A response is only meaningful for a request (id present).
    let Some(id) = id else { return Ok(()) };
    write_line(
        out,
        &json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    )
    .await
}

async fn send_error(out: &mut Stdout, id: Value, code: i64, message: &str) -> Result<()> {
    write_line(
        out,
        &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    )
    .await
}

// ---------------------------------------------------------------------------
// Tool catalog (name, description, JSON-Schema for arguments)
// ---------------------------------------------------------------------------

fn tool_catalog() -> Vec<Value> {
    // Full route + filter schema, shared by `search` and `offer`.
    let search_props = json!({
        "from": { "type": "string", "description": "Departure IATA code or city name" },
        "to": { "type": "string", "description": "Destination IATA code or city name" },
        "date": { "type": "string", "description": "Departure date YYYY-MM-DD" },
        "return_date": { "type": "string", "description": "Return date YYYY-MM-DD (omit for one-way)" },
        "adults": { "type": "integer", "minimum": 1, "default": 1 },
        "children": { "type": "integer", "minimum": 0, "default": 0 },
        "infants_seat": { "type": "integer", "minimum": 0, "default": 0 },
        "infants_lap": { "type": "integer", "minimum": 0, "default": 0 },
        "class": { "type": "string", "enum": ["economy", "premium-economy", "business", "first"] },
        "stops": { "type": "string", "enum": ["all", "nonstop", "one-stop"] },
        "sort": { "type": "string", "enum": ["top-flights", "best", "price", "departure-time", "arrival-time", "duration", "emissions"] },
        "min_layover": { "type": "integer", "minimum": 0, "description": "Minimum layover in minutes" },
        "max_layover": { "type": "integer", "minimum": 0, "description": "Maximum layover in minutes" },
        "lower_emissions": { "type": "boolean", "default": false, "description": "Restrict to lower-CO₂ flights" },
        "airlines": { "type": "array", "items": { "type": "string" }, "description": "Include only these airlines/alliances (IATA code, or ONEWORLD/SKYTEAM/STAR_ALLIANCE)" },
        "exclude_airlines": { "type": "array", "items": { "type": "string" }, "description": "Exclude these airlines/alliances" },
        "via": { "type": "array", "items": { "type": "string" }, "description": "Require a connection through these IATA airport codes" },
        "exclude_basic": { "type": "boolean", "default": false, "description": "Exclude basic-economy fares" },
        "time": { "type": "string", "description": "Departure time window as HH-HH (24h), e.g. 6-22" },
        "arr_time": { "type": "string", "description": "Arrival time window as HH-HH" },
        "ret_time": { "type": "string", "description": "Return departure time window as HH-HH" },
        "ret_arr_time": { "type": "string", "description": "Return arrival time window as HH-HH" },
        "bags": { "type": "integer", "minimum": 0, "maximum": 2, "description": "Minimum checked bags included" },
        "carry_on": { "type": "integer", "minimum": 0, "maximum": 2, "description": "Minimum carry-on bags included" },
        "max_price": { "type": "integer", "description": "Maximum total price in the search currency" }
    });

    // Tool-specific options layered on top of the shared route/filter schema.
    let mut search_input = search_props.clone();
    search_input["priced_only"] = json!({
        "type": "boolean",
        "default": false,
        "description": "Omit itineraries returned without a bookable price"
    });
    let mut offer_input = search_props.clone();
    offer_input["open"] = json!({
        "type": "boolean",
        "default": false,
        "description": "Open the cheapest offer's booking URL in the default browser"
    });

    vec![
        json!({
            "name": "search",
            "description": "Search flights for a route and date (one-way or round-trip). Returns itineraries with price, stops, duration, and legs.",
            "inputSchema": { "type": "object", "properties": search_input, "required": ["from", "to", "date"] }
        }),
        json!({
            "name": "price_graph",
            "description": "Cheapest fare per departure day over N months for a route. Returns [{date, price}].",
            "inputSchema": {
                "type": "object",
                "properties": json!({
                    "from": { "type": "string" }, "to": { "type": "string" },
                    "date": { "type": "string", "description": "Start date YYYY-MM-DD" },
                    "months": { "type": "integer", "minimum": 1, "default": 3 },
                    "adults": { "type": "integer", "minimum": 1, "default": 1 },
                    "class": { "type": "string", "enum": ["economy", "premium-economy", "business", "first"] }
                }),
                "required": ["from", "to", "date"]
            }
        }),
        json!({
            "name": "cheapest_dates",
            "description": "Cheapest departure dates over N months. Set trip_days for round trips of that length; omit for one-way.",
            "inputSchema": {
                "type": "object",
                "properties": json!({
                    "from": { "type": "string" }, "to": { "type": "string" },
                    "date": { "type": "string", "description": "Earliest departure date YYYY-MM-DD" },
                    "months": { "type": "integer", "minimum": 1, "default": 3 },
                    "trip_days": { "type": "integer", "description": "Round-trip length in nights; omit for one-way" },
                    "adults": { "type": "integer", "minimum": 1, "default": 1 },
                    "class": { "type": "string", "enum": ["economy", "premium-economy", "business", "first"] }
                }),
                "required": ["from", "to", "date"]
            }
        }),
        json!({
            "name": "explore",
            "description": "Explore cheap destinations from an origin airport. Optional destination airport, travel month, duration, interest, budget, and traveler/cabin filters.",
            "inputSchema": {
                "type": "object",
                "properties": json!({
                    "from": { "type": "string", "description": "Origin IATA code" },
                    "to": { "type": "string", "description": "Optional destination IATA code" },
                    "month": { "type": "integer", "minimum": 1, "maximum": 12 },
                    "duration": { "type": "string", "enum": ["weekend", "week", "2-weeks"], "default": "week", "description": "Trip length" },
                    "interest": { "type": "string", "description": "Interest category name (e.g. beaches) or Knowledge-Graph MID (e.g. /m/01rwk)" },
                    "budget": { "type": "integer", "description": "Max total price in the chosen currency" },
                    "max_flight_hours": { "type": "integer", "minimum": 0, "description": "Max one-way flight duration in hours" },
                    "carry_on": { "type": "integer", "minimum": 0, "maximum": 2 },
                    "checked": { "type": "integer", "minimum": 0, "maximum": 2 },
                    "class": { "type": "string", "enum": ["economy", "premium-economy", "business", "first"] },
                    "adults": { "type": "integer", "minimum": 1, "default": 1 },
                    "children": { "type": "integer", "minimum": 0, "default": 0 },
                    "infants_seat": { "type": "integer", "minimum": 0, "default": 0 },
                    "infants_lap": { "type": "integer", "minimum": 0, "default": 0 }
                }),
                "required": ["from"]
            }
        }),
        json!({
            "name": "deals",
            "description": "Find discounted destinations from an origin (price vs typical price). out/ret define the trip-length anchor.",
            "inputSchema": {
                "type": "object",
                "properties": json!({
                    "from": { "type": "string", "description": "Origin IATA code" },
                    "out": { "type": "string", "description": "Outbound date YYYY-MM-DD" },
                    "ret": { "type": "string", "description": "Return date YYYY-MM-DD" },
                    "nonstop": { "type": "boolean", "default": false },
                    "max_hours": { "type": "integer", "description": "Max one-way duration in hours" },
                    "class": { "type": "string", "enum": ["economy", "premium-economy", "business", "first"] },
                    "adults": { "type": "integer", "minimum": 1, "default": 1 },
                    "children": { "type": "integer", "minimum": 0, "default": 0 },
                    "infants_seat": { "type": "integer", "minimum": 0, "default": 0 },
                    "infants_lap": { "type": "integer", "minimum": 0, "default": 0 }
                }),
                "required": ["from", "out", "ret"]
            }
        }),
        json!({
            "name": "date_grid",
            "description": "Price grid over a range of departure and return dates. Returns the cheapest fare for each (departure, return) date pair.",
            "inputSchema": {
                "type": "object",
                "properties": json!({
                    "from": { "type": "string", "description": "Departure IATA code or city name" },
                    "to": { "type": "string", "description": "Destination IATA code or city name" },
                    "dep_start": { "type": "string", "description": "Earliest departure date YYYY-MM-DD" },
                    "dep_end": { "type": "string", "description": "Latest departure date YYYY-MM-DD" },
                    "ret_start": { "type": "string", "description": "Earliest return date YYYY-MM-DD" },
                    "ret_end": { "type": "string", "description": "Latest return date YYYY-MM-DD" },
                    "adults": { "type": "integer", "minimum": 1, "default": 1 },
                    "class": { "type": "string", "enum": ["economy", "premium-economy", "business", "first"] },
                    "stops": { "type": "string", "enum": ["all", "nonstop", "one-stop"] }
                }),
                "required": ["from", "to", "dep_start", "dep_end", "ret_start", "ret_end"]
            }
        }),
        json!({
            "name": "offer",
            "description": "Booking offers for the cheapest itinerary on a route (one-way or round-trip): airlines, total price, and a resolved booking URL per channel. Accepts the same filters as search.",
            "inputSchema": { "type": "object", "properties": offer_input, "required": ["from", "to", "date"] }
        }),
        json!({
            "name": "multi_city",
            "description": "Multi-city / open-jaw search across 2 or more legs. Returns itineraries with price, stops, duration, and legs.",
            "inputSchema": {
                "type": "object",
                "properties": json!({
                    "legs": {
                        "type": "array",
                        "minItems": 2,
                        "description": "Ordered flight legs (2 or more)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "from": { "type": "string", "description": "Departure IATA code or city name" },
                                "to": { "type": "string", "description": "Destination IATA code or city name" },
                                "date": { "type": "string", "description": "Departure date YYYY-MM-DD" }
                            },
                            "required": ["from", "to", "date"]
                        }
                    },
                    "adults": { "type": "integer", "minimum": 1, "default": 1 },
                    "children": { "type": "integer", "minimum": 0, "default": 0 },
                    "infants_seat": { "type": "integer", "minimum": 0, "default": 0 },
                    "infants_lap": { "type": "integer", "minimum": 0, "default": 0 },
                    "class": { "type": "string", "enum": ["economy", "premium-economy", "business", "first"] },
                    "sort": { "type": "string", "enum": ["top-flights", "best", "price", "departure-time", "arrival-time", "duration", "emissions"] },
                    "max_price": { "type": "integer", "description": "Maximum total price in the search currency" },
                    "bags": { "type": "integer", "minimum": 0, "maximum": 2 },
                    "carry_on": { "type": "integer", "minimum": 0, "maximum": 2 }
                }),
                "required": ["legs"]
            }
        }),
    ]
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

async fn handle_tool_call(
    params: &Value,
    client: &ApiClient,
) -> std::result::Result<String, String> {
    let name = params
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or_else(|| "tools/call missing 'name'".to_string())?;
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);

    match name {
        "search" => tool_search(&args, client).await,
        "price_graph" => tool_price_graph(&args, client).await,
        "cheapest_dates" => tool_cheapest_dates(&args, client).await,
        "explore" => tool_explore(&args, client).await,
        "deals" => tool_deals(&args, client).await,
        "date_grid" => tool_date_grid(&args, client).await,
        "offer" => tool_offer(&args, client).await,
        "multi_city" => tool_multi_city(&args, client).await,
        other => Err(format!("unknown tool: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn req_str(args: &Value, key: &str) -> std::result::Result<String, String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("missing or non-string argument: {key}"))
}

fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

fn opt_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|n| n as u32)
}

fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

/// Map an optional `duration` argument to an explore trip length, defaulting to
/// one week (matching the CLI `explore` default).
fn parse_duration(args: &Value) -> ExploreDuration {
    match opt_str(args, "duration").as_deref() {
        Some("weekend") => ExploreDuration::Weekend,
        Some("2-weeks") | Some("two-weeks") => ExploreDuration::TwoWeeks,
        _ => ExploreDuration::OneWeek,
    }
}

fn parse_date(s: &str) -> std::result::Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("invalid date {s:?}: {e}"))
}

fn parse_class(s: &str) -> std::result::Result<TravelClass, String> {
    match s.to_lowercase().as_str() {
        "economy" | "eco" => Ok(TravelClass::Economy),
        "premium-economy" | "premium_economy" => Ok(TravelClass::PremiumEconomy),
        "business" | "biz" => Ok(TravelClass::Business),
        "first" => Ok(TravelClass::First),
        _ => Err(format!("unknown class {s:?}")),
    }
}

fn parse_stops(s: &str) -> std::result::Result<StopOptions, String> {
    match s.to_lowercase().as_str() {
        "all" | "any" => Ok(StopOptions::All),
        "nonstop" | "non-stop" | "direct" => Ok(StopOptions::NoStop),
        "one-stop" | "one_stop" | "onestop" => Ok(StopOptions::OneOrLess),
        _ => Err(format!("unknown stops {s:?}")),
    }
}

/// Build `Travelers` from `adults` / `children` / `infants_seat` / `infants_lap`
/// arguments (missing counts default to 0, adults to 1). The `Travelers::new`
/// order is `[adults, children, infant_on_lap, infant_in_seat]`.
fn travelers_from(args: &Value) -> std::result::Result<Travelers, String> {
    let adults = opt_u32(args, "adults").unwrap_or(1) as i32;
    let children = opt_u32(args, "children").unwrap_or(0) as i32;
    let infants_lap = opt_u32(args, "infants_lap").unwrap_or(0) as i32;
    let infants_seat = opt_u32(args, "infants_seat").unwrap_or(0) as i32;
    Travelers::new(vec![adults, children, infants_lap, infants_seat]).map_err(|e| e.to_string())
}

fn opt_str_array(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_airlines(items: &[String]) -> std::result::Result<Vec<AirlineFilter>, String> {
    items
        .iter()
        .map(|s| {
            s.parse::<AirlineFilter>()
                .map_err(|e| format!("invalid airline {s:?}: {e}"))
        })
        .collect()
}

fn parse_sort(s: &str) -> std::result::Result<SortOrder, String> {
    SortOrder::from_str(s, true).map_err(|e| format!("unknown sort {s:?}: {e}"))
}

/// Parse a `"HH-HH"` 24-hour window into `(min_hour, max_hour)`.
fn parse_time_window(s: &str) -> std::result::Result<(u32, u32), String> {
    let (lo, hi) = s
        .split_once('-')
        .ok_or_else(|| format!("invalid time window {s:?}, expected HH-HH"))?;
    let lo = lo
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid time window {s:?}, expected HH-HH"))?;
    let hi = hi
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("invalid time window {s:?}, expected HH-HH"))?;
    Ok((lo, hi))
}

/// Assemble a `FlightTimes` from an optional departure window (`dep_key`) and an
/// optional arrival window (`arr_key`). Returns `None` when neither is present;
/// a missing side is left unrestricted (`0, 0`).
fn flight_times(
    args: &Value,
    dep_key: &str,
    arr_key: &str,
) -> std::result::Result<Option<FlightTimes>, String> {
    let dep = opt_str(args, dep_key);
    let arr = opt_str(args, arr_key);
    if dep.is_none() && arr.is_none() {
        return Ok(None);
    }
    let (dmin, dmax) = match dep {
        Some(s) => parse_time_window(&s)?,
        None => (0, 0),
    };
    let (amin, amax) = match arr {
        Some(s) => parse_time_window(&s)?,
        None => (0, 0),
    };
    Ok(Some(FlightTimes::new(dmin, dmax, amin, amax)))
}

/// Build a route `Config` from the common argument set shared by search,
/// price_graph, and cheapest_dates. `with_return` controls whether a
/// `return_date` argument is honoured.
async fn build_route_config(
    args: &Value,
    client: &ApiClient,
    with_return: bool,
) -> std::result::Result<Config, String> {
    let from = req_str(args, "from")?;
    let to = req_str(args, "to")?;
    let date = parse_date(&req_str(args, "date")?)?;

    let mut b = Config::builder()
        .departure(&from, client)
        .await
        .map_err(|e| e.to_string())?
        .destination(&to, client)
        .await
        .map_err(|e| e.to_string())?
        .departing_date(date)
        .travelers(travelers_from(args)?);

    if let Some(c) = opt_str(args, "class") {
        b = b.travel_class(parse_class(&c)?);
    }
    if let Some(s) = opt_str(args, "stops") {
        b = b.stop_options(parse_stops(&s)?);
    }
    if with_return {
        if let Some(ret) = opt_str(args, "return_date") {
            b = b.return_date(parse_date(&ret)?);
        }
    }

    // Optional filters — applied only when the argument is present.
    if let Some(s) = opt_str(args, "sort") {
        b = b.sort_order(parse_sort(&s)?);
    }
    if let Some(m) = opt_u32(args, "min_layover") {
        b = b.stopover_min(StopoverDuration::Minutes(m));
    }
    if let Some(m) = opt_u32(args, "max_layover") {
        b = b.stopover_max(StopoverDuration::Minutes(m));
    }
    if args
        .get("lower_emissions")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        b = b.lower_emissions(true);
    }
    let airlines = opt_str_array(args, "airlines");
    if !airlines.is_empty() {
        b = b.airlines_include(parse_airlines(&airlines)?);
    }
    let exclude_airlines = opt_str_array(args, "exclude_airlines");
    if !exclude_airlines.is_empty() {
        b = b.airlines_exclude(parse_airlines(&exclude_airlines)?);
    }
    let via = opt_str_array(args, "via");
    if !via.is_empty() {
        b = b.connecting_airports(via);
    }
    if args
        .get("exclude_basic")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        b = b.exclude_basic_economy(true);
    }
    if let Some(t) = flight_times(args, "time", "arr_time")? {
        b = b.departing_times(t);
    }
    if let Some(t) = flight_times(args, "ret_time", "ret_arr_time")? {
        b = b.return_times(t);
    }
    let bags = opt_u32(args, "bags");
    let carry_on = opt_u32(args, "carry_on");
    if bags.is_some() || carry_on.is_some() {
        b = b.baggage(carry_on.unwrap_or(0) as u8, bags.unwrap_or(0) as u8);
    }
    if let Some(p) = args.get("max_price").and_then(|v| v.as_i64()) {
        b = b.max_price(p as i32);
    }

    b.build().map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tool implementations
// ---------------------------------------------------------------------------

async fn tool_search(args: &Value, client: &ApiClient) -> std::result::Result<String, String> {
    let config = build_route_config(args, client, true).await?;
    let res = client
        .request_flights(&config)
        .await
        .map_err(|e| e.to_string())?;
    let mut flights = res.get_all_flights();
    if opt_bool(args, "priced_only").unwrap_or(false) {
        flights.retain(|f| f.itinerary_cost.trip_cost.is_some());
    }
    serde_json::to_string(&flights).map_err(|e| e.to_string())
}

async fn tool_price_graph(args: &Value, client: &ApiClient) -> std::result::Result<String, String> {
    let config = build_route_config(args, client, false).await?;
    let months = Months::new(opt_u32(args, "months").unwrap_or(3));
    let graph = client
        .request_graph(&config, months)
        .await
        .map_err(|e| e.to_string())?;
    let mut points: Vec<_> = graph
        .get_all_graphs()
        .into_iter()
        .filter_map(|g| g.maybe_get_date_price())
        .map(|(d, p)| json!({ "date": d.to_string(), "price": p }))
        .collect();
    points.sort_by(|a, b| a["date"].as_str().cmp(&b["date"].as_str()));
    serde_json::to_string(&points).map_err(|e| e.to_string())
}

async fn tool_cheapest_dates(
    args: &Value,
    client: &ApiClient,
) -> std::result::Result<String, String> {
    let config = build_route_config(args, client, false).await?;
    let months = Months::new(opt_u32(args, "months").unwrap_or(3));
    let trip_days = opt_u32(args, "trip_days");
    let results = client
        .cheapest_dates(&config, months, trip_days)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&results).map_err(|e| e.to_string())
}

async fn tool_explore(args: &Value, client: &ApiClient) -> std::result::Result<String, String> {
    let from = req_str(args, "from")?;
    let origin = Location {
        loc_identifier: from.to_uppercase(),
        loc_type: PlaceType::Airport,
        location_name: None,
    };
    let destination = opt_str(args, "to").map(|t| Location {
        loc_identifier: t.to_uppercase(),
        loc_type: PlaceType::Airport,
        location_name: None,
    });
    let trip_date = opt_u32(args, "month").map(|m| ExploreDate { month: m as u8 });
    let interest = match opt_str(args, "interest").as_deref() {
        Some(raw) => Some(resolve_interest(raw).map_err(|e| e.to_string())?),
        None => None,
    };
    let baggage = match (opt_u32(args, "carry_on"), opt_u32(args, "checked")) {
        (None, None) => None,
        (c, k) => Some((c.unwrap_or(0) as u8, k.unwrap_or(0) as u8)),
    };
    let travel_class = match opt_str(args, "class") {
        Some(c) => parse_class(&c)?,
        None => TravelClass::Economy,
    };

    let config = ExploreConfig {
        origin: vec![origin],
        destination,
        trip_date,
        trip_duration: parse_duration(args),
        max_price: opt_u32(args, "budget").map(|b| b as i32),
        interest,
        max_flight_duration_minutes: opt_u32(args, "max_flight_hours").map(|h| h * 60),
        baggage,
        travellers: travelers_from(args)?,
        travel_class,
        ..Default::default()
    };

    let mut results = client
        .request_explore(&config)
        .await
        .map_err(|e| e.to_string())?;
    results.retain(|r| r.price.is_some());
    serde_json::to_string(&results).map_err(|e| e.to_string())
}

async fn tool_deals(args: &Value, client: &ApiClient) -> std::result::Result<String, String> {
    let from = req_str(args, "from")?;
    let out = parse_date(&req_str(args, "out")?)?;
    let ret = parse_date(&req_str(args, "ret")?)?;
    let class = match opt_str(args, "class") {
        Some(c) => parse_class(&c)?,
        None => TravelClass::Economy,
    };
    let origin = Location {
        loc_identifier: from.to_uppercase(),
        loc_type: PlaceType::Airport,
        location_name: None,
    };
    let config = DealConfig {
        origin: vec![origin],
        outbound_date: out,
        return_date: ret,
        nonstop: args
            .get("nonstop")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        max_duration_minutes: opt_u32(args, "max_hours").map(|h| h * 60),
        travel_class: class,
        travellers: travelers_from(args)?,
    };
    let deals = client
        .request_deals(&config)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&deals).map_err(|e| e.to_string())
}

async fn tool_date_grid(args: &Value, client: &ApiClient) -> std::result::Result<String, String> {
    let from = req_str(args, "from")?;
    let to = req_str(args, "to")?;
    let dep_start = parse_date(&req_str(args, "dep_start")?)?;
    let dep_end = parse_date(&req_str(args, "dep_end")?)?;
    let ret_start = parse_date(&req_str(args, "ret_start")?)?;
    let ret_end = parse_date(&req_str(args, "ret_end")?)?;

    let mut b = Config::builder()
        .departure(&from, client)
        .await
        .map_err(|e| e.to_string())?
        .destination(&to, client)
        .await
        .map_err(|e| e.to_string())?
        .departing_date(dep_start)
        .return_date(ret_end)
        .travelers(travelers_from(args)?);
    if let Some(c) = opt_str(args, "class") {
        b = b.travel_class(parse_class(&c)?);
    }
    if let Some(s) = opt_str(args, "stops") {
        b = b.stop_options(parse_stops(&s)?);
    }
    let config = b.build().map_err(|e| e.to_string())?;

    let grid = client
        .request_date_grid(&config, dep_start, dep_end, ret_start, ret_end)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&grid).map_err(|e| e.to_string())
}

async fn tool_offer(args: &Value, client: &ApiClient) -> std::result::Result<String, String> {
    let config = build_route_config(args, client, true).await?;

    let result = client
        .request_flights(&config)
        .await
        .map_err(|e| e.to_string())?;
    let first = result
        .get_all_flights()
        .into_iter()
        .next()
        .ok_or_else(|| "no flights found to price".to_string())?;
    config
        .fixed_flights
        .add_element(first)
        .map_err(|e| e.to_string())?;

    // Round trip: also lock in the cheapest return leg.
    if config.return_date.is_some() {
        let second = client
            .request_flights(&config)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(ret) = second.get_all_flights().into_iter().next() {
            config
                .fixed_flights
                .add_element(ret)
                .map_err(|e| e.to_string())?;
        }
    }

    let offers = client
        .request_offer(&config)
        .await
        .map_err(|e| e.to_string())?;

    // Flatten to priced offers (cheapest first) and resolve each booking URL so
    // clients receive clickable links instead of opaque click tokens.
    let mut groups: Vec<_> = offers
        .response
        .iter()
        .flat_map(|r| &r.offers)
        .filter(|o| o.price.is_some())
        .collect();
    groups.sort_by_key(|o| o.price.unwrap_or(i32::MAX));

    let mut enriched: Vec<Value> = Vec::new();
    for o in &groups {
        let booking_url = match o.click_token.as_deref() {
            Some(token) => client.resolve_booking_url(token).await.ok(),
            None => None,
        };
        enriched.push(json!({
            "airline_names": o.airline_names,
            "price": o.price,
            "booking_url": booking_url,
        }));
    }

    // Optionally open the cheapest resolved URL in the host's default browser.
    // Notes go to stderr so the JSON-RPC stream on stdout stays clean.
    if opt_bool(args, "open").unwrap_or(false) {
        if let Some(url) = enriched.first().and_then(|o| o["booking_url"].as_str()) {
            eprintln!("Opening cheapest booking URL in your browser…");
            if let Err(e) = webbrowser::open(url) {
                eprintln!("could not open browser: {e}");
            }
        }
    }

    serde_json::to_string(&enriched).map_err(|e| e.to_string())
}

async fn tool_multi_city(args: &Value, client: &ApiClient) -> std::result::Result<String, String> {
    let legs = args
        .get("legs")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing 'legs' array".to_string())?;
    if legs.len() < 2 {
        return Err("multi_city requires at least 2 legs".to_string());
    }

    let mut builder = MultiCityConfig::builder().travellers(travelers_from(args)?);
    if let Some(c) = opt_str(args, "class") {
        builder = builder.travel_class(parse_class(&c)?);
    }
    if let Some(s) = opt_str(args, "sort") {
        builder = builder.sort_order(parse_sort(&s)?);
    }
    if let Some(p) = args.get("max_price").and_then(|v| v.as_i64()) {
        builder = builder.max_price(p as i32);
    }
    let bags = opt_u32(args, "bags");
    let carry_on = opt_u32(args, "carry_on");
    if bags.is_some() || carry_on.is_some() {
        builder = builder.baggage(carry_on.unwrap_or(0) as u8, bags.unwrap_or(0) as u8);
    }

    for (i, leg) in legs.iter().enumerate() {
        let from = req_str(leg, "from").map_err(|e| format!("leg {i}: {e}"))?;
        let to = req_str(leg, "to").map_err(|e| format!("leg {i}: {e}"))?;
        let date = parse_date(&req_str(leg, "date").map_err(|e| format!("leg {i}: {e}"))?)?;
        builder = builder
            .add_leg(&from, &to, date, client)
            .await
            .map_err(|e| e.to_string())?;
    }

    let config = builder.build().map_err(|e| e.to_string())?;
    let flights = client
        .request_multi_city_flights(&config)
        .await
        .map_err(|e| e.to_string())?;
    serde_json::to_string(&flights.get_all_flights()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_catalog_has_expected_tools() {
        let names: Vec<String> = tool_catalog()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"search".to_string()));
        assert!(names.contains(&"price_graph".to_string()));
        assert!(names.contains(&"cheapest_dates".to_string()));
        assert!(names.contains(&"explore".to_string()));
        assert!(names.contains(&"deals".to_string()));
        assert!(names.contains(&"date_grid".to_string()));
        assert!(names.contains(&"offer".to_string()));
        assert!(names.contains(&"multi_city".to_string()));
    }

    #[test]
    fn search_schema_advertises_full_filter_set() {
        let search = tool_catalog()
            .into_iter()
            .find(|t| t["name"] == "search")
            .expect("search tool present");
        let props = &search["inputSchema"]["properties"];
        for key in [
            "sort",
            "min_layover",
            "max_layover",
            "lower_emissions",
            "airlines",
            "exclude_airlines",
            "via",
            "exclude_basic",
            "time",
            "arr_time",
            "ret_time",
            "ret_arr_time",
            "bags",
            "carry_on",
            "max_price",
            "priced_only",
            "children",
            "infants_seat",
            "infants_lap",
        ] {
            assert!(props.get(key).is_some(), "search schema missing {key}");
        }
    }

    #[test]
    fn multi_city_schema_requires_legs_array() {
        let mc = tool_catalog()
            .into_iter()
            .find(|t| t["name"] == "multi_city")
            .expect("multi_city tool present");
        assert_eq!(
            mc["inputSchema"]["properties"]["legs"]["type"].as_str(),
            Some("array")
        );
        assert_eq!(mc["inputSchema"]["required"][0].as_str(), Some("legs"));
    }

    #[test]
    fn every_tool_has_name_description_and_schema() {
        for t in tool_catalog() {
            assert!(t["name"].as_str().is_some());
            assert!(t["description"].as_str().is_some());
            assert_eq!(t["inputSchema"]["type"].as_str(), Some("object"));
            assert!(t["inputSchema"]["properties"].is_object());
        }
    }

    #[test]
    fn initialize_result_advertises_tools_capability() {
        let r = initialize_result();
        assert_eq!(r["protocolVersion"].as_str(), Some(PROTOCOL_VERSION));
        assert!(r["capabilities"]["tools"].is_object());
        assert_eq!(r["serverInfo"]["name"].as_str(), Some(SERVER_NAME));
    }

    #[test]
    fn tool_result_marks_errors() {
        let ok = tool_result(Ok("[]".into()));
        assert_eq!(ok["isError"].as_bool(), Some(false));
        assert_eq!(ok["content"][0]["text"].as_str(), Some("[]"));

        let err = tool_result(Err("boom".into()));
        assert_eq!(err["isError"].as_bool(), Some(true));
        assert_eq!(err["content"][0]["text"].as_str(), Some("boom"));
    }

    #[test]
    fn parse_helpers_validate_input() {
        assert!(parse_date("2026-09-15").is_ok());
        assert!(parse_date("nope").is_err());
        assert!(parse_class("business").is_ok());
        assert!(parse_class("zzz").is_err());
        assert!(parse_stops("nonstop").is_ok());
        assert!(parse_stops("zzz").is_err());
        assert!(parse_sort("emissions").is_ok());
        assert!(parse_sort("top-flights").is_ok());
        assert!(parse_sort("zzz").is_err());
        assert_eq!(parse_time_window("6-22").unwrap(), (6, 22));
        assert!(parse_time_window("nope").is_err());
        assert!(parse_airlines(&["LX".to_string(), "ONEWORLD".to_string()]).is_ok());
        assert!(parse_airlines(&["".to_string()]).is_err());
    }

    #[test]
    fn travelers_from_reads_all_counts() {
        let v = json!({ "adults": 2, "children": 1, "infants_seat": 1 });
        let t = travelers_from(&v).unwrap();
        assert_eq!(t.adults, 2);
        assert_eq!(t.children, 1);
        assert_eq!(t.infant_in_seat, 1);
        // Missing adults defaults to 1.
        assert_eq!(travelers_from(&json!({})).unwrap().adults, 1);
    }

    #[test]
    fn opt_str_array_reads_string_lists() {
        let v = json!({ "via": ["ZRH", "MUC"] });
        assert_eq!(opt_str_array(&v, "via"), vec!["ZRH", "MUC"]);
        assert!(opt_str_array(&v, "missing").is_empty());
    }

    #[test]
    fn req_str_reports_missing() {
        let v = json!({ "from": "LHR" });
        assert_eq!(req_str(&v, "from").unwrap(), "LHR");
        assert!(req_str(&v, "to").is_err());
    }

    #[test]
    fn tool_specific_options_land_on_correct_tools() {
        let cat = tool_catalog();
        let search = cat
            .iter()
            .find(|t| t["name"] == "search")
            .expect("search tool present");
        let offer = cat
            .iter()
            .find(|t| t["name"] == "offer")
            .expect("offer tool present");
        let sp = &search["inputSchema"]["properties"];
        let op = &offer["inputSchema"]["properties"];
        // priced_only is search-only.
        assert!(
            sp.get("priced_only").is_some(),
            "search should advertise priced_only"
        );
        assert!(
            op.get("priced_only").is_none(),
            "offer should not advertise priced_only"
        );
        // open is offer-only.
        assert!(op.get("open").is_some(), "offer should advertise open");
        assert!(sp.get("open").is_none(), "search should not advertise open");
    }

    #[test]
    fn opt_bool_reads_flags() {
        assert_eq!(opt_bool(&json!({ "flag": true }), "flag"), Some(true));
        assert_eq!(opt_bool(&json!({ "flag": false }), "flag"), Some(false));
        assert_eq!(opt_bool(&json!({}), "flag"), None);
        // Non-boolean values are not coerced.
        assert_eq!(opt_bool(&json!({ "flag": "yes" }), "flag"), None);
    }

    #[test]
    fn route_tools_advertise_class_and_travelers() {
        let cat = tool_catalog();
        let pg = cat
            .iter()
            .find(|t| t["name"] == "price_graph")
            .expect("price_graph tool present");
        let cd = cat
            .iter()
            .find(|t| t["name"] == "cheapest_dates")
            .expect("cheapest_dates tool present");
        let dl = cat
            .iter()
            .find(|t| t["name"] == "deals")
            .expect("deals tool present");
        assert!(
            pg["inputSchema"]["properties"].get("class").is_some(),
            "price_graph should advertise class"
        );
        assert!(
            cd["inputSchema"]["properties"].get("class").is_some(),
            "cheapest_dates should advertise class"
        );
        for k in ["children", "infants_seat", "infants_lap"] {
            assert!(
                dl["inputSchema"]["properties"].get(k).is_some(),
                "deals should advertise {k}"
            );
        }
    }

    #[test]
    fn explore_schema_advertises_full_filter_set() {
        let ex = tool_catalog()
            .into_iter()
            .find(|t| t["name"] == "explore")
            .expect("explore tool present");
        let props = &ex["inputSchema"]["properties"];
        for key in [
            "duration",
            "interest",
            "max_flight_hours",
            "carry_on",
            "checked",
            "class",
            "children",
            "infants_seat",
            "infants_lap",
        ] {
            assert!(props.get(key).is_some(), "explore schema missing {key}");
        }
    }

    #[test]
    fn parse_duration_maps_strings() {
        assert_eq!(parse_duration(&json!({})).as_wire_code(), 2); // default = 1 week
        assert_eq!(
            parse_duration(&json!({ "duration": "weekend" })).as_wire_code(),
            1
        );
        assert_eq!(
            parse_duration(&json!({ "duration": "2-weeks" })).as_wire_code(),
            3
        );
        assert_eq!(
            parse_duration(&json!({ "duration": "week" })).as_wire_code(),
            2
        );
    }
}
