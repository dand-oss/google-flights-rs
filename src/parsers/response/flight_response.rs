use core::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use crate::parsers::common::get_idx;
use crate::parsers::common::GetOuterErrorMessages;
use crate::parsers::common::SerializeToWeb;

use crate::parsers::common::{decode_inner_object, decode_outer_object, object_empty_as_none};
use anyhow::anyhow;
use anyhow::Result;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Stable leaf types — left unchanged, no unknown fields
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct AirplaneInfo {
    pub code: String,
    pub flight_number: String,
    #[serde(default)]
    pub plane_crew_by: Option<String>,
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct Date {
    pub year: i32,
    pub month: i32,
    pub day: i32,
}

impl Date {
    /// Convert to [`chrono::NaiveDate`], returning `None` for invalid dates
    /// (e.g. the zero-value `Date { year: 0, month: 0, day: 0 }`).
    pub fn to_naive(&self) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(self.year, self.month as u32, self.day as u32)
    }
}

impl SerializeToWeb for Date {
    fn serialize_to_web(&self) -> Result<String> {
        let date = self.to_naive().ok_or_else(|| anyhow!("Invalid date!"))?;
        Ok(date.format("%Y-%m-%d").to_string())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(default)]
pub struct Hour {
    #[serde(default)]
    pub hour: Option<i32>,
    #[serde(default)]
    pub minute: i32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ItineraryCost {
    #[serde(deserialize_with = "object_empty_as_none")]
    pub trip_cost: Option<TripCost>,
    pub departure_token: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TripCost {
    pub unknown: Option<String>,
    pub price: i32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TripCostContainer {
    pub trip_cost: TripCost,
    cost_protobuf: String,
}

// PriceGraph and its helpers — unchanged (currently working, no refactor needed)
#[derive(Debug, Deserialize, Serialize)]
pub struct PriceGraph {
    unknown0: i32,
    current_lowest_price: TripCost,
    lowest_hist_price: TripCost,
    lowest_price_days_ago: Vec<Option<i32>>,
    pub usual_price_low_bound: TripCost,
    usual_price_high_bound: TripCost,
    unknown6: i32,
    unknown7: Option<Value>,
    unknown8: Option<String>,
    unknown9: Option<String>,
    price_graph: Option<Vec<Vec<PricePoint>>>,
    unknown11: Value,
    destination_city_name: String,
    #[serde(default)]
    cheapest_to_book: Option<CheapestBook>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CheapestBook {
    unknown0: Option<SimilarDate>,
    unknown1: SimilarDate,
    unknown2: SimilarDate,
    in_average_cheaper: TripCost,
}

#[derive(Debug, Deserialize, Serialize)]
struct SimilarDate {
    unknown0: Vec<i32>,
    unknown1: i32,
    #[serde(default)]
    unknown2: Option<Vec<i32>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PricePoint {
    price_epoch: i64,
    price_point: i32,
}

// ---------------------------------------------------------------------------
// ConnectionInfo — one layover hop (index 13 of raw Itinerary array)
// ---------------------------------------------------------------------------

/// Details about a single layover / connection within an itinerary.
#[derive(Debug, Serialize, Clone)]
pub struct ConnectionInfo {
    /// Minutes spent at the connecting airport.
    pub connection_time_minutes: i32,
    /// IATA code of the airport the inbound leg lands at.
    pub arrival_airport: String,
    /// IATA code of the airport the outbound leg departs from (same building, usually).
    pub departure_airport: String,
    /// Warning codes, e.g. `1` = overnight layover.
    pub connection_warnings: Option<Vec<i32>>,
    pub arriving_airport_name: Option<String>,
    pub arriving_city: Option<String>,
    pub departure_airport_name: Option<String>,
    pub departure_city: Option<String>,
}

impl<'de> Deserialize<'de> for ConnectionInfo {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(ConnectionInfo {
            connection_time_minutes: get_idx(&arr, 0).unwrap_or(0),
            arrival_airport: get_idx(&arr, 1).unwrap_or_default(),
            departure_airport: get_idx(&arr, 2).unwrap_or_default(),
            connection_warnings: get_idx(&arr, 3),
            arriving_airport_name: get_idx(&arr, 4),
            arriving_city: get_idx(&arr, 5),
            departure_airport_name: get_idx(&arr, 6),
            departure_city: get_idx(&arr, 7),
        })
    }
}

// ---------------------------------------------------------------------------
// Emissions — CO2 data (index 22 of raw Itinerary array)
// ---------------------------------------------------------------------------

/// CO2 / emissions data for an itinerary.
/// All values are in grams.
#[derive(Debug, Serialize, Clone)]
pub struct Emissions {
    /// How much more or less CO2 this itinerary emits vs. the typical route,
    /// expressed as a percentage (negative = greener than average).
    pub emission_vs_average_percent: Option<i64>,
    /// Estimated CO2 for this specific flight, in grams.
    pub co2_this_flight_g: Option<i64>,
    /// Typical CO2 for this route, in grams.
    pub co2_typical_route_g: Option<i64>,
    /// Lowest CO2 found for this route, in grams.
    pub co2_lowest_route_g: Option<i64>,
}

impl<'de> Deserialize<'de> for Emissions {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(Emissions {
            emission_vs_average_percent: get_idx(&arr, 3),
            co2_this_flight_g: get_idx(&arr, 7),
            co2_typical_route_g: get_idx(&arr, 8),
            co2_lowest_route_g: get_idx(&arr, 10),
        })
    }
}

// ---------------------------------------------------------------------------
// Amenities — decoded from the 12-slot array at leg position 12
// ---------------------------------------------------------------------------

/// Per-leg amenities reported by Google Flights.
///
/// All fields are tri-state: `Some(true)` / `Some(false)` when Google
/// published the signal for this leg, `None` when it did not.
///
/// Confirmed slot mapping from live captures in May 2026, cross-checked
/// against the independently reverse-engineered decoder in
/// <https://github.com/punitarani/fli>:
/// slot 1 → wifi, slot 5 → power outlet, slot 9 → on-demand video,
/// slot 11 → integer legroom rating with values 2 or 3 observed.
#[derive(Debug, Serialize, Clone, Default, PartialEq)]
pub struct Amenities {
    pub wifi: Option<bool>,
    pub power: Option<bool>,
    pub on_demand_video: Option<bool>,
    pub legroom_rating: Option<i64>,
}

impl Amenities {
    /// `true` when none of the known slots carried a usable value.
    pub fn is_empty(&self) -> bool {
        self.wifi.is_none()
            && self.power.is_none()
            && self.on_demand_video.is_none()
            && self.legroom_rating.is_none()
    }
}

impl<'de> Deserialize<'de> for Amenities {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(Amenities {
            wifi: get_idx(&arr, 1),
            power: get_idx(&arr, 5),
            on_demand_video: get_idx(&arr, 9),
            legroom_rating: get_idx(&arr, 11),
        })
    }
}

// ---------------------------------------------------------------------------
// FlightInfo — Vec<Value> based, extract only fields used by SerializeToWeb
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone, Default)]
pub struct FlightInfo {
    pub departure_airport_code: String,
    pub destination_airport_code: String,
    pub departure_time: Hour,
    pub arrival_time: Hour,
    /// Duration of this individual leg in minutes.
    pub leg_duration_minutes: Option<i32>,
    pub departure_date: Date,
    pub arrival_date: Date,
    pub airplane_info: AirplaneInfo,
    /// Aircraft model at leg position 17, for example "Airbus A321neo".
    pub aircraft: Option<String>,
    /// Legroom description, for example "76 cm" — the long form at
    /// position 30, falling back to the short form at position 14.
    pub legroom: Option<String>,
    /// Cabin amenity flags at position 12. `None` when Google published
    /// no usable amenity signal for this leg.
    pub amenities: Option<Amenities>,
    /// `true` when the leg arrives on a later calendar day — position 19.
    pub overnight: Option<bool>,
    /// CO₂ emissions for this individual leg, in grams — position 31.
    pub co2_emissions_g: Option<i64>,
}

impl<'de> Deserialize<'de> for FlightInfo {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        let legroom_short: Option<String> = get_idx(&arr, 14);
        let legroom_long: Option<String> = get_idx(&arr, 30);
        let amenities: Option<Amenities> = get_idx(&arr, 12);
        Ok(FlightInfo {
            departure_airport_code: get_idx(&arr, 3).unwrap_or_default(),
            destination_airport_code: get_idx(&arr, 6).unwrap_or_default(),
            departure_time: get_idx(&arr, 8).unwrap_or_default(),
            arrival_time: get_idx(&arr, 10).unwrap_or_default(),
            leg_duration_minutes: get_idx(&arr, 11),
            departure_date: get_idx(&arr, 20).unwrap_or_default(),
            arrival_date: get_idx(&arr, 21).unwrap_or_default(),
            airplane_info: get_idx(&arr, 22).unwrap_or_default(),
            aircraft: get_idx(&arr, 17),
            legroom: legroom_long.or(legroom_short),
            amenities: amenities.filter(|a| !a.is_empty()),
            overnight: get_idx(&arr, 19),
            co2_emissions_g: get_idx(&arr, 31),
        })
    }
}

impl SerializeToWeb for FlightInfo {
    fn serialize_to_web(&self) -> Result<String> {
        Ok(format!(
            r#"[\"{}\",\"{}\",\"{}\",null,\"{}\",\"{}\"]"#,
            self.departure_airport_code,
            self.departure_date.serialize_to_web()?,
            self.destination_airport_code,
            self.airplane_info.code,
            self.airplane_info.flight_number
        ))
    }
}

// ---------------------------------------------------------------------------
// Itinerary — Vec<Value> based
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct Itinerary {
    /// Primary operating carrier code (e.g. `"LX"` or `"multi"`).
    pub flight_by: String,
    /// One entry per leg.
    pub flight_details: Vec<FlightInfo>,
    /// Total door-to-door duration including layovers, in minutes.
    pub total_time_minutes: i64,
    /// One entry per layover; empty / None for non-stop flights.
    pub connection_info: Option<Vec<ConnectionInfo>>,
    /// CO2 emissions data for this itinerary.
    pub emissions: Option<Emissions>,
    /// `true` when the itinerary requires a self-transfer — separate tickets
    /// or an unprotected connection. Position 12 of the raw itinerary array.
    pub self_transfer: Option<bool>,
}

