use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use rayon::prelude::*;
use ruff_python_ast::statement_visitor::{walk_stmt, StatementVisitor};
use ruff_python_ast::visitor::{walk_expr, Visitor};
use ruff_python_ast::{Expr, Stmt};

/// One import statement, reduced to what file resolution needs.
#[derive(Debug, PartialEq)]
enum Import {
    /// `import a.b` -> ["a", "b"]; `from a.b import c` -> ["a", "b"] and ["a", "b", "c"].
    Absolute(Vec<String>),
    /// `from ..a import b` -> level 2, module ["a"], names ["b"].
    Relative {
        level: u32,
        module: Vec<String>,
        names: Vec<String>,
    },
}

#[derive(Default)]
struct ImportCollector {
    imports: Vec<Import>,
}

impl<'a> StatementVisitor<'a> for ImportCollector {
    fn visit_stmt(&mut self, stmt: &'a Stmt) {
        match stmt {
            Stmt::Import(import) => {
                for alias in &import.names {
                    self.imports
                        .push(Import::Absolute(dotted(alias.name.as_str())));
                }
            }
            Stmt::ImportFrom(import) => {
                let module = import
                    .module
                    .as_ref()
                    .map(|m| dotted(m.as_str()))
                    .unwrap_or_default();
                let names = import.names.iter().map(|a| a.name.to_string()).collect();
                if import.level > 0 {
                    self.imports.push(Import::Relative {
                        level: import.level,
                        module,
                        names,
                    });
                } else if !module.is_empty() {
                    for name in &names {
                        let mut parts = module.clone();
                        parts.push(name.clone());
                        self.imports.push(Import::Absolute(parts));
                    }
                    self.imports.push(Import::Absolute(module));
                }
            }
            _ => walk_stmt(self, stmt),
        }
    }
}

/// Detects calls that import modules by a runtime name, which static
/// resolution cannot follow: `__import__(...)` and `importlib.import_module(...)`.
#[derive(Default)]
struct DynamicImportFinder {
    found: bool,
}

impl<'a> Visitor<'a> for DynamicImportFinder {
    fn visit_expr(&mut self, expr: &'a Expr) {
        if let Expr::Call(call) = expr {
            let dynamic = match call.func.as_ref() {
                Expr::Name(name) => name.id.as_str() == "__import__",
                Expr::Attribute(attr) => {
                    attr.attr.as_str() == "import_module"
                        && matches!(attr.value.as_ref(), Expr::Name(n) if n.id.as_str() == "importlib")
                }
                _ => false,
            };
            if dynamic {
                self.found = true;
                return;
            }
        }
        walk_expr(self, expr);
    }
}

fn dotted(module: &str) -> Vec<String> {
    module.split('.').map(str::to_owned).collect()
}

/// Imports of one source file, plus whether it imports anything dynamically.
/// A file that does not parse yields no imports, like `ast.parse` raising.
fn analyze(source: &str) -> (Vec<Import>, bool) {
    let Ok(parsed) = ruff_python_parser::parse_module(source) else {
        return (Vec::new(), false);
    };
    let suite = parsed.suite();
    let mut collector = ImportCollector::default();
    collector.visit_body(suite);

    let mut dynamic = false;
    if source.contains("__import__") || source.contains("import_module") {
        let mut finder = DynamicImportFinder::default();
        finder.visit_body(suite);
        dynamic = finder.found;
    }
    (collector.imports, dynamic)
}

/// Exact-case file existence. `Path::is_file` is case-insensitive on macOS's
/// default filesystem, so a lookup is checked against the parent directory's
/// real listing, read once per directory.
#[derive(Default)]
struct DirCache {
    files: HashMap<PathBuf, HashSet<OsString>>,
}

