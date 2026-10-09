# python_import_graph

Resolve the import statements of many Python files to the files they load,
in parallel. Built for dependency-graph tooling (which files does this
entry point reach? which tests or builds does a change affect?) where
walking every file with `ast` is too slow.

```python
from python_import_graph import resolve_imports

edges = resolve_imports(["src/app.py", "src/util/io.py"], search_roots=["src", "lib"])
# {"src/app.py": (["src/util/io.py", "lib/shared/__init__.py"], False), ...}
```

- Parses with [ruff](https://github.com/astral-sh/ruff)'s parser, so current
  Python syntax (3.14) is supported.
- Finds every `import` / `from ... import` statement, including ones nested
  in functions, classes, `if`/`try`/`with`/`match` blocks.
- `import a.b` resolves to `a/b.py` or `a/b/__init__.py` under the first
  search root that has it. `from a import b` resolves `a` and, when `b` is a
  submodule, `a/b.py` too. Relative imports resolve against the importing
  file's package.
- Lookups are case-exact on every platform, so results computed on a
  case-insensitive filesystem (macOS) match the ones from Linux.
- Imports that resolve to nothing on disk (stdlib, third-party packages,
  names that are attributes rather than submodules) are left out.
- Each result also says whether the file calls `__import__` or
  `importlib.import_module`, whose targets static resolution cannot see.

## Development

```bash
PYO3_PYTHON=$(which python3) cargo test --no-default-features
uv run --with maturin maturin build --release
```
