"""Live cross-validation against fli, https://github.com/punitarani/fli.

Both tools reverse-engineer the same Google Flights endpoints, so on the
same route, date, and cabin they should surface the same fare landscape.
This guards against silent wire-format drift in either tool: if a request
slot regresses — trip type, sort, stops — prices or itineraries diverge
far beyond normal cache jitter.

Requirements: RUN_LIVE_TESTS set to a non-empty value AND the fli CLI on
PATH. Skipped otherwise, so offline CI is unaffected.
"""

import datetime
import json
import os
import shutil
import subprocess

import pytest

import gflights

RUN_LIVE = bool(os.environ.get("RUN_LIVE_TESTS"))
FLI = shutil.which("fli")

pytestmark = pytest.mark.skipif(
    not (RUN_LIVE and FLI),
    reason="needs RUN_LIVE_TESTS=1 and the fli CLI on PATH",
)

ORIGIN = "BKK"
DESTINATION = "NRT"
# Price agreement tolerance. Google serves slightly different cached fare
# snapshots per session, so identical requests can differ by a few percent.
CHEAPEST_TOLERANCE = 0.10


def _search_date() -> str:
    return (datetime.date.today() + datetime.timedelta(days=60)).isoformat()


def _fli_flights(date: str) -> list[dict]:
    out = subprocess.run(
        [FLI, "flights", ORIGIN, DESTINATION, date, "--sort", "CHEAPEST", "--format", "json"],
        capture_output=True,
        text=True,
        timeout=120,
        check=True,
    )
    payload = json.loads(out.stdout)
    assert payload.get("success"), f"fli reported failure: {payload}"
    # Zero-price rows are fli parser artifacts; drop them like its docs advise.
    return [f for f in payload["flights"] if f.get("price")]


@pytest.mark.asyncio
async def test_cheapest_fare_and_itineraries_agree_with_fli():
    date = _search_date()

    client = gflights.Client(currency=gflights.Currency.USD, country="US")
    ours = await client.search(
        origin=ORIGIN,
        destination=DESTINATION,
        date=date,
        filters=gflights.SearchFilters(sort="price"),
    )
    theirs = _fli_flights(date)

    assert ours, "gflights returned no flights"
    assert theirs, "fli returned no flights"

    our_cheapest = min(f.price for f in ours if f.price)
    their_cheapest = min(f["price"] for f in theirs)
    spread = abs(our_cheapest - their_cheapest) / max(our_cheapest, their_cheapest)
    assert spread <= CHEAPEST_TOLERANCE, (
        f"cheapest fares diverged beyond {CHEAPEST_TOLERANCE:.0%}: "
        f"gflights {our_cheapest} vs fli {their_cheapest}"
    )

    # Itinerary overlap: the same physical flights should appear in both
    # result sets. Compare first-leg flight numbers.
    our_flights = {
        f"{leg.from_airport}-{leg.to_airport}"
        for f in ours
        for leg in [f.legs[0]]
    }
    their_flights = {
        f"{leg['departure_airport']['code']}-{leg['arrival_airport']['code']}"
        for f in theirs
        for leg in [f["legs"][0]]
    }
    common = our_flights & their_flights
    assert common, (
        "no first-leg overlap between gflights and fli results — "
        f"gflights: {sorted(our_flights)[:5]}, fli: {sorted(their_flights)[:5]}"
    )