impl DirCache {
    fn is_file(&mut self, path: &Path) -> bool {
        let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
            return false;
        };
        self.files
            .entry(parent.to_path_buf())
            .or_insert_with(|| list_files(parent))
            .contains(name)
    }

    /// `base/a/b.py`, else `base/a/b/__init__.py`.
    fn module_file(&mut self, base: &Path, parts: &[String]) -> Option<PathBuf> {
        let (last, dirs) = parts.split_last()?;
        let dir: PathBuf = dirs.iter().fold(base.to_path_buf(), |p, part| p.join(part));
        let module = dir.join(format!("{last}.py"));
        if self.is_file(&module) {
            return Some(module);
        }
        let package = dir.join(last).join("__init__.py");
        self.is_file(&package).then_some(package)
    }
}

fn list_files(dir: &Path) -> HashSet<OsString> {
    let Ok(entries) = fs::read_dir(dir) else {
        return HashSet::new();
    };
    entries
        .flatten()
        .filter(|entry| match entry.file_type() {
            Ok(t) if t.is_symlink() => fs::metadata(entry.path()).is_ok_and(|m| m.is_file()),
            Ok(t) => t.is_file(),
            Err(_) => false,
        })
        .map(|entry| entry.file_name())
        .collect()
}

fn resolve(
    file: &Path,
    imports: &[Import],
    search_roots: &[PathBuf],
    cache: &mut DirCache,
) -> BTreeSet<PathBuf> {
    let mut targets = BTreeSet::new();
    for import in imports {
        match import {
            Import::Absolute(parts) => {
                if let Some(found) = search_roots
                    .iter()
                    .find_map(|root| cache.module_file(root, parts))
                {
                    targets.insert(found);
                }
            }
            Import::Relative {
                level,
                module,
                names,
            } => {
                let mut base = file.parent().unwrap_or(file);
                for _ in 1..*level {
                    base = base.parent().unwrap_or(base);
                }
                for name in names {
                    let mut parts = module.clone();
                    parts.push(name.clone());
                    targets.extend(cache.module_file(base, &parts));
                }
                if !module.is_empty() {
                    targets.extend(cache.module_file(base, module));
                }
            }
        }
    }
    targets
}

type FileImports = (Vec<String>, bool);

fn resolve_all(
    paths: &[String],
    search_roots: &[String],
) -> Result<HashMap<String, FileImports>, PyErr> {
    let analyzed: Vec<(Vec<Import>, bool)> = paths
        .par_iter()
        .map(|path| {
            let bytes = fs::read(path).map_err(|e| PyOSError::new_err(format!("{path}: {e}")))?;
            let source = String::from_utf8(bytes)
                .map_err(|e| PyValueError::new_err(format!("{path}: not UTF-8: {e}")))?;
            Ok(analyze(&source))
        })
        .collect::<PyResult<_>>()?;

    let roots: Vec<PathBuf> = search_roots.iter().map(PathBuf::from).collect();
    let mut cache = DirCache::default();
    Ok(paths
        .iter()
        .zip(analyzed)
        .map(|(path, (imports, dynamic))| {
            let targets = resolve(Path::new(path), &imports, &roots, &mut cache)
                .into_iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            (path.clone(), (targets, dynamic))
        })
        .collect())
}

/// Resolve the import statements of each file in `paths` to the files they
/// load, searching `search_roots` in order for absolute imports (as
/// `sys.path` would) and the importing file's own package for relative ones.
///
/// Returns `{path: (resolved_files, has_dynamic_imports)}`. Imports that
/// resolve to nothing on disk (stdlib, third-party, names that are not
/// submodules) are left out. `has_dynamic_imports` is true when the file
/// calls `__import__` or `importlib.import_module`, whose targets static
/// resolution cannot see. A file that fails to parse maps to `([], False)`.
#[pyfunction]
fn resolve_imports(
    py: Python<'_>,
    paths: Vec<String>,
    search_roots: Vec<String>,
) -> PyResult<HashMap<String, FileImports>> {
    py.detach(|| resolve_all(&paths, &search_roots))
}

