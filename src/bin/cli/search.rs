use anyhow::{anyhow, Result};
use clap::Parser;
use gflights::parsers::common::{AirlineFilter, FlightTimes, SortOrder, StopoverDuration};
use gflights::parsers::flight_response::ItineraryContainer;
use gflights::requests::api::ApiClient;

use super::{build_config, CommonArgs, OutputFormat};
use gflights::requests::config::Config;

/// Parse an `H-H` 24-hour-clock time window into a from-to hour pair.
fn parse_time_window(s: &str) -> Result<(u32, u32)> {
    let (from, to) = s
        .split_once('-')
        .ok_or_else(|| anyhow!("time window must be H-H in 24h format, e.g. 6-20: {s}"))?;
    let from: u32 = from.trim().parse()?;
    let to: u32 = to.trim().parse()?;
    if from > 23 || to > 23 || from > to {
        return Err(anyhow!(
            "time window hours must be 0-23 with FROM <= TO: {s}"
        ));
    }
    Ok((from, to))
}

/// Arguments for the `search` subcommand.
#[derive(Parser, Debug)]
pub struct SearchArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// Sort order.
    #[arg(long, default_value = "best")]
    pub sort: SortOrder,

    /// Minimum layover duration in minutes (rounded up to the next 30 min interval).
    #[arg(long)]
    pub min_layover: Option<u32>,

    /// Maximum layover duration in minutes (rounded up to the next 30 min interval).
    #[arg(long)]
    pub max_layover: Option<u32>,

    /// Restrict results to lower-CO₂ emissions flights.
    #[arg(long)]
    pub lower_emissions: bool,

    /// Airline IATA code (e.g. LX, LH) or alliance name (ONEWORLD, SKYTEAM, STAR_ALLIANCE)
    /// to include. May be repeated for multiple airlines/alliances.
    #[arg(long = "airline")]
    pub airlines: Vec<AirlineFilter>,

    /// Airline IATA code or alliance name to exclude.
    /// May be repeated for multiple airlines/alliances.
    #[arg(long = "exclude-airline")]
    pub exclude_airlines: Vec<AirlineFilter>,

    /// Require a connection through this IATA airport code (e.g. CDG, AMS).
    /// May be repeated for multiple airports.
    #[arg(long = "via")]
    pub connecting_airports: Vec<String>,

    /// Show a CO₂ emissions column (kg per passenger).
    #[arg(long = "show-co2")]
    pub show_co2: bool,

    /// Show detailed info: layover airports and +1 marker for next-day arrivals.
    #[arg(long)]
    pub detail: bool,

    /// Exclude basic-economy fares from results.
    #[arg(long)]
    pub exclude_basic: bool,

    /// Outbound departure time window in 24h format, for example 6-20.
    #[arg(long)]
    pub time: Option<String>,

    /// Outbound arrival time window in 24h format, for example 8-22.
    #[arg(long)]
    pub arr_time: Option<String>,

    /// Return-leg departure time window in 24h format. Round trips only.
    #[arg(long)]
    pub ret_time: Option<String>,

    /// Return-leg arrival time window in 24h format. Round trips only.
    #[arg(long)]
    pub ret_arr_time: Option<String>,

    /// Number of checked bags, 0 to 2, to include in the displayed price.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=2))]
    pub bags: Option<u8>,

    /// Number of carry-on bags, 0 to 2, to include in the displayed price.
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=2))]
    pub carry_on: Option<u8>,

    /// Maximum total price in the result currency.
    #[arg(long)]
    pub max_price: Option<i32>,

    /// Omit itineraries Google returned without a bookable price.
    ///
    /// Off by default: unpriced rows still carry schedule information, so they
    /// are kept (rendered as "—", sorted last under `--sort price`). Set this
    /// for a clean, fully-priced list when piping or building fare tables.
    #[arg(long)]
    pub priced_only: bool,

    /// Print the browsable Google Flights URL for this search and exit.
    ///
    /// No API call is made — the URL is built from the route, dates, travelers,
    /// and cabin. Pipe it to `xdg-open`/`open` to launch Google Flights.
    #[arg(long)]
    pub web_url: bool,
}