impl<'de> Deserialize<'de> for Itinerary {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(Itinerary {
            flight_by: get_idx(&arr, 0).unwrap_or_default(),
            flight_details: get_idx(&arr, 2).unwrap_or_default(),
            total_time_minutes: get_idx(&arr, 9).unwrap_or(0),
            connection_info: get_idx(&arr, 13),
            emissions: get_idx(&arr, 22),
            self_transfer: get_idx(&arr, 12),
        })
    }
}

impl Itinerary {
    /// Number of layover stops (0 = non-stop, 1 = one stop, etc.).
    pub fn stop_count(&self) -> usize {
        self.connection_info.as_ref().map_or(0, |v| v.len())
    }

    /// Returns `true` if the flight lands on a later calendar date than it
    /// departs (i.e. a "next-day arrival" or later).
    ///
    /// Compares the departure date of the first leg with the arrival date of
    /// the last leg.  Returns `false` if dates are missing or invalid.
    pub fn arrives_next_day(&self) -> bool {
        let Some(first) = self.flight_details.first() else {
            return false;
        };
        let Some(last) = self.flight_details.last() else {
            return false;
        };
        match (
            first.departure_date.to_naive(),
            last.arrival_date.to_naive(),
        ) {
            (Some(dep), Some(arr)) => arr > dep,
            _ => false,
        }
    }

    /// The final arrival date of the itinerary, or `None` if unavailable.
    pub fn arrival_date(&self) -> Option<NaiveDate> {
        self.flight_details.last()?.arrival_date.to_naive()
    }

    /// The outbound departure date, or `None` if unavailable.
    pub fn departure_date(&self) -> Option<NaiveDate> {
        self.flight_details.first()?.departure_date.to_naive()
    }

    /// IATA codes of every connecting (layover) airport, in order.
    ///
    /// Derived from the inter-leg arrival airports — i.e. the airport each
    /// layover lands at. Empty for a non-stop itinerary.
    pub fn layover_airports(&self) -> Vec<&str> {
        self.connection_info.as_ref().map_or_else(Vec::new, |v| {
            v.iter().map(|c| c.arrival_airport.as_str()).collect()
        })
    }

    /// Returns `true` if this itinerary connects through **any** of the given
    /// IATA airport codes (case-insensitive).
    ///
    /// Google's connecting-airport ("via") filter is a *soft* server-side hint:
    /// the response still includes non-stop itineraries that don't route through
    /// the requested airports (they appear under "other flights" in the web UI).
    /// Use this to apply a strict client-side filter when you want results
    /// restricted to itineraries that actually stop at a via airport.
    ///
    /// An empty `airports` slice always returns `true` (no restriction).
    pub fn connects_via(&self, airports: &[String]) -> bool {
        if airports.is_empty() {
            return true;
        }
        let layovers = self.layover_airports();
        airports
            .iter()
            .any(|want| layovers.iter().any(|got| got.eq_ignore_ascii_case(want)))
    }
}

// ---------------------------------------------------------------------------
// ItineraryContainer — Vec<Value> based
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct ItineraryContainer {
    pub itinerary: Itinerary,
    pub itinerary_cost: ItineraryCost,
    /// Raw protobuf-encoded journey string — used for offer requests.
    pub departure_protobuf: String,
    /// `true` when the itinerary mixes cabin classes across legs —
    /// position 10 of the raw container array.
    pub mixed_cabin: Option<bool>,
}

impl<'de> Deserialize<'de> for ItineraryContainer {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(ItineraryContainer {
            itinerary: get_idx(&arr, 0)
                .ok_or_else(|| serde::de::Error::custom("missing itinerary at index 0"))?,
            itinerary_cost: get_idx(&arr, 1)
                .ok_or_else(|| serde::de::Error::custom("missing itinerary_cost at index 1"))?,
            departure_protobuf: get_idx(&arr, 8).unwrap_or_default(),
            mixed_cabin: get_idx(&arr, 10),
        })
    }
}

impl ItineraryContainer {
    pub fn get_departure_token(&self) -> String {
        self.itinerary_cost.departure_token.clone()
    }
}

// ---------------------------------------------------------------------------
// ItineraryContainerList — Vec<Value> based
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct ItineraryContainerList {
    pub itinerary_list: Vec<ItineraryContainer>,
}

impl<'de> Deserialize<'de> for ItineraryContainerList {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(ItineraryContainerList {
            itinerary_list: get_idx(&arr, 0).unwrap_or_default(),
        })
    }
}

// ---------------------------------------------------------------------------
// CheaperTravelDifferentDates and helpers — Vec<Value> based
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Clone)]
pub struct CheaperTravelDifferentDates {
    pub proposed_departure_date: NaiveDate,
    pub proposed_return_date: Option<NaiveDate>,
    pub proposed_trip_cost: Option<TripCostContainer>,
}

impl<'de> Deserialize<'de> for CheaperTravelDifferentDates {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(CheaperTravelDifferentDates {
            proposed_departure_date: get_idx(&arr, 0)
                .ok_or_else(|| serde::de::Error::custom("missing departure date at index 0"))?,
            proposed_return_date: get_idx(&arr, 1),
            proposed_trip_cost: get_idx(&arr, 2),
        })
    }
}

impl CheaperTravelDifferentDates {
    pub fn maybe_get_date_price(&self) -> Option<(NaiveDate, i32)> {
        self.proposed_trip_cost
            .as_ref()
            .map(|f| (self.proposed_departure_date, f.trip_cost.price))
    }
}

impl Display for CheaperTravelDifferentDates {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(return_date) = self.proposed_return_date {
            write!(
                f,
                "Proposed departure date: {}, proposed return date: {}, proposed trip cost: {}",
                self.proposed_departure_date,
                return_date,
                self.proposed_trip_cost
                    .as_ref()
                    .map(|f| f.trip_cost.price)
                    .unwrap_or_default()
            )
        } else {
            write!(
                f,
                "One way, proposed departure date: {}, proposed trip cost: {}",
                self.proposed_departure_date,
                self.proposed_trip_cost
                    .as_ref()
                    .map(|f| f.trip_cost.price)
                    .unwrap_or_default()
            )
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct CheaperTravelDifferentPlaces {
    #[serde(default)]
    pub dates: Option<Vec<CheaperTravelDifferentDates>>,
}

impl<'de> Deserialize<'de> for CheaperTravelDifferentPlaces {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(CheaperTravelDifferentPlaces {
            dates: get_idx(&arr, 0),
        })
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct CheaperTravelDifferentDatesContainer {
    pub different_dates: Option<CheaperTravelDifferentDates>,
    pub different_airport_or_dates: Option<CheaperTravelDifferentPlaces>,
}

impl<'de> Deserialize<'de> for CheaperTravelDifferentDatesContainer {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(CheaperTravelDifferentDatesContainer {
            different_dates: get_idx(&arr, 0),
            different_airport_or_dates: get_idx(&arr, 4),
        })
    }
}

// ---------------------------------------------------------------------------
// RawResponse — Vec<Value> based, only fields we actually use
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RawResponse {
    pub best_flights: Option<ItineraryContainerList>,
    pub other_flights: Option<ItineraryContainerList>,
    pub price_graph: Option<PriceGraph>,
    pub travel_cheaper_different_date: Option<Vec<CheaperTravelDifferentDatesContainer>>,
}

impl<'de> Deserialize<'de> for RawResponse {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let arr = Vec::<Value>::deserialize(d)?;
        Ok(RawResponse {
            best_flights: get_idx(&arr, 2),
            other_flights: get_idx(&arr, 3),
            price_graph: get_idx(&arr, 5),
            travel_cheaper_different_date: get_idx(&arr, 6),
        })
    }
}

impl RawResponse {
    pub fn maybe_get_all_flights(&self) -> Option<Vec<ItineraryContainer>> {
        let capacity = self
            .best_flights
            .as_ref()
            .map_or(0, |f| f.itinerary_list.len())
            + self
                .other_flights
                .as_ref()
                .map_or(0, |f| f.itinerary_list.len());
        if capacity == 0 {
            return None;
        }
        let mut all_itineraries: Vec<ItineraryContainer> = Vec::with_capacity(capacity);
        if let Some(f) = &self.best_flights {
            all_itineraries.extend(f.itinerary_list.iter().cloned());
        }
        if let Some(f) = &self.other_flights {
            all_itineraries.extend(f.itinerary_list.iter().cloned());
        }
        Some(all_itineraries)
    }

    fn get_usual_price_bound(&self) -> Option<i32> {
        self.price_graph
            .as_ref()
            .map(|f| f.usual_price_low_bound.price)
    }
}

// ---------------------------------------------------------------------------
// FlightResponseContainer
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct FlightResponseContainer {
    pub responses: Vec<RawResponse>,
}

impl FlightResponseContainer {
    pub fn get_usual_price_bound(&self) -> Option<i32> {
        let mut res: Vec<i32> = self
            .responses
            .iter()
            .flat_map(|f| f.get_usual_price_bound())
            .collect();
        res.sort();
        res.into_iter().next()
    }

    /// Return all itineraries across every response chunk, deduplicated by
    /// `departure_token`.  Google's streaming API often sends the same flight
    /// in multiple `wrb.fr` chunks; this method keeps only the first occurrence.
    pub fn get_all_flights(&self) -> Vec<ItineraryContainer> {
        self.get_all_flights_via(&[])
    }

    /// Like [`Self::get_all_flights`] but strictly restricts results to
    /// itineraries connecting through one of `connecting_airports` (the "via"
    /// filter). An empty slice imposes no restriction.
    ///
    /// Google's server-side via filter is soft: the `best_flights` container
    /// honours it, but the `other_flights` container still returns non-stop
    /// itineraries that skip the via airport. Because both containers are
    /// merged here, this client-side pass is what makes "via" strict — see
    /// [`Itinerary::connects_via`].
    pub fn get_all_flights_via(&self, connecting_airports: &[String]) -> Vec<ItineraryContainer> {
        let mut seen = std::collections::HashSet::new();
        self.responses
            .iter()
            .filter_map(|r| r.maybe_get_all_flights())
            .flatten()
            .filter(|f| f.itinerary.connects_via(connecting_airports))
            .filter(|f| seen.insert(f.itinerary_cost.departure_token.clone()))
            .collect()
    }
}

pub fn create_raw_response_vec(raw_inputs: String) -> Result<FlightResponseContainer> {
    let outer: Vec<RawResponseContainerVec> = decode_outer_object(raw_inputs.as_ref())?;
    let inner_objects: Vec<String> = outer
        .into_iter()
        .flat_map(|f| f.resp)
        .filter_map(|f| f.payload)
        .collect();
    let inner: Vec<RawResponse> = inner_objects
        .into_iter()
        .map(|f| decode_inner_object(&f))
        .filter_map(|f| f.ok())
        .collect();
    let response = FlightResponseContainer { responses: inner };
    Ok(response)
}

impl TryFrom<&str> for RawResponse {
    type Error = anyhow::Error;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let outer: Vec<RawResponseContainerVec> = decode_outer_object(value)?;
        let inner_object = outer
            .first()
            .ok_or_else(|| anyhow!("Malformed data!"))?
            .resp
            .first()
            .ok_or_else(|| anyhow!("Malformed data!"))?
            .payload
            .as_ref()
            .ok_or_else(|| anyhow!("Malformed data!"))?;
        decode_inner_object(inner_object)
    }
}