#[pymodule]
fn python_import_graph(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(resolve_imports, m)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imports(source: &str) -> Vec<Import> {
        analyze(source).0
    }

    fn abs(parts: &[&str]) -> Import {
        Import::Absolute(parts.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn collects_nested_and_deferred_imports() {
        let source = "import a.b\n\
                      def f():\n    from c import d\n\
                      class K:\n    if x:\n        import e\n    else:\n        import f\n\
                      try:\n    import g\nexcept ImportError, ValueError:\n    import h\n\
                      match y:\n    case 1:\n        import i\n";
        assert_eq!(
            imports(source),
            vec![
                abs(&["a", "b"]),
                abs(&["c", "d"]),
                abs(&["c"]),
                abs(&["e"]),
                abs(&["f"]),
                abs(&["g"]),
                abs(&["h"]),
                abs(&["i"]),
            ]
        );
    }

    #[test]
    fn keeps_relative_imports_unresolved() {
        assert_eq!(
            imports("from ..pkg.sub import mod\nfrom . import x, y\n"),
            vec![
                Import::Relative {
                    level: 2,
                    module: vec!["pkg".into(), "sub".into()],
                    names: vec!["mod".into()],
                },
                Import::Relative {
                    level: 1,
                    module: vec![],
                    names: vec!["x".into(), "y".into()],
                },
            ]
        );
    }

    #[test]
    fn syntax_error_yields_nothing() {
        assert_eq!(analyze("import a\ndef (:\n"), (vec![], false));
    }

    #[test]
    fn detects_dynamic_imports_only_as_calls() {
        assert!(analyze("m = __import__(name)\n").1);
        assert!(analyze("import importlib\nimportlib.import_module(n)\n").1);
        assert!(!analyze("# __import__ and import_module in a comment\nx = '__import__'\n").1);
    }

    struct TempTree(PathBuf);

    impl TempTree {
        fn new(name: &str, files: &[&str]) -> Self {
            let root = std::env::temp_dir()
                .join(format!("python_import_graph-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            for file in files {
                let path = root.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "").unwrap();
            }
            TempTree(root)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn resolves_modules_packages_and_relative_submodules() {
        let tree = TempTree::new(
            "resolve",
            &[
                "pkg/__init__.py",
                "pkg/mod.py",
                "pkg/sub/__init__.py",
                "pkg/sub/leaf.py",
                "other/x.py",
            ],
        );
        let root = &tree.0;
        let roots = vec![root.clone()];
        let mut cache = DirCache::default();
        let file = root.join("pkg/sub/__init__.py");
        let found = resolve(
            &file,
            &[
                abs(&["pkg", "mod"]),
                abs(&["pkg"]),
                abs(&["os", "path"]),
                Import::Relative {
                    level: 1,
                    module: vec![],
                    names: vec!["leaf".into()],
                },
                Import::Relative {
                    level: 2,
                    module: vec!["mod".into()],
                    names: vec!["thing".into()],
                },
            ],
            &roots,
            &mut cache,
        );
        let expected: BTreeSet<PathBuf> = ["pkg/mod.py", "pkg/__init__.py", "pkg/sub/leaf.py"]
            .iter()
            .map(|p| root.join(p))
            .collect();
        assert_eq!(found, expected);
    }

    #[test]
    fn first_search_root_wins() {
        let tree = TempTree::new("roots", &["a/m.py", "b/m.py"]);
        let roots = vec![tree.0.join("a"), tree.0.join("b")];
        let found = resolve(
            &tree.0.join("x.py"),
            &[abs(&["m"])],
            &roots,
            &mut DirCache::default(),
        );
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            vec![tree.0.join("a/m.py")]
        );
    }

    #[test]
    fn lookups_are_case_exact() {
        let tree = TempTree::new("case", &["pkg/messaging.py"]);
        let roots = vec![tree.0.clone()];
        let found = resolve(
            &tree.0.join("x.py"),
            &[abs(&["pkg", "Messaging"])],
            &roots,
            &mut DirCache::default(),
        );
        assert!(found.is_empty());
    }
}
