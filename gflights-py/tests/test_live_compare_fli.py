"""Live cross-validation against fli, https://github.com/punitarani/fli.

Both tools reverse-engineer the same Google Flights endpoints, so on the
same route, date, and cabin they should surface the same fare landscape.
This guards against silent wire-format drift in either tool: if a request
slot regresses — trip type, sort, stops — prices or itineraries diverge
far beyond normal cache jitter.

Requirements: RUN_LIVE_TESTS set to a non-empty value AND the fli CLI on
PATH. Install fli with ``uv pip install --group live`` (the ``live`` dependency
group in pyproject.toml). Skipped otherwise, so offline CI is unaffected.
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
# Minimum itinerary-overlap ratio between the two tools. Computed as the
# overlap coefficient |A∩B| / min(|A|, |B|) over (airline, flight_number)
# pairs, so it stays meaningful even when the two result sets differ in size.
# Both tools query the same endpoint, so the smaller set's physical flights
# should overwhelmingly appear in the other; a low bar here still catches a
# wire-format regression far more sharply than merely asserting non-empty.
OVERLAP_MIN_RATIO = 0.30


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
    # result sets. Identify each leg by its (airline, flight_number) pair —
    # the marketing designator that pins a specific flight — rather than by
    # airport codes, which are constant for a point-to-point route and so make
    # the overlap trivially non-empty. Both tools parse this designator from
    # the same raw response slot, so the pairs are directly comparable.
    our_flights = {
        (leg.airline_code, leg.flight_number)
        for f in ours
        for leg in f.legs
        if leg.flight_number
    }
    their_flights = {
        (leg["airline"]["code"], leg["flight_number"])
        for f in theirs
        for leg in f["legs"]
        if leg.get("flight_number")
    }
    assert our_flights, "gflights returned no flight-number designators"
    assert their_flights, "fli returned no flight-number designators"

    common = our_flights & their_flights
    # Overlap coefficient: fraction of the smaller set's flights also present
    # in the other. Robust to the two tools returning different result counts.
    ratio = len(common) / min(len(our_flights), len(their_flights))
    assert ratio >= OVERLAP_MIN_RATIO, (
        f"itinerary overlap {ratio:.0%} below the {OVERLAP_MIN_RATIO:.0%} "
        f"threshold — gflights: {sorted(our_flights)[:5]}, "
        f"fli: {sorted(their_flights)[:5]}, common: {sorted(common)[:5]}"
    )
