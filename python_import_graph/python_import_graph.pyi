from collections.abc import Sequence

def resolve_imports(
    paths: Sequence[str], search_roots: Sequence[str]
) -> dict[str, tuple[list[str], bool]]:
    """Resolve the import statements of each file in `paths` to the files they load.

    Absolute imports are searched in `search_roots`, in order, as `sys.path`
    would; relative imports from the importing file's own package. Returns
    `{path: (resolved_files, has_dynamic_imports)}`. Imports that resolve to
    nothing on disk (stdlib, third-party, names that are not submodules) are
    left out. `has_dynamic_imports` is true when the file calls `__import__`
    or `importlib.import_module`. A file that fails to parse maps to
    `([], False)`.
    """