// ---------------------------------------------------------------------------
// Outer envelope types — unchanged
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct RawResponseContainer {
    unknown0: String,
    unknown1: Option<i32>,
    pub payload: Option<String>,
    #[serde(default)]
    unknown3: Option<String>,
    #[serde(default)]
    unknown4: Option<String>,
    #[serde(default)]
    maybe_error: Option<ErrorContainer>,
}

impl GetOuterErrorMessages for RawResponseContainer {
    fn get_error_messages(&self) -> Option<Vec<String>> {
        match &self.maybe_error {
            Some(ErrorContainer::Error(e)) => e.get_error_messages(),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(untagged)]
enum ErrorContainer {
    Success(Vec<Option<i32>>),
    Error(ErrorFromBackend),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ErrorFromBackend {
    unknown0: Value,
    unknown1: Value,
    maybe_error_container: Option<Vec<ErrorSpecific>>,
}

impl GetOuterErrorMessages for ErrorFromBackend {
    fn get_error_messages(&self) -> Option<Vec<String>> {
        let error_specific_vec: Vec<ErrorSpecific> = self.maybe_error_container.as_ref()?.to_vec();
        let messages: Vec<String> = error_specific_vec
            .iter()
            .filter_map(|f| f.error_message.as_ref())
            .map(|f| f.to_string())
            .collect();

        match messages.len() {
            0 => None,
            _ => Some(messages),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ErrorSpecific {
    error_message: Option<String>,
    garbage_data: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(transparent)]
pub struct RawResponseContainerVec {
    pub resp: Vec<RawResponseContainer>,
}

impl GetOuterErrorMessages for RawResponseContainerVec {
    fn get_error_messages(&self) -> Option<Vec<String>> {
        let messages: Vec<String> = self
            .resp
            .iter()
            .filter_map(|f| f.get_error_messages())
            .flatten()
            .collect();
        match messages.len() {
            0 => None,
            _ => Some(messages),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_parse_airline_json() {
        let json_str = r#"["LX","1628",null,"SWISS"]"#;

        let result: Result<AirplaneInfo, serde_json::Error> = serde_json::from_str(json_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_flight_info_json() {
        let json_str = r#"[null,null,"Helvetic","ZRH","Zurich Airport","Milan Malpensa Airport","MXP",null,[13,10],null,[14,5],55,[null,null,null,null,null,true],2,"74 cm",null,1,"Embraer 195 E2",[null,true],false,[2024,1,27],[2024,1,27],["LX","1628",null,"SWISS"],null,null,1,null,null,null,null,"74 centimetres",37467]"#;
        let result: Result<FlightInfo, serde_json::Error> = serde_json::from_str(json_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_itinerary_one_stop_lux_mxp_via_zrh() {
        // LUX→ZRH→MXP, 2 legs, 1 layover at ZRH (75 min), total 195 min, CO2 data present
        let mystr = r#"["LX", ["SWISS"], [[null, null, null, "LUX", "Luxembourg Airport", "Zurich Airport", "ZRH", null, [10, 50], null, [11, 55], 65, [], 1, "76 cm", null, 1, "Airbus A220-100 Passenger", null, false, [2024, 1, 27], [2024, 1, 27], ["LX", "751", null, "SWISS"], null, null, 1, null, null, null, null, "76 centimetres", 40497], [null, null, "Helvetic", "ZRH", "Zurich Airport", "Milan Malpensa Airport", "MXP", null, [13, 10], null, [14, 5], 55, [null, null, null, null, null, true], 2, "74 cm", null, 1, "Embraer 195 E2", [null, true], false, [2024, 1, 27], [2024, 1, 27], ["LX", "1628", null, "SWISS"], null, null, 1, null, null, null, null, "74 centimetres", 37467]], "LUX", [2024, 1, 27], [10, 50], "MXP", [2024, 1, 27], [14, 5], 195, null, null, false, [[75, "ZRH", "ZRH", null, "Zurich Airport", "ZÃ¼rich", "Zurich Airport", "ZÃ¼rich"]], null, null, null, "G3nUPe", [[1705070296848121, 139803069, 858572], null, null, null, null, [[2]]], 1, null, null, [null, null, 1, -9, null, true, true, 78000, 86000, null, 119000, 1, false], [1], [["LX", "SWISS", "https://www.swiss.com/gb/en/prepare/special-care"]]]"#;

        let result: Result<Itinerary, serde_json::Error> = serde_json::from_str(mystr);
        assert!(result.is_ok());
        let it = result.unwrap();
        assert_eq!(it.flight_by, "LX");
        assert_eq!(it.total_time_minutes, 195);
        assert_eq!(it.stop_count(), 1);
        let conn = it.connection_info.as_ref().unwrap();
        assert_eq!(conn[0].arrival_airport, "ZRH");
        assert_eq!(conn[0].connection_time_minutes, 75);
        let em = it.emissions.as_ref().unwrap();
        assert_eq!(em.co2_this_flight_g, Some(78000));
        assert_eq!(em.co2_typical_route_g, Some(86000));
        // individual leg durations
        assert_eq!(it.flight_details[0].leg_duration_minutes, Some(65));
        assert_eq!(it.flight_details[1].leg_duration_minutes, Some(55));
    }

    #[test]
    fn test_connects_via_strict_filter() {
        // LX→ZRH→MXP, one layover at ZRH.
        let mystr = r#"["LX", ["SWISS"], [[null, null, null, "LUX", "Luxembourg Airport", "Zurich Airport", "ZRH", null, [10, 50], null, [11, 55], 65, [], 1, "76 cm", null, 1, "Airbus A220-100 Passenger", null, false, [2024, 1, 27], [2024, 1, 27], ["LX", "751", null, "SWISS"], null, null, 1, null, null, null, null, "76 centimetres", 40497], [null, null, "Helvetic", "ZRH", "Zurich Airport", "Milan Malpensa Airport", "MXP", null, [13, 10], null, [14, 5], 55, [null, null, null, null, null, true], 2, "74 cm", null, 1, "Embraer 195 E2", [null, true], false, [2024, 1, 27], [2024, 1, 27], ["LX", "1628", null, "SWISS"], null, null, 1, null, null, null, null, "74 centimetres", 37467]], "LUX", [2024, 1, 27], [10, 50], "MXP", [2024, 1, 27], [14, 5], 195, null, null, false, [[75, "ZRH", "ZRH", null, "Zurich Airport", "ZÃ¼rich", "Zurich Airport", "ZÃ¼rich"]], null, null, null, "G3nUPe", [[1705070296848121, 139803069, 858572], null, null, null, null, [[2]]], 1, null, null, [null, null, 1, -9, null, true, true, 78000, 86000, null, 119000, 1, false], [1], [["LX", "SWISS", "https://www.swiss.com/gb/en/prepare/special-care"]]]"#;
        let it: Itinerary = serde_json::from_str(mystr).unwrap();

        assert_eq!(it.layover_airports(), vec!["ZRH"]);
        // Empty filter = no restriction.
        assert!(it.connects_via(&[]));
        // Matches the actual layover (case-insensitive).
        assert!(it.connects_via(&["ZRH".to_string()]));
        assert!(it.connects_via(&["zrh".to_string()]));
        // Any-of semantics: one of the requested airports matches.
        assert!(it.connects_via(&["DXB".to_string(), "ZRH".to_string()]));
        // No requested airport on the route → excluded.
        assert!(!it.connects_via(&["DXB".to_string()]));
    }

    #[test]
    fn test_connects_via_nonstop_excluded() {
        // LG direct LUX→MXP, no connection_info → never matches a via filter.
        let mystr = r#"["LG", ["Luxair"], [[null, null, null, "LUX", "Luxembourg Airport", "Milan Malpensa Airport", "MXP", null, [11, 10], null, [12, 25], 75, [], 1, "76 cm", null, 1, "Dash-8", null, false, [2024, 1, 27], [2024, 1, 27], ["LG", "6993", null, "Luxair"], null, null, 1, null, null, null, null, "76 centimetres", 35968]], "LUX", [2024, 1, 27], [11, 10], "MXP", [2024, 1, 27], [12, 25], 75, null, null, false, null, null, null, ["ITA"], "VDOwRb", [[1705070296848121, 139803069, 858572], null, null, null, null, [[6]]], 1, null, null, [null, null, 1, -58, null, true, true, 36000, 86000, [true], 119000, 1, false], [1], [["LG", "Luxair", "x"]]]"#;
        let it: Itinerary = serde_json::from_str(mystr).unwrap();
        assert!(it.layover_airports().is_empty());
        assert_eq!(it.stop_count(), 0);
        // Non-stop never connects via anything → strict via filter drops it.
        assert!(!it.connects_via(&["DXB".to_string()]));
        // But no filter still keeps it.
        assert!(it.connects_via(&[]));
    }

    #[test]
    fn test_parse_itinerary_container_with_booking_token() {
        let mystr = r#"[["LX", ["SWISS"], [[null, null, null, "LUX", "Luxembourg Airport", "Zurich Airport", "ZRH", null, [10, 50], null, [11, 55], 65, [], 1, "76 cm", null, 1, "Airbus A220-100 Passenger", null, false, [2024, 1, 27], [2024, 1, 27], ["LX", "751", null, "SWISS"], null, null, 1, null, null, null, null, "76 centimetres", 40497], [null, null, "Helvetic", "ZRH", "Zurich Airport", "Milan Malpensa Airport", "MXP", null, [13, 10], null, [14, 5], 55, [null, null, null, null, null, true], 2, "74 cm", null, 1, "Embraer 195 E2", [null, true], false, [2024, 1, 27], [2024, 1, 27], ["LX", "1628", null, "SWISS"], null, null, 1, null, null, null, null, "74 centimetres", 37467]], "LUX", [2024, 1, 27], [10, 50], "MXP", [2024, 1, 27], [14, 5], 195, null, null, false, [[75, "ZRH", "ZRH", null, "Zurich Airport", "ZÃ¼rich", "Zurich Airport", "ZÃ¼rich"]], null, null, null, "G3nUPe", [[1705070296848121, 139803069, 858572], null, null, null, null, [[2]]], 1, null, null, [null, null, 1, -9, null, true, true, 78000, 86000, null, 119000, 1, false], [1], [["LX", "SWISS", "https://www.swiss.com/gb/en/prepare/special-care"]]], [[null, 138], "CjRISnlhWXVsbHpfclVBSEhWcVFCRy0tLS0tLS0td2VicXIxMkFBQUFBR1doVHRnTTFpVHVBEgxMWDc1MXxMWDE2MjgaCgihaxACGgNFVVI4HHDDdQ=="], null, true, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCKFrIs4BCrgBClkKA0xVWBIZMjAyNC0wMS0yN1QxMDo1MDowMCswMTowMBoDWlJIIhkyMDI0LTAxLTI3VDExOjU1OjAwKzAxOjAwKgJMWDIDNzUxOgJMWEIDNzUxSAFSAzIyMQpbCgNaUkgSGTIwMjQtMDEtMjdUMTM6MTA6MDArMDE6MDAaA01YUCIZMjAyNC0wMS0yN1QxNDowNTowMCswMTowMCoCTFgyBDE2Mjg6AkxYQgQxNjI4SAFSAzI5NRIECAMQARgBKAAyBwoFU1dJU1M\\u003d\"]", [[1]], false]"#;

        let result: Result<ItineraryContainer, serde_json::Error> = serde_json::from_str(mystr);
        assert!(result.is_ok());
        let container = result.unwrap();
        assert!(!container.itinerary_cost.departure_token.is_empty());
        assert_eq!(container.itinerary.total_time_minutes, 195);
    }

    #[test]
    fn test_itinerary_list() {
        let mystr = r#"[[["LX", ["SWISS"], [[null, null, null, "LUX", "Luxembourg Airport", "Zurich Airport", "ZRH", null, [10, 50], null, [11, 55], 65, [], 1, "76 cm", null, 1, "Airbus A220-100 Passenger", null, false, [2024, 1, 27], [2024, 1, 27], ["LX", "751", null, "SWISS"], null, null, 1, null, null, null, null, "76 centimetres", 40497], [null, null, "Helvetic", "ZRH", "Zurich Airport", "Milan Malpensa Airport", "MXP", null, [13, 10], null, [14, 5], 55, [null, null, null, null, null, true], 2, "74 cm", null, 1, "Embraer 195 E2", [null, true], false, [2024, 1, 27], [2024, 1, 27], ["LX", "1628", null, "SWISS"], null, null, 1, null, null, null, null, "74 centimetres", 37467]], "LUX", [2024, 1, 27], [10, 50], "MXP", [2024, 1, 27], [14, 5], 195, null, null, false, [[75, "ZRH", "ZRH", null, "Zurich Airport", "ZÃ¼rich", "Zurich Airport", "ZÃ¼rich"]], null, null, null, "G3nUPe", [[1705070296848121, 139803069, 858572], null, null, null, null, [[2]]], 1, null, null, [null, null, 1, -9, null, true, true, 78000, 86000, null, 119000, 1, false], [1], [["LX", "SWISS", "https://www.swiss.com/gb/en/prepare/special-care"]]], [[null, 138], "CjRISnlhWXVsbHpfclVBSEhWcVFCRy0tLS0tLS0td2VicXIxMkFBQUFBR1doVHRnTTFpVHVBEgxMWDc1MXxMWDE2MjgaCgihaxACGgNFVVI4HHDDdQ=="], null, true, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCKFrIs4BCrgBClkKA0xVWBIZMjAyNC0wMS0yN1QxMDo1MDowMCswMTowMBoDWlJIIhkyMDI0LTAxLTI3VDExOjU1OjAwKzAxOjAwKgJMWDIDNzUxOgJMWEIDNzUxSAFSAzIyMQpbCgNaUkgSGTIwMjQtMDEtMjdUMTM6MTA6MDArMDE6MDAaA01YUCIZMjAyNC0wMS0yN1QxNDowNTowMCswMTowMCoCTFgyBDE2Mjg6AkxYQgQxNjI4SAFSAzI5NRIECAMQARgBKAAyBwoFU1dJU1M\\u003d\"]", [[1]], false], [["multi", ["Lufthansa", "Air Dolomiti"], [[null, null, "Lufthansa CityLine", "LUX", "Luxembourg Airport", "Munich International Airport", "MUC", null, [9, 40], null, [10, 45], 65, [], 1, "79 cm", null, 1, "Canadair RJ 900", null, false, [2024, 1, 27], [2024, 1, 27], ["LH", "2317", null, "Lufthansa"], null, null, 1, null, null, null, null, "79 centimetres", 63873], [null, null, null, "MUC", "Munich International Airport", "Milan Malpensa Airport", "MXP", null, [11, 30], null, [12, 35], 65, [], 1, "79 cm", [["LH", "9448", null, "Lufthansa"]], 1, "Embraer 195", [null, true], false, [2024, 1, 27], [2024, 1, 27], ["EN", "8274", null, "Air Dolomiti"], null, null, 1, null, null, null, null, "79 centimetres", 55785]], "LUX", [2024, 1, 27], [9, 40], "MXP", [2024, 1, 27], [12, 35], 175, null, null, false, [[45, "MUC", "MUC", null, "Munich International Airport", "Munich", "Munich International Airport", "Munich"]], null, null, null, "zd8P7d", [[1705070296848121, 139803069, 858572], null, null, null, null, [[3]]], 1, null, null, [null, null, 3, 40, null, true, true, 120000, 86000, null, 119000, 2, false], [1], [["LH", "Lufthansa", "https://www.lufthansa.com/gb/en/travelling-with-special-requirements"], ["EN", "Air Dolomiti", "https://www.airdolomiti.eu/assistance"]]], [[null, 145], "CjRISnlhWXVsbHpfclVBSEhWcVFCRy0tLS0tLS0td2VicXIxMkFBQUFBR1doVHRnTTFpVHVBEg1MSDIzMTd8RU44Mjc0GgoIoXEQAhoDRVVSOBxwjHw="], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCKFxIuIBCroBClsKA0xVWBIZMjAyNC0wMS0yN1QwOTo0MDowMCswMTowMBoDTVVDIhkyMDI0LTAxLTI3VDEwOjQ1OjAwKzAxOjAwKgJMSDIEMjMxNzoCTEhCBDIzMTdIAVIDQ1I5ClsKA01VQxIZMjAyNC0wMS0yN1QxMTozMDowMCswMTowMBoDTVhQIhkyMDI0LTAxLTI3VDEyOjM1OjAwKzAxOjAwKgJFTjIEODI3NDoCTEhCBDk0NDhIAVIDRTk1EgQIAxABGAEoADIZCglMdWZ0aGFuc2EKDEFpciBEb2xvbWl0aQ\\u003d\\u003d\"]", [[2]], false], [["KL", ["KLM"], [[null, null, "German Airways", "LUX", "Luxembourg Airport", "Amsterdam Airport Schiphol", "AMS", null, [14, 45], null, [16], 75, [null, null, null, null, null, true], 1, "79 cm", null, 1, "Embraer 190", null, false, [2024, 1, 27], [2024, 1, 27], ["KL", "1742", null, "KLM"], null, null, 1, null, null, null, null, "79 centimetres", 57027], [null, null, "KLM Cityhopper", "AMS", "Amsterdam Airport Schiphol", "Linate Airport", "LIN", null, [16, 55], null, [18, 35], 100, [], 2, "74 cm", null, 1, "Embraer 175", null, false, [2024, 1, 27], [2024, 1, 27], ["KL", "1621", null, "KLM"], null, null, 1, null, null, null, null, "74 centimetres", 91358]], "LUX", [2024, 1, 27], [14, 45], "LIN", [2024, 1, 27], [18, 35], 230, null, null, false, [[55, "AMS", "AMS", null, "Amsterdam Airport Schiphol", "Amsterdam", "Amsterdam Airport Schiphol", "Amsterdam"]], null, null, null, "goZ5db", [[1705070296848121, 139803069, 858572], null, null, null, null, [[4]]], 1, null, null, [null, null, 3, 72, null, true, true, 148000, 86000, null, 119000, 3, false], [1], [["KL", "KLM", "https://www.klm.co.uk/information/assistance-health"]]], [[null, 154], "CjRISnlhWXVsbHpfclVBSEhWcVFCRy0tLS0tLS0td2VicXIxMkFBQUFBR1doVHRnTTFpVHVBEg1LTDE3NDJ8S0wxNjIxGgoI4ncQAhoDRVVSOBxwnYMB"], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCOJ3Is4BCroBClsKA0xVWBIZMjAyNC0wMS0yN1QxNDo0NTowMCswMTowMBoDQU1TIhkyMDI0LTAxLTI3VDE2OjAwOjAwKzAxOjAwKgJLTDIEMTc0MjoCS0xCBDE3NDJIAVIDRTkwClsKA0FNUxIZMjAyNC0wMS0yN1QxNjo1NTowMCswMTowMBoDTElOIhkyMDI0LTAxLTI3VDE4OjM1OjAwKzAxOjAwKgJLTDIEMTYyMToCS0xCBDE2MjFIAVIDRTdXEgQIAxABGAEoADIFCgNLTE0\\u003d\"]", [[2]], false], [["LH", ["Lufthansa"], [[null, null, "Lufthansa CityLine", "LUX", "Luxembourg Airport", "Frankfurt Airport", "FRA", null, [6, 35], null, [7, 25], 50, [], 3, "81 cm", null, 1, "Embraer 190", null, false, [2024, 1, 27], [2024, 1, 27], ["LH", "399", null, "Lufthansa"], null, null, 1, null, null, null, null, "81 centimetres", 46528], [null, null, null, "FRA", "Frankfurt Airport", "Milan Malpensa Airport", "MXP", null, [9, 10], null, [10, 20], 70, [], 1, "76 cm", null, 1, "Airbus A320", null, false, [2024, 1, 27], [2024, 1, 27], ["LH", "246", null, "Lufthansa"], null, null, 1, null, null, null, null, "76 centimetres", 55017]], "LUX", [2024, 1, 27], [6, 35], "MXP", [2024, 1, 27], [10, 20], 225, null, null, false, [[105, "FRA", "FRA", null, "Frankfurt Airport", "Frankfurt", "Frankfurt Airport", "Frankfurt"]], null, null, null, "D2ou8e", [[1705070296848121, 139803069, 858572], null, null, null, null, [[5]]], 1, null, null, [null, null, 3, 19, null, true, true, 102000, 86000, null, 119000, 1, false], [1], [["LH", "Lufthansa", "https://www.lufthansa.com/gb/en/travelling-with-special-requirements"]]], [[null, 159], "CjRISnlhWXVsbHpfclVBSEhWcVFCRy0tLS0tLS0td2VicXIxMkFBQUFBR1doVHRnTTFpVHVBEgtMSDM5OXxMSDI0NhoKCIZ8EAIaA0VVUjgccPWHAQ=="], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCIZ8ItABCrYBClkKA0xVWBIZMjAyNC0wMS0yN1QwNjozNTowMCswMTowMBoDRlJBIhkyMDI0LTAxLTI3VDA3OjI1OjAwKzAxOjAwKgJMSDIDMzk5OgJMSEIDMzk5SAFSA0U5MApZCgNGUkESGTIwMjQtMDEtMjdUMDk6MTA6MDArMDE6MDAaA01YUCIZMjAyNC0wMS0yN1QxMDoyMDowMCswMTowMCoCTEgyAzI0NjoCTEhCAzI0NkgBUgMzMjASBAgDEAEYASgAMgsKCUx1ZnRoYW5zYQ\\u003d\\u003d\"]", [[2]], false], [["LG", ["Luxair"], [[null, null, null, "LUX", "Luxembourg Airport", "Milan Malpensa Airport", "MXP", null, [11, 10], null, [12, 25], 75, [], 1, "76 cm", [["AZ", "7879", null, "ITA"]], 1, "De Havilland-Bombardier Dash-8", null, false, [2024, 1, 27], [2024, 1, 27], ["LG", "6993", null, "Luxair"], null, null, 1, null, null, null, null, "76 centimetres", 35968]], "LUX", [2024, 1, 27], [11, 10], "MXP", [2024, 1, 27], [12, 25], 75, null, null, false, null, null, null, ["ITA"], "VDOwRb", [[1705070296848121, 139803069, 858572], null, null, null, null, [[6]]], 1, null, null, [null, null, 1, -58, null, true, true, 36000, 86000, [true], 119000, 1, false], [1], [["LG", "Luxair", "https://www.luxair.lu/en/information/passenger-assistance"]]], [[null, 230], "CjRISnlhWXVsbHpfclVBSEhWcVFCRy0tLS0tLS0td2VicXIxMkFBQUFBR1doVHRnTTFpVHVBEgZMRzY5OTMaCwjVswEQAhoDRVVSOBxw7sQB"], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoECNWzASJ4Cl0KWwoDTFVYEhkyMDI0LTAxLTI3VDExOjEwOjAwKzAxOjAwGgNNWFAiGTIwMjQtMDEtMjdUMTI6MjU6MDArMDE6MDAqAkxHMgQ2OTkzOgJMR0IENjk5M0gBUgNESDQSBAgDEAEYASgAMg0KBkx1eGFpcgoDSVRB\"]", [[1]], false]]"#;

        let jd: &mut serde_json::Deserializer<serde_json::de::StrRead<'_>> =
            &mut serde_json::Deserializer::from_str(mystr);
        let result: Result<Vec<ItineraryContainer>, _> = serde_path_to_error::deserialize(jd);
        match result {
            Ok(_) => assert!(result.is_ok()),
            Err(err) => {
                panic!("{}", err.path())
            }
        }
    }

    #[test]
    fn test_itinerary_list_container() {
        let mystr = r#"[[[["LX", ["SWISS"], [[null, null, null, "LUX", "Luxembourg Airport", "Zurich Airport", "ZRH", null, [10, 50], null, [11, 55], 65, [], 1, "76 cm", null, 1, "Airbus A220-100 Passenger", null, false, [2024, 1, 27], [2024, 1, 27], ["LX", "751", null, "SWISS"], null, null, 1, null, null, null, null, "76 centimetres", 40497], [null, null, "Helvetic", "ZRH", "Zurich Airport", "Milan Malpensa Airport", "MXP", null, [13, 10], null, [14, 5], 55, [null, null, null, null, null, true], 2, "74 cm", null, 1, "Embraer 195 E2", [null, true], false, [2024, 1, 27], [2024, 1, 27], ["LX", "1628", null, "SWISS"], null, null, 1, null, null, null, null, "74 centimetres", 37467]], "LUX", [2024, 1, 27], [10, 50], "MXP", [2024, 1, 27], [14, 5], 195, null, null, false, [[75, "ZRH", "ZRH", null, "Zurich Airport", "ZÃ¼rich", "Zurich Airport", "ZÃ¼rich"]], null, null, null, "G3nUPe", [[1705063796213762, 139803069, 858572], null, null, null, null, [[2]]], 1, null, null, [null, null, 1, -9, null, true, true, 78000, 86000, null, 119000, 1, false], [1], [["LX", "SWISS", "https://www.swiss.com/gb/en/prepare/special-care"]]], [[null, 138], "CjRIX0VzeG9hMURhNElBRGVpM2dCRy0tLS0tLS0tLXdmZG4yMEFBQUFBR1doTlhRRFc1SE9BEgxMWDc1MXxMWDE2MjgaCgihaxACGgNFVVI4HHCxdQ=="], null, true, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCKFrIs4BCrgBClkKA0xVWBIZMjAyNC0wMS0yN1QxMDo1MDowMCswMTowMBoDWlJIIhkyMDI0LTAxLTI3VDExOjU1OjAwKzAxOjAwKgJMWDIDNzUxOgJMWEIDNzUxSAFSAzIyMQpbCgNaUkgSGTIwMjQtMDEtMjdUMTM6MTA6MDArMDE6MDAaA01YUCIZMjAyNC0wMS0yN1QxNDowNTowMCswMTowMCoCTFgyBDE2Mjg6AkxYQgQxNjI4SAFSAzI5NRIECAMQARgBKAAyBwoFU1dJU1M\\u003d\"]", [[1]], false], [["multi", ["Lufthansa", "Air Dolomiti"], [[null, null, "Lufthansa CityLine", "LUX", "Luxembourg Airport", "Munich International Airport", "MUC", null, [9, 40], null, [10, 45], 65, [], 1, "79 cm", null, 1, "Canadair RJ 900", null, false, [2024, 1, 27], [2024, 1, 27], ["LH", "2317", null, "Lufthansa"], null, null, 1, null, null, null, null, "79 centimetres", 63873], [null, null, null, "MUC", "Munich International Airport", "Milan Malpensa Airport", "MXP", null, [11, 30], null, [12, 35], 65, [], 1, "79 cm", [["LH", "9448", null, "Lufthansa"]], 1, "Embraer 195", [null, true], false, [2024, 1, 27], [2024, 1, 27], ["EN", "8274", null, "Air Dolomiti"], null, null, 1, null, null, null, null, "79 centimetres", 55785]], "LUX", [2024, 1, 27], [9, 40], "MXP", [2024, 1, 27], [12, 35], 175, null, null, false, [[45, "MUC", "MUC", null, "Munich International Airport", "Munich", "Munich International Airport", "Munich"]], null, null, null, "zd8P7d", [[1705063796213762, 139803069, 858572], null, null, null, null, [[3]]], 1, null, null, [null, null, 3, 40, null, true, true, 120000, 86000, null, 119000, 2, false], [1], [["LH", "Lufthansa", "https://www.lufthansa.com/gb/en/travelling-with-special-requirements"], ["EN", "Air Dolomiti", "https://www.airdolomiti.eu/assistance"]]], [[null, 145], "CjRIX0VzeG9hMURhNElBRGVpM2dCRy0tLS0tLS0tLXdmZG4yMEFBQUFBR1doTlhRRFc1SE9BEg1MSDIzMTd8RU44Mjc0GgoIoXEQAhoDRVVSOBxw+ns="], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCKFxIuIBCroBClsKA0xVWBIZMjAyNC0wMS0yN1QwOTo0MDowMCswMTowMBoDTVVDIhkyMDI0LTAxLTI3VDEwOjQ1OjAwKzAxOjAwKgJMSDIEMjMxNzoCTEhCBDIzMTdIAVIDQ1I5ClsKA01VQxIZMjAyNC0wMS0yN1QxMTozMDowMCswMTowMBoDTVhQIhkyMDI0LTAxLTI3VDEyOjM1OjAwKzAxOjAwKgJFTjIEODI3NDoCTEhCBDk0NDhIAVIDRTk1EgQIAxABGAEoADIZCglMdWZ0aGFuc2EKDEFpciBEb2xvbWl0aQ\\u003d\\u003d\"]", [[2]], false], [["KL", ["KLM"], [[null, null, "German Airways", "LUX", "Luxembourg Airport", "Amsterdam Airport Schiphol", "AMS", null, [14, 45], null, [16], 75, [null, null, null, null, null, true], 1, "79 cm", null, 1, "Embraer 190", null, false, [2024, 1, 27], [2024, 1, 27], ["KL", "1742", null, "KLM"], null, null, 1, null, null, null, null, "79 centimetres", 57027], [null, null, "KLM Cityhopper", "AMS", "Amsterdam Airport Schiphol", "Linate Airport", "LIN", null, [16, 55], null, [18, 35], 100, [], 2, "74 cm", null, 1, "Embraer 175", null, false, [2024, 1, 27], [2024, 1, 27], ["KL", "1621", null, "KLM"], null, null, 1, null, null, null, null, "74 centimetres", 91358]], "LUX", [2024, 1, 27], [14, 45], "LIN", [2024, 1, 27], [18, 35], 230, null, null, false, [[55, "AMS", "AMS", null, "Amsterdam Airport Schiphol", "Amsterdam", "Amsterdam Airport Schiphol", "Amsterdam"]], null, null, null, "goZ5db", [[1705063796213762, 139803069, 858572], null, null, null, null, [[4]]], 1, null, null, [null, null, 3, 72, null, true, true, 148000, 86000, null, 119000, 3, false], [1], [["KL", "KLM", "https://www.klm.co.uk/information/assistance-health"]]], [[null, 154], "CjRIX0VzeG9hMURhNElBRGVpM2dCRy0tLS0tLS0tLXdmZG4yMEFBQUFBR1doTlhRRFc1SE9BEg1LTDE3NDJ8S0wxNjIxGgoI4ncQAhoDRVVSOBxwiYMB"], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCOJ3Is4BCroBClsKA0xVWBIZMjAyNC0wMS0yN1QxNDo0NTowMCswMTowMBoDQU1TIhkyMDI0LTAxLTI3VDE2OjAwOjAwKzAxOjAwKgJLTDIEMTc0MjoCS0xCBDE3NDJIAVIDRTkwClsKA0FNUxIZMjAyNC0wMS0yN1QxNjo1NTowMCswMTowMBoDTElOIhkyMDI0LTAxLTI3VDE4OjM1OjAwKzAxOjAwKgJLTDIEMTYyMToCS0xCBDE2MjFIAVIDRTdXEgQIAxABGAEoADIFCgNLTE0\\u003d\"]", [[2]], false], [["LH", ["Lufthansa"], [[null, null, "Lufthansa CityLine", "LUX", "Luxembourg Airport", "Frankfurt Airport", "FRA", null, [6, 35], null, [7, 25], 50, [], 3, "81 cm", null, 1, "Embraer 190", null, false, [2024, 1, 27], [2024, 1, 27], ["LH", "399", null, "Lufthansa"], null, null, 1, null, null, null, null, "81 centimetres", 46528], [null, null, null, "FRA", "Frankfurt Airport", "Milan Malpensa Airport", "MXP", null, [9, 10], null, [10, 20], 70, [], 1, "76 cm", null, 1, "Airbus A320", null, false, [2024, 1, 27], [2024, 1, 27], ["LH", "246", null, "Lufthansa"], null, null, 1, null, null, null, null, "76 centimetres", 55017]], "LUX", [2024, 1, 27], [6, 35], "MXP", [2024, 1, 27], [10, 20], 225, null, null, false, [[105, "FRA", "FRA", null, "Frankfurt Airport", "Frankfurt", "Frankfurt Airport", "Frankfurt"]], null, null, null, "D2ou8e", [[1705063796213762, 139803069, 858572], null, null, null, null, [[5]]], 1, null, null, [null, null, 3, 19, null, true, true, 102000, 86000, null, 119000, 1, false], [1], [["LH", "Lufthansa", "https://www.lufthansa.com/gb/en/travelling-with-special-requirements"]]], [[null, 159], "CjRIX0VzeG9hMURhNElBRGVpM2dCRy0tLS0tLS0tLXdmZG4yMEFBQUFBR1doTlhRRFc1SE9BEgtMSDM5OXxMSDI0NhoKCIZ8EAIaA0VVUjgccOGHAQ=="], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoDCIZ8ItABCrYBClkKA0xVWBIZMjAyNC0wMS0yN1QwNjozNTowMCswMTowMBoDRlJBIhkyMDI0LTAxLTI3VDA3OjI1OjAwKzAxOjAwKgJMSDIDMzk5OgJMSEIDMzk5SAFSA0U5MApZCgNGUkESGTIwMjQtMDEtMjdUMDk6MTA6MDArMDE6MDAaA01YUCIZMjAyNC0wMS0yN1QxMDoyMDowMCswMTowMCoCTEgyAzI0NjoCTEhCAzI0NkgBUgMzMjASBAgDEAEYASgAMgsKCUx1ZnRoYW5zYQ\\u003d\\u003d\"]", [[2]], false], [["LG", ["Luxair"], [[null, null, null, "LUX", "Luxembourg Airport", "Milan Malpensa Airport", "MXP", null, [11, 10], null, [12, 25], 75, [], 1, "76 cm", [["AZ", "7879", null, "ITA"]], 1, "De Havilland-Bombardier Dash-8", null, false, [2024, 1, 27], [2024, 1, 27], ["LG", "6993", null, "Luxair"], null, null, 1, null, null, null, null, "76 centimetres", 35968]], "LUX", [2024, 1, 27], [11, 10], "MXP", [2024, 1, 27], [12, 25], 75, null, null, false, null, null, null, ["ITA"], "VDOwRb", [[1705063796213762, 139803069, 858572], null, null, null, null, [[6]]], 1, null, null, [null, null, 1, -58, null, true, true, 36000, 86000, [true], 119000, 1, false], [1], [["LG", "Luxair", "https://www.luxair.lu/en/information/passenger-assistance"]]], [[null, 230], "CjRIX0VzeG9hMURhNElBRGVpM2dCRy0tLS0tLS0tLXdmZG4yMEFBQUFBR1doTlhRRFc1SE9BEgZMRzY5OTMaCwjVswEQAhoDRVVSOBxw0MQB"], null, false, [], [false, false, false], false, [], "[\"CAISA0VVUhoECNWzASJ4Cl0KWwoDTFVYEhkyMDI0LTAxLTI3VDExOjEwOjAwKzAxOjAwGgNNWFAiGTIwMjQtMDEtMjdUMTI6MjU6MDArMDE6MDAqAkxHMgQ2OTkzOgJMR0IENjk5M0gBUgNESDQSBAgDEAEYASgAMg0KBkx1eGFpcgoDSVRB\"]", [[1]], false]], null, false, false, [1]]"#;

        let jd: &mut serde_json::Deserializer<serde_json::de::StrRead<'_>> =
            &mut serde_json::Deserializer::from_str(mystr);
        let result: Result<ItineraryContainerList, _> = serde_path_to_error::deserialize(jd);
        match result {
            Ok(_) => assert!(result.is_ok()),
            Err(err) => {
                panic!("{}", err.path())
            }
        }
    }

    #[test]
    fn test_raw_response_all() {
        let mystr =
            fs::read_to_string("test_files/raw_gflights.response").expect("Cannot read from file");

        let raw_resp: RawResponseContainerVec =
            serde_json::from_str(&mystr).expect("Error in parsing");
        let inner_obj = &raw_resp.resp[0].payload.as_ref().unwrap();
        let jd: &mut serde_json::Deserializer<serde_json::de::StrRead<'_>> =
            &mut serde_json::Deserializer::from_str(inner_obj);
        let result: Result<RawResponse, _> = serde_path_to_error::deserialize(jd);
        match result {
            Ok(_) => assert!(result.is_ok()),
            Err(err) => {
                panic!("{} at {}", err, err.path())
            }
        }
    }

    #[test]
    fn test_tokyo_response() {
        let datafiles = [
            "test_files/lux_tokyo_oneway.txt",
            "test_files/lux_milan_oneway.txt",
            "test_files/lux_dubai_oneway.txt",
            "test_files/flights_new_test.txt",
            "test_files/response_non_uniform_city_images.txt",
            "test_files/raw.response",
        ];
        for itinerary in datafiles.iter() {
            let mystr = fs::read_to_string(itinerary).expect("Cannot read from file");

            let jd: &mut serde_json::Deserializer<serde_json::de::StrRead<'_>> =
                &mut serde_json::Deserializer::from_str(&mystr);
            let result: Result<RawResponse, _> = serde_path_to_error::deserialize(jd);
            match result {
                Ok(_) => assert!(result.is_ok()),
                Err(err) => {
                    panic!("{} at {} (file: {})", err, err.path(), itinerary)
                }
            }
        }
    }

    #[test]
    fn test_multi_line_response() {
        let datafiles = "test_files/raw_multiline.txt";
        let mystr = fs::read_to_string(datafiles).expect("Cannot read from file");
        let additionals = mystr
            .lines()
            .skip(3)
            .step_by(2)
            .filter(|f| f.starts_with(r#"[["wrb.fr""#))
            .max_by_key(|line| line.len())
            .unwrap_or_default();
        let raw_resp: RawResponseContainerVec =
            serde_json::from_str(additionals).expect("Error in parsing");
        let inner_obj = &raw_resp.resp[0].payload.as_ref().unwrap();
        let jd: &mut serde_json::Deserializer<serde_json::de::StrRead<'_>> =
            &mut serde_json::Deserializer::from_str(inner_obj);
        let result: Result<RawResponse, _> = serde_path_to_error::deserialize(jd);

        assert!(result.is_ok())
    }

    #[test]
    fn it_works_check_low_price_is_some() -> Result<()> {
        let my_string =
            fs::read_to_string("test_files/low_price_in_second_line.txt").expect("error here");

        let outer: Vec<RawResponseContainerVec> = decode_outer_object(my_string.as_ref())?;

        let inner_objects: Vec<String> = outer
            .into_iter()
            .flat_map(|f| f.resp)
            .filter_map(|f| f.payload)
            .collect();

        let inner: Vec<RawResponse> = inner_objects
            .into_iter()
            .flat_map(|f| decode_inner_object(&f))
            .collect();

        let low_price_usual: Vec<Option<i32>> = inner
            .iter()
            .map(|f| {
                f.price_graph
                    .as_ref()
                    .map(|f| f.usual_price_low_bound.price)
            })
            .filter(|f| f.is_some())
            .collect();

        assert!(low_price_usual.first().unwrap().is_some());
        Ok(())
    }

    #[test]
    fn test_return_response() -> Result<()> {
        let datafiles = "test_files/response_with_first_fixed_full.txt";
        let mystr = fs::read_to_string(datafiles).expect("Cannot read from file");
        let result = create_raw_response_vec(mystr);

        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_hour_can_be_empty() {
        let hour_str = "{}".to_string();
        let hour = serde_json::from_str::<Hour>(&hour_str);
        assert!(hour.is_ok());
        let parsed = serde_json::to_string(&hour.unwrap()).unwrap();
        let res = r#"{"hour":null,"minute":0}"#.to_string();
        assert_eq!(parsed, res);
    }

    #[test]
    fn test_parse_itinerary_nonstop_has_zero_stop_count() {
        // Direct LUX→MXP on Luxair: no connection_info (null at index 13)
        let mystr = r#"["LG", ["Luxair"], [[null, null, null, "LUX", "Luxembourg Airport", "Milan Malpensa Airport", "MXP", null, [11, 10], null, [12, 25], 75, [], 1, "76 cm", [["AZ", "7879", null, "ITA"]], 1, "De Havilland-Bombardier Dash-8", null, false, [2024, 1, 27], [2024, 1, 27], ["LG", "6993", null, "Luxair"], null, null, 1, null, null, null, null, "76 centimetres", 35968]], "LUX", [2024, 1, 27], [11, 10], "MXP", [2024, 1, 27], [12, 25], 75, null, null, false, null, null, null, ["ITA"], "VDOwRb", [[1705070296848121, 139803069, 858572], null, null, null, null, [[6]]], 1, null, null, [null, null, 1, -58, null, true, true, 36000, 86000, [true], 119000, 1, false], [1], [["LG", "Luxair", "https://www.luxair.lu/en/information/passenger-assistance"]]]"#;
        let it: Itinerary = serde_json::from_str(mystr).expect("parse failed");
        assert_eq!(it.stop_count(), 0);
        assert_eq!(it.total_time_minutes, 75);
        assert_eq!(it.flight_by, "LG");
        assert_eq!(it.flight_details[0].leg_duration_minutes, Some(75));
    }

    #[test]
    fn test_cheaper_travel_different_places_can_be_empty() {
        let cheaper_travel_str = "[]".to_string();
        let cheaper_travel =
            serde_json::from_str::<CheaperTravelDifferentPlaces>(&cheaper_travel_str);
        assert!(cheaper_travel.is_ok());
        let parsed = serde_json::to_string(&cheaper_travel.unwrap()).unwrap();
        let res = r#"{"dates":null}"#.to_string();
        assert_eq!(parsed, res);
    }

    #[test]
    fn test_test_return_response() -> Result<()> {
        let datafiles = [
            "test_files/error0.txt",
            "test_files/error1.txt",
            "test_files/with_28_elements.txt",
        ]
        .to_vec();

        for datafile in datafiles.iter() {
            let mystr = fs::read_to_string(datafile).expect("Cannot read from file");
            let other: Result<RawResponse, _> = decode_inner_object(&mystr);
            match other {
                Ok(_) => assert!(other.is_ok()),
                Err(err) => {
                    panic!("{} (file: {:?})", err, datafile)
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Date::serialize_to_web
    // -----------------------------------------------------------------------

    #[test]
    fn date_serialize_to_web_valid() {
        let d = Date {
            year: 2024,
            month: 3,
            day: 15,
        };
        assert_eq!(d.serialize_to_web().unwrap(), "2024-03-15");
    }

    #[test]
    fn date_serialize_to_web_invalid_returns_err() {
        let d = Date {
            year: 2024,
            month: 13, // invalid month
            day: 1,
        };
        assert!(d.serialize_to_web().is_err());
    }

    // -----------------------------------------------------------------------
    // FlightInfo::serialize_to_web
    // -----------------------------------------------------------------------

    #[test]
    fn flight_info_serialize_to_web_produces_correct_format() {
        let fi = FlightInfo {
            departure_airport_code: "LHR".to_owned(),
            destination_airport_code: "JFK".to_owned(),
            departure_time: Hour::default(),
            arrival_time: Hour::default(),
            leg_duration_minutes: Some(420),
            departure_date: Date {
                year: 2025,
                month: 8,
                day: 1,
            },
            arrival_date: Date {
                year: 2025,
                month: 8,
                day: 1,
            },
            airplane_info: AirplaneInfo {
                code: "BA".to_owned(),
                flight_number: "175".to_owned(),
                plane_crew_by: None,
                name: "Boeing 747".to_owned(),
            },
            aircraft: None,
            legroom: None,
            amenities: None,
            overnight: None,
            co2_emissions_g: None,
        };
        let serialized = fi.serialize_to_web().unwrap();
        assert!(serialized.contains("LHR"));
        assert!(serialized.contains("JFK"));
        assert!(serialized.contains("2025-08-01"));
        assert!(serialized.contains("BA"));
        assert!(serialized.contains("175"));
    }

    // -----------------------------------------------------------------------
    // CheaperTravelDifferentDates Display + maybe_get_date_price
    // -----------------------------------------------------------------------

    #[test]
    fn cheaper_travel_different_dates_display_with_return() {
        let entry = CheaperTravelDifferentDates {
            proposed_departure_date: NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
            proposed_return_date: Some(NaiveDate::from_ymd_opt(2025, 8, 15).unwrap()),
            proposed_trip_cost: Some(TripCostContainer {
                trip_cost: TripCost {
                    unknown: None,
                    price: 499,
                },
                cost_protobuf: String::new(),
            }),
        };
        let s = format!("{}", entry);
        assert!(s.contains("2025-08-01"), "should contain departure date");
        assert!(s.contains("2025-08-15"), "should contain return date");
        assert!(s.contains("499"), "should contain price");
    }

    #[test]
    fn cheaper_travel_different_dates_display_one_way() {
        let entry = CheaperTravelDifferentDates {
            proposed_departure_date: NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
            proposed_return_date: None,
            proposed_trip_cost: None,
        };
        let s = format!("{}", entry);
        assert!(s.contains("One way"), "should indicate one-way");
        assert!(s.contains("2025-08-01"), "should contain departure date");
    }

    #[test]
    fn maybe_get_date_price_returns_none_when_no_cost() {
        let entry = CheaperTravelDifferentDates {
            proposed_departure_date: NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
            proposed_return_date: None,
            proposed_trip_cost: None,
        };
        assert_eq!(entry.maybe_get_date_price(), None);
    }

    #[test]
    fn maybe_get_date_price_returns_date_and_price() {
        let entry = CheaperTravelDifferentDates {
            proposed_departure_date: NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(),
            proposed_return_date: None,
            proposed_trip_cost: Some(TripCostContainer {
                trip_cost: TripCost {
                    unknown: None,
                    price: 350,
                },
                cost_protobuf: String::new(),
            }),
        };
        let result = entry.maybe_get_date_price();
        assert_eq!(
            result,
            Some((NaiveDate::from_ymd_opt(2025, 8, 1).unwrap(), 350))
        );
    }

    // -----------------------------------------------------------------------
    // FlightResponseContainer::get_usual_price_bound
    // -----------------------------------------------------------------------

    #[test]
    fn flight_response_container_get_usual_price_bound_returns_none_when_no_price_graph() {
        let container = FlightResponseContainer {
            responses: vec![RawResponse {
                best_flights: None,
                other_flights: None,
                price_graph: None,
                travel_cheaper_different_date: None,
            }],
        };
        assert_eq!(container.get_usual_price_bound(), None);
    }

    // -----------------------------------------------------------------------
    // FlightResponseContainer::get_all_flights — deduplication
    // -----------------------------------------------------------------------

    fn make_itinerary_container(token: &str) -> ItineraryContainer {
        ItineraryContainer {
            itinerary: Itinerary {
                flight_by: "LX".to_string(),
                flight_details: vec![],
                total_time_minutes: 120,
                connection_info: None,
                emissions: None,
                self_transfer: None,
            },
            itinerary_cost: ItineraryCost {
                trip_cost: Some(TripCost {
                    unknown: None,
                    price: 100,
                }),
                departure_token: token.to_string(),
            },
            departure_protobuf: String::new(),
            mixed_cabin: None,
        }
    }

    #[test]
    fn get_all_flights_deduplicates_by_token() {
        // Two responses share "tok_b"; only one copy should appear in the output.
        let container = FlightResponseContainer {
            responses: vec![
                RawResponse {
                    best_flights: Some(ItineraryContainerList {
                        itinerary_list: vec![
                            make_itinerary_container("tok_a"),
                            make_itinerary_container("tok_b"),
                        ],
                    }),
                    other_flights: None,
                    price_graph: None,
                    travel_cheaper_different_date: None,
                },
                RawResponse {
                    best_flights: Some(ItineraryContainerList {
                        itinerary_list: vec![
                            make_itinerary_container("tok_b"), // duplicate
                            make_itinerary_container("tok_c"),
                        ],
                    }),
                    other_flights: None,
                    price_graph: None,
                    travel_cheaper_different_date: None,
                },
            ],
        };

        let flights = container.get_all_flights();
        assert_eq!(
            flights.len(),
            3,
            "expected 3 unique tokens, got: {}",
            flights.len()
        );

        let tokens: std::collections::HashSet<&str> = flights
            .iter()
            .map(|f| f.itinerary_cost.departure_token.as_str())
            .collect();
        assert!(tokens.contains("tok_a"));
        assert!(tokens.contains("tok_b"));
        assert!(tokens.contains("tok_c"));
    }

    #[test]
    fn get_all_flights_empty_container_returns_empty_vec() {
        let container = FlightResponseContainer { responses: vec![] };
        assert!(container.get_all_flights().is_empty());
    }

    #[test]
    fn get_all_flights_merges_best_and_other_flights() {
        // One response with both best_flights and other_flights should return all.
        let container = FlightResponseContainer {
            responses: vec![RawResponse {
                best_flights: Some(ItineraryContainerList {
                    itinerary_list: vec![make_itinerary_container("best_1")],
                }),
                other_flights: Some(ItineraryContainerList {
                    itinerary_list: vec![make_itinerary_container("other_1")],
                }),
                price_graph: None,
                travel_cheaper_different_date: None,
            }],
        };

        let flights = container.get_all_flights();
        assert_eq!(flights.len(), 2);
        let tokens: std::collections::HashSet<&str> = flights
            .iter()
            .map(|f| f.itinerary_cost.departure_token.as_str())
            .collect();
        assert!(tokens.contains("best_1"));
        assert!(tokens.contains("other_1"));
    }

    #[test]
    fn get_all_flights_all_duplicates_returns_single_copy() {
        // Three responses all sending the same token → exactly one result.
        let container = FlightResponseContainer {
            responses: (0..3)
                .map(|_| RawResponse {
                    best_flights: Some(ItineraryContainerList {
                        itinerary_list: vec![make_itinerary_container("only_one")],
                    }),
                    other_flights: None,
                    price_graph: None,
                    travel_cheaper_different_date: None,
                })
                .collect(),
        };

        let flights = container.get_all_flights();
        assert_eq!(flights.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Structural regression tests: parse real fixtures and check invariants
    //
    // The `lux_*_oneway.txt` fixtures are already-decoded inner JSON
    // (RawResponse format), not the raw multi-line wrb.fr envelope.
    // We parse them with serde_json directly and exercise the helper methods.
    // -----------------------------------------------------------------------

    fn parse_fixture(path: &str) -> RawResponse {
        let body =
            fs::read_to_string(path).unwrap_or_else(|_| panic!("cannot read fixture: {path}"));
        serde_json::from_str(&body).unwrap_or_else(|e| panic!("failed to parse {path}: {e}"))
    }

    /// `lux_tokyo_oneway.txt` parses without error and contains at least one flight.
    #[test]
    fn fixture_lux_tokyo_oneway_non_empty_flights() {
        let resp = parse_fixture("test_files/lux_tokyo_oneway.txt");
        let flights = resp.maybe_get_all_flights().unwrap_or_default();
        assert!(
            !flights.is_empty(),
            "expected at least one flight in lux_tokyo fixture"
        );
    }

    /// All airline codes in the Tokyo fixture are non-empty.
    #[test]
    fn fixture_lux_tokyo_oneway_all_airlines_non_empty() {
        let resp = parse_fixture("test_files/lux_tokyo_oneway.txt");
        for flight in resp.maybe_get_all_flights().unwrap_or_default() {
            assert!(
                !flight.itinerary.flight_by.is_empty(),
                "flight_by should be non-empty"
            );
        }
    }

    /// All departure tokens in the Tokyo fixture are non-empty.
    #[test]
    fn fixture_lux_tokyo_oneway_all_departure_tokens_non_empty() {
        let resp = parse_fixture("test_files/lux_tokyo_oneway.txt");
        for flight in resp.maybe_get_all_flights().unwrap_or_default() {
            assert!(
                !flight.itinerary_cost.departure_token.is_empty(),
                "departure_token should be non-empty"
            );
        }
    }

    /// All total_time_minutes values in the Tokyo fixture are positive.
    #[test]
    fn fixture_lux_tokyo_oneway_total_time_positive() {
        let resp = parse_fixture("test_files/lux_tokyo_oneway.txt");
        for flight in resp.maybe_get_all_flights().unwrap_or_default() {
            assert!(
                flight.itinerary.total_time_minutes > 0,
                "total_time_minutes must be > 0"
            );
        }
    }

    /// All airport codes in the Tokyo fixture are 3-char uppercase ASCII.
    #[test]
    fn fixture_lux_tokyo_oneway_airport_codes_are_3char_uppercase() {
        let resp = parse_fixture("test_files/lux_tokyo_oneway.txt");
        for flight in resp.maybe_get_all_flights().unwrap_or_default() {
            for leg in &flight.itinerary.flight_details {
                let dep = &leg.departure_airport_code;
                let arr = &leg.destination_airport_code;
                // Airport codes are 3-char uppercase ASCII; city identifiers start with /
                if !dep.starts_with('/') {
                    assert_eq!(dep.len(), 3, "departure code '{dep}' should be 3 chars");
                    assert!(
                        dep.chars().all(|c| c.is_ascii_uppercase()),
                        "departure code '{dep}' should be uppercase ASCII"
                    );
                }
                if !arr.starts_with('/') {
                    assert_eq!(arr.len(), 3, "arrival code '{arr}' should be 3 chars");
                    assert!(
                        arr.chars().all(|c| c.is_ascii_uppercase()),
                        "arrival code '{arr}' should be uppercase ASCII"
                    );
                }
            }
        }
    }

    /// `lux_dubai_oneway.txt` parses without error and contains at least one flight.
    #[test]
    fn fixture_lux_dubai_oneway_non_empty_flights() {
        let resp = parse_fixture("test_files/lux_dubai_oneway.txt");
        let flights = resp.maybe_get_all_flights().unwrap_or_default();
        assert!(
            !flights.is_empty(),
            "expected at least one flight in lux_dubai fixture"
        );
    }

    /// All departure tokens in the Dubai fixture are non-empty.
    #[test]
    fn fixture_lux_dubai_oneway_all_departure_tokens_non_empty() {
        let resp = parse_fixture("test_files/lux_dubai_oneway.txt");
        for flight in resp.maybe_get_all_flights().unwrap_or_default() {
            assert!(
                !flight.itinerary_cost.departure_token.is_empty(),
                "departure_token should be non-empty"
            );
        }
    }

    /// All total_time_minutes values in the Dubai fixture are positive.
    #[test]
    fn fixture_lux_dubai_oneway_total_time_positive() {
        let resp = parse_fixture("test_files/lux_dubai_oneway.txt");
        for flight in resp.maybe_get_all_flights().unwrap_or_default() {
            assert!(
                flight.itinerary.total_time_minutes > 0,
                "total_time_minutes must be > 0"
            );
        }
    }

    // -- Date::to_naive -------------------------------------------------------

    #[test]
    fn date_to_naive_valid() {
        use chrono::Datelike;
        let d = Date {
            year: 2026,
            month: 9,
            day: 10,
        };
        let naive = d.to_naive().unwrap();
        assert_eq!(naive.year(), 2026);
        assert_eq!(naive.month(), 9);
        assert_eq!(naive.day(), 10);
    }

    #[test]
    fn date_to_naive_zero_returns_none() {
        let d = Date {
            year: 0,
            month: 0,
            day: 0,
        };
        assert!(d.to_naive().is_none());
    }

    // -- Itinerary::arrives_next_day / arrival_date ---------------------------

    fn make_flight_info(dep: (i32, i32, i32), arr: (i32, i32, i32)) -> FlightInfo {
        FlightInfo {
            departure_date: Date {
                year: dep.0,
                month: dep.1,
                day: dep.2,
            },
            arrival_date: Date {
                year: arr.0,
                month: arr.1,
                day: arr.2,
            },
            ..Default::default()
        }
    }

    fn make_itinerary_with_legs(legs: Vec<FlightInfo>) -> Itinerary {
        Itinerary {
            flight_by: "LX".to_string(),
            flight_details: legs,
            total_time_minutes: 0,
            connection_info: None,
            emissions: None,
            self_transfer: None,
        }
    }

    #[test]
    fn arrives_next_day_same_day_is_false() {
        let it = make_itinerary_with_legs(vec![make_flight_info((2026, 9, 10), (2026, 9, 10))]);
        assert!(!it.arrives_next_day());
    }

    #[test]
    fn arrives_next_day_next_day_is_true() {
        let it = make_itinerary_with_legs(vec![make_flight_info((2026, 9, 10), (2026, 9, 11))]);
        assert!(it.arrives_next_day());
    }

    #[test]
    fn arrives_next_day_multi_leg_uses_last_arrival() {
        // Two legs: dep 9-10, conn same day, arr 9-11 (overnight)
        let it = make_itinerary_with_legs(vec![
            make_flight_info((2026, 9, 10), (2026, 9, 10)),
            make_flight_info((2026, 9, 10), (2026, 9, 11)),
        ]);
        assert!(it.arrives_next_day());
    }

    #[test]
    fn arrives_next_day_empty_legs_is_false() {
        let it = make_itinerary_with_legs(vec![]);
        assert!(!it.arrives_next_day());
    }

    #[test]
    fn arrival_date_returns_last_leg_arrival() {
        let it = make_itinerary_with_legs(vec![
            make_flight_info((2026, 9, 10), (2026, 9, 10)),
            make_flight_info((2026, 9, 10), (2026, 9, 11)),
        ]);
        let arr = it.arrival_date().unwrap();
        assert_eq!(arr, NaiveDate::from_ymd_opt(2026, 9, 11).unwrap());
    }

    #[test]
    fn departure_date_returns_first_leg_departure() {
        let it = make_itinerary_with_legs(vec![make_flight_info((2026, 9, 10), (2026, 9, 10))]);
        let dep = it.departure_date().unwrap();
        assert_eq!(dep, NaiveDate::from_ymd_opt(2026, 9, 10).unwrap());
    }
    #[test]
    fn amenities_decode_from_known_slots() {
        // slot 1 = wifi, slot 5 = power, slot 9 = on-demand video, slot 11 = legroom rating
        let raw = serde_json::json!([
            null, true, null, null, null, false, null, null, null, true, null, 2
        ]);
        let a: Amenities = serde_json::from_value(raw).unwrap();
        assert_eq!(a.wifi, Some(true));
        assert_eq!(a.power, Some(false));
        assert_eq!(a.on_demand_video, Some(true));
        assert_eq!(a.legroom_rating, Some(2));
        assert!(!a.is_empty());
    }

    #[test]
    fn amenities_all_null_is_empty() {
        let raw = serde_json::json!([null, null, null]);
        let a: Amenities = serde_json::from_value(raw).unwrap();
        assert!(a.is_empty());
    }

    #[test]
    fn flight_info_decodes_aircraft_legroom_amenities_and_leg_co2() {
        // Positional leg array: 3 = departure, 6 = arrival, 8/10 = times,
        // 11 = duration, 12 = amenities, 14 = legroom short, 17 = aircraft,
        // 19 = overnight, 20/21 = dates, 22 = airline info, 30 = legroom long,
        // 31 = leg CO2 grams.
        let mut leg = vec![serde_json::Value::Null; 32];
        leg[3] = serde_json::json!("BKK");
        leg[6] = serde_json::json!("HKG");
        leg[8] = serde_json::json!([0, 20]);
        leg[10] = serde_json::json!([4, 30]);
        leg[11] = serde_json::json!(190);
        leg[12] = serde_json::json!([
            null, true, null, null, null, true, null, null, null, null, null, 3
        ]);
        leg[14] = serde_json::json!("71 cm");
        leg[17] = serde_json::json!("Airbus A321neo");
        leg[19] = serde_json::json!(true);
        leg[20] = serde_json::json!([2026, 10, 9]);
        leg[21] = serde_json::json!([2026, 10, 9]);
        leg[22] = serde_json::json!(["UO", "733", null, "Hong Kong Express"]);
        leg[30] = serde_json::json!("Below average legroom (71 cm)");
        leg[31] = serde_json::json!(82000);

        let fi: FlightInfo = serde_json::from_value(serde_json::Value::Array(leg)).unwrap();
        assert_eq!(fi.aircraft.as_deref(), Some("Airbus A321neo"));
        assert_eq!(fi.legroom.as_deref(), Some("Below average legroom (71 cm)"));
        assert_eq!(fi.overnight, Some(true));
        assert_eq!(fi.co2_emissions_g, Some(82000));
        let amenities = fi.amenities.expect("amenities should be present");
        assert_eq!(amenities.wifi, Some(true));
        assert_eq!(amenities.power, Some(true));
        assert_eq!(amenities.legroom_rating, Some(3));
    }

    #[test]
    fn flight_info_legroom_falls_back_to_short_form() {
        let mut leg = vec![serde_json::Value::Null; 23];
        leg[3] = serde_json::json!("BKK");
        leg[6] = serde_json::json!("HKG");
        leg[14] = serde_json::json!("71 cm");
        let fi: FlightInfo = serde_json::from_value(serde_json::Value::Array(leg)).unwrap();
        assert_eq!(fi.legroom.as_deref(), Some("71 cm"));
        assert!(fi.amenities.is_none());
    }

    #[test]
    fn itinerary_decodes_self_transfer_flag() {
        let mut raw = vec![serde_json::Value::Null; 23];
        raw[0] = serde_json::json!("UO");
        raw[2] = serde_json::json!([]);
        raw[9] = serde_json::json!(745);
        raw[12] = serde_json::json!(true);
        let it: Itinerary = serde_json::from_value(serde_json::Value::Array(raw)).unwrap();
        assert_eq!(it.self_transfer, Some(true));
    }
}
