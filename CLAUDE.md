# gflights — Claude instructions

## Workspace layout

```
google-flights-rs/
├── src/                    Rust library + CLI
│   ├── bin/cli/            CLI subcommands
│   ├── parsers/            Request builders & response parsers
│   └── requests/           ApiClient, Config, retry logic
├── gflights-py/            Python bindings (pyo3 + maturin)
│   ├── src/lib.rs          Rust extension (_gflights)
│   ├── gflights/           Python package (re-exports, stubs, types)
│   └── tests/              Python test suite
├── benches/                Criterion benchmarks
└── tests/                  Rust integration / live API tests
```

---

## Git workflow

- **Never commit directly to `master`** — always use a feature or fix branch.
- Branch naming: `feat/<topic>`, `fix/<topic>`, `chore/<topic>`.
- After every feature or fix is complete, run `/verify` to confirm the change works end-to-end at the CLI / Python surface before merging. The `--profile dev` build is sufficient for verification.

---

## Before every commit — run locally first

Always verify locally before pushing. CI runs the same checks and failing there wastes time.

### Rust crate

```sh
cargo fmt                                        # format (required — CI blocks on diff)
cargo clippy --all-targets -- -D warnings        # lint (zero warnings policy)
cargo test --lib                                 # 152 unit tests
cargo test --bin gflights                        # 13 CLI tests
cargo test --doc                                 # doc tests
cargo build --benches                            # ensure benchmarks still compile
```

### Python bindings (run from `gflights-py/`)

```sh
cd gflights-py
python -m maturin develop                        # rebuild extension after Rust changes
.venv/Scripts/pytest.exe tests/test_import.py tests/test_types.py tests/test_errors.py -v
```

All offline tests must pass before pushing.

---

## Live / integration tests

These hit the real Google Flights API.  They are skipped unless the
`RUN_LIVE_TESTS` environment variable is set to a non-empty value.

### Rust
```sh
RUN_LIVE_TESTS=1 cargo test --test live_api
```

### Python
```sh
cd gflights-py
RUN_LIVE_TESTS=1 .venv/Scripts/pytest.exe tests/test_live.py -v
```

---

## Test coverage

Keep line coverage **≥ 80%** for the Rust crate.

```sh
cargo install cargo-tarpaulin          # one-time
cargo tarpaulin --out Stdout           # check coverage
```

Current baseline: **84%** overall (parsers 84–99%; `api.rs` ~26% — network code, accepted).
If coverage drops below 80%, add tests before merging.

---

## Examples parity rule

Every user-facing **action** (something a user runs to get a result) must have a
corresponding example in `examples/`. Technical / infrastructure features
(proxy, user-agent rotation, retry, rate-limiting) do **not** need an example —
they are exercised through the action examples and the live tests.

| Action | Example file |
|---|---|
| Flight search + offers + booking URL | `examples/flights.rs` |
| Price graph | `examples/graph.rs` |
| Date grid | `examples/date_grid.rs` |
| Cheapest dates (one-way + round-trip) | `examples/cheapest_dates.rs` |
| Booking offers + URL resolution | `examples/offer.rs` |
| Multi-city search | `examples/multi_city.rs` |
| Explore destinations | `examples/explore.rs` |
| Flight deals | `examples/deals.rs` |

When adding a new public **action**, add or update the relevant example. All examples guard network calls behind `RUN_LIVE=1` so `cargo test --examples` passes offline.

```sh
cargo build --examples                   # must compile clean
RUN_LIVE=1 cargo run --example <name>    # smoke-test with network
```

---

## Python bindings parity rule

Whenever `src/` (the Rust crate) changes, update `gflights-py/` to match:

| Rust change | Bindings update needed |
|---|---|
| New public method / field / type | Expose in `gflights-py/src/lib.rs`; add to `gflights/_gflights.pyi` |
| Renamed / removed API | Mirror in bindings |
| New `Config` option or filter | Add parameter to the affected Python method(s) |
| New response field | Expose on the relevant Python data class |
| Behaviour change | Update affected Python tests |

The Rust crate and Python bindings must stay in sync at all times.

---

## Building the Python extension

```sh
cd gflights-py
uv venv --python 3.11 .venv            # one-time
uv pip install maturin pytest pytest-asyncio
python -m maturin develop              # build + install into .venv
```

After any change to `gflights-py/src/lib.rs`, re-run `maturin develop` before running Python tests.

---

## Security & dependency hygiene

```sh
cargo audit                            # check for CVEs (runs in CI)
```

Zero CVE policy — fix or justify any advisory before merging.

---

## Benchmarks

```sh
cargo bench                            # must be run from the project root (test_files/ paths)
```

Benchmarks are in `benches/parse.rs`. They use fixtures from `test_files/`.
Do not move or rename fixtures without updating the benchmark.

---

## Publishing (Rust crate)

`cargo publish --dry-run` must be clean before tagging a release.
The crate is live at https://crates.io/crates/gflights.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->