pub async fn cmd_search(args: SearchArgs, client: &ApiClient) -> Result<()> {
    let mut config = build_config(&args.common, client)
        .await?
        .with_sort_order(args.sort);

    // Short-circuit: print the browsable Google Flights URL and stop, without
    // hitting the search API.
    if args.web_url {
        println!("{}", config.to_flight_url());
        return Ok(());
    }

    // Apply filter flags that live on SearchArgs rather than CommonArgs.
    config.airlines_include = args.airlines;
    config.airlines_exclude = args.exclude_airlines;
    config.connecting_airports = args.connecting_airports;
    config.lower_emissions = args.lower_emissions;
    if let Some(mins) = args.min_layover {
        config.stopover_min = StopoverDuration::Minutes(mins);
    }
    if let Some(mins) = args.max_layover {
        config.stopover_max = StopoverDuration::Minutes(mins);
    }
    config.exclude_basic_economy = args.exclude_basic;

    // Time-of-day windows. FlightTimes treats 0 as "no bound", which matches
    // the "whole day" default when only one side of a window is given.
    if args.time.is_some() || args.arr_time.is_some() {
        let (dep_from, dep_to) = args.time.as_deref().map_or(Ok((0, 0)), parse_time_window)?;
        let (arr_from, arr_to) = args
            .arr_time
            .as_deref()
            .map_or(Ok((0, 0)), parse_time_window)?;
        config.departing_times = FlightTimes::new(dep_from, dep_to, arr_from, arr_to);
    }
    if args.ret_time.is_some() || args.ret_arr_time.is_some() {
        if args.common.r#return.is_none() {
            return Err(anyhow!(
                "--ret-time/--ret-arr-time require a round trip via --return"
            ));
        }
        let (dep_from, dep_to) = args
            .ret_time
            .as_deref()
            .map_or(Ok((0, 0)), parse_time_window)?;
        let (arr_from, arr_to) = args
            .ret_arr_time
            .as_deref()
            .map_or(Ok((0, 0)), parse_time_window)?;
        config.return_times = FlightTimes::new(dep_from, dep_to, arr_from, arr_to);
    }

    // Baggage-inclusive pricing and the price cap.
    if args.bags.is_some() || args.carry_on.is_some() {
        config.baggage = Some((args.carry_on.unwrap_or(0), args.bags.unwrap_or(0)));
    }
    config.max_price = args.max_price;

    let results = client.request_flights(&config).await?;
    // Strict "via": Google's other_flights container leaks non-stops that skip
    // the requested connecting airport, so filter client-side.
    let mut flights = results.get_all_flights_via(&config.connecting_airports);

    // Client-side sort — guarantees the requested order regardless of what
    // Google returns.  `Best` and `TopFlights` keep Google's own ordering.
    match args.sort {
        SortOrder::Best | SortOrder::TopFlights => {}
        SortOrder::Emissions => {
            flights.sort_by_key(|f| {
                f.itinerary
                    .emissions
                    .as_ref()
                    .and_then(|e| e.co2_this_flight_g)
                    .unwrap_or(i64::MAX)
            });
        }
        SortOrder::Price => {
            flights.sort_by_key(|f| {
                f.itinerary_cost
                    .trip_cost
                    .as_ref()
                    .map(|c| c.price)
                    .unwrap_or(i32::MAX)
            });
        }
        SortOrder::Duration => {
            flights.sort_by_key(|f| f.itinerary.total_time_minutes);
        }
        SortOrder::DepartureTime => {
            flights.sort_by_key(|f| {
                f.itinerary
                    .flight_details
                    .first()
                    .map(|d| d.departure_time.hour.unwrap_or(0) * 60 + d.departure_time.minute)
            });
        }
        SortOrder::ArrivalTime => {
            flights.sort_by_key(|f| {
                f.itinerary
                    .flight_details
                    .last()
                    .map(|d| d.arrival_time.hour.unwrap_or(0) * 60 + d.arrival_time.minute)
            });
        }
    }

    // Drop itineraries Google returned without a bookable price. Applied after
    // the sort so the priced rows keep their order; done before the empty check
    // so a window where everything was unpriced still reports "No flights found".
    if args.priced_only {
        retain_priced_only(&mut flights);
    }

    if flights.is_empty() {
        eprintln!("No flights found.");
        return Ok(());
    }

    match args.common.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&flights)?);
        }
        OutputFormat::Table => {
            // Build header dynamically depending on flags.
            if args.show_co2 {
                println!(
                    "{:<8}  {:>6}  {:>5}  {:>5}  {:>7}  ROUTE",
                    "AIRLINE", "PRICE", "STOPS", "MINS", "CO2(kg)"
                );
            } else {
                println!(
                    "{:<8}  {:>6}  {:>5}  {:>5}  ROUTE",
                    "AIRLINE", "PRICE", "STOPS", "MINS"
                );
            }
            println!("{}", "-".repeat(if args.show_co2 { 70 } else { 60 }));

            for f in &flights {
                let price = f
                    .itinerary_cost
                    .trip_cost
                    .as_ref()
                    .map(|c| c.price.to_string())
                    .unwrap_or_else(|| "—".into());
                let from = f
                    .itinerary
                    .flight_details
                    .first()
                    .map(|d| d.departure_airport_code.as_str())
                    .unwrap_or("?");
                let to = f
                    .itinerary
                    .flight_details
                    .last()
                    .map(|d| d.destination_airport_code.as_str())
                    .unwrap_or("?");

                // "+1" marker when the final leg arrives the calendar day after departure.
                let next_day = if args.detail && f.itinerary.arrives_next_day() {
                    " +1"
                } else {
                    ""
                };

                let route = format!("{}→{}{}", from, to, next_day);

                if args.show_co2 {
                    let co2_str = f
                        .itinerary
                        .emissions
                        .as_ref()
                        .and_then(|e| e.co2_this_flight_g)
                        .map(|g| format!("{}", g / 1000))
                        .unwrap_or_else(|| "—".into());
                    println!(
                        "{:<8}  {:>6}  {:>5}  {:>5}  {:>7}  {}",
                        f.itinerary.flight_by,
                        price,
                        f.itinerary.stop_count(),
                        f.itinerary.total_time_minutes,
                        co2_str,
                        route,
                    );
                } else {
                    println!(
                        "{:<8}  {:>6}  {:>5}  {:>5}  {}",
                        f.itinerary.flight_by,
                        price,
                        f.itinerary.stop_count(),
                        f.itinerary.total_time_minutes,
                        route,
                    );
                }

                // Detail row: layover airports for multi-stop itineraries.
                if args.detail {
                    if let Some(conns) = &f.itinerary.connection_info {
                        if !conns.is_empty() {
                            let via_parts: Vec<String> = conns
                                .iter()
                                .map(|c| {
                                    format!(
                                        "{} ({} min)",
                                        c.arrival_airport, c.connection_time_minutes
                                    )
                                })
                                .collect();
                            println!("             via {}", via_parts.join(" → "));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// Extension trait used only by search to apply sort order after build.
trait WithSortOrder {
    fn with_sort_order(self, sort: SortOrder) -> Self;
}

impl WithSortOrder for Config {
    fn with_sort_order(mut self, sort: SortOrder) -> Self {
        self.sort_order = sort;
        self
    }
}

/// Keep only itineraries that carry a bookable price.
///
/// Unpriced itineraries (`trip_cost == None`) still carry schedule data, so the
/// `--priced-only` flag is opt-in — the default keeps them, rendering the price
/// as "—" and sorting them last under `--sort price`.
fn retain_priced_only(flights: &mut Vec<ItineraryContainer>) {
    flights.retain(|f| f.itinerary_cost.trip_cost.is_some());
}

#[cfg(test)]
mod tests {
    use super::*;
    use gflights::parsers::flight_response::{Itinerary, ItineraryCost, TripCost};

    /// Minimal itinerary container — priced when `priced` is true.
    fn container(priced: bool) -> ItineraryContainer {
        ItineraryContainer {
            itinerary: Itinerary {
                flight_by: "MH".into(),
                flight_details: vec![],
                total_time_minutes: 130,
                connection_info: None,
                emissions: None,
                self_transfer: None,
            },
            itinerary_cost: ItineraryCost {
                trip_cost: priced.then_some(TripCost {
                    unknown: None,
                    price: 137,
                }),
                departure_token: "tok".into(),
            },
            departure_protobuf: String::new(),
            mixed_cabin: None,
        }
    }

    #[test]
    fn priced_only_defaults_off() {
        let args = SearchArgs::try_parse_from([
            "search",
            "--from",
            "BKK",
            "--to",
            "KUL",
            "--date",
            "2026-08-11",
        ])
        .unwrap();
        assert!(!args.priced_only);
    }

    #[test]
    fn priced_only_parses() {
        let args = SearchArgs::try_parse_from([
            "search",
            "--from",
            "BKK",
            "--to",
            "KUL",
            "--date",
            "2026-08-11",
            "--priced-only",
        ])
        .unwrap();
        assert!(args.priced_only);
    }

    #[test]
    fn retain_priced_only_drops_unpriced_rows() {
        let mut flights = vec![container(true), container(false), container(true)];
        retain_priced_only(&mut flights);
        assert_eq!(flights.len(), 2);
        assert!(flights.iter().all(|f| f.itinerary_cost.trip_cost.is_some()));
    }

    #[test]
    fn retain_priced_only_keeps_all_when_everything_priced() {
        let mut flights = vec![container(true), container(true)];
        retain_priced_only(&mut flights);
        assert_eq!(flights.len(), 2);
    }

    #[test]
    fn web_url_defaults_off_and_parses() {
        let a = SearchArgs::try_parse_from([
            "search",
            "--from",
            "BKK",
            "--to",
            "KUL",
            "--date",
            "2026-08-11",
        ])
        .unwrap();
        assert!(!a.web_url);
        let b = SearchArgs::try_parse_from([
            "search",
            "--from",
            "BKK",
            "--to",
            "KUL",
            "--date",
            "2026-08-11",
            "--web-url",
        ])
        .unwrap();
        assert!(b.web_url);
    }
}
