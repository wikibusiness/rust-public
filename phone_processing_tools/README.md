# Phone processing tools

Batched Python bindings for crawl phone extraction, contact-URI classification,
validation, formatting and source/frequency merging. Phone policy runs in Rust;
callers may use their existing HTML, JSON-LD and URL decoders to supply text,
structured strings and `ParsedLink` records. Native work releases the GIL once
per batch. A synchronous call still blocks its calling event loop.

## Compatibility

The bundled numbering metadata comes from Python `phonenumbers==9.0.40`, not
rlibphonenumber's default database. Normalization uses the exact tables from
`Unidecode==1.4.0`, `python-stdnum==2.2` and numeric properties from
`regex==2026.9.29`. `metadata_versions()` reports these versions.

The package preserves the platform's phone/fax/unknown precedence, copyright-year
filter, Unicode/parenthesis cleanup, URI quirks, ordered source/raw merging,
validated-first frequency sorting and one-leading-zero suffix matching.
Existing behavior includes dropping `primary` during grouping and raising on
missing raw/source lists in some validation combinations; neither is silently
fixed here. HTML trees, Pydantic models and caller input records are not mutated.

Parsing and validation use Apache-2.0 rlibphonenumber 2.2.13 with an explicit
metadata utility. Its default features remain enabled because 2.2.13 otherwise
fails to compile a leading-zero helper, but its global utility is not used.

## Build and check

```bash
PYO3_PYTHON=/path/to/platform/.venv/bin/python cargo test --no-default-features
PYO3_PYTHON=/path/to/platform/.venv/bin/python uv run --no-project --with maturin maturin build --release --sdist
uv pip install --python /path/to/platform/.venv/bin/python target/wheels/*.whl --reinstall
/path/to/platform/.venv/bin/python tests/check.py
```

The platform integration includes `scripts/benchmark_phones.py <baseline-ref>
--check`. It compares actual old/new full extraction on six repository fixtures,
randomized normalization/URI/merge inputs and all-region numbering examples. Its
optimized-Python baseline uses equivalent `str.translate` cleanup. Timings
include binding/model conversion, but exclude HTML parsing and network/model
work. Warmed scalar formatting includes the compatibility adapter's bounded
cache, so that gain is not a Rust-computation benchmark.

## Refresh the data

Run from this directory, substituting a newer **explicit** phonenumbers version
when updating numbering plans:

```bash
uv run --no-project --with grpcio-tools --with phonenumbers==9.0.40 \
  --with Unidecode==1.4.0 --with python-stdnum==2.2 --with regex==2026.9.29 \
  python tools/generate_data.py
```

Review the generated data diff, rerun the platform differential corpus, bump
Cargo's version, and release new wheels. The platform must then update its pin;
there are no runtime downloads or silent metadata updates. Regex fields are
wrapped as `^(?:pattern)$` for rlibphonenumber's regex-triplet representation;
Python capture replacement rules are converted to Rust `$1` syntax.

## License and publishing

GPL-3.0-or-later. See `LICENSE`, `NOTICE` and the upstream texts in `licenses/`.
The GPL-derived normalization tables are not relicensed as Apache/MIT. Source
distributions include the data, generator, schema, Cargo lockfile and build
configuration needed to rebuild the wheels. Publish matching source distributions
alongside wheels, and retain these notices. This package is GPL, not AGPL.
