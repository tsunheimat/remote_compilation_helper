//! Nested-runtime prohibition gate (G013 / Asupersync blocker 44.8).
//!
//! Scan every RABS source file as Rust syntax. Test-only bodies, comments and
//! strings cannot be runtime entries. Each reviewed synchronous entry is pinned
//! to a function and exact construction/entry counts, never an entire file.
//! Async bodies are forbidden even inside a reviewed function. This is a static
//! gate, not a call-graph proof; worker I/O also refuses an active Cx at runtime.

use proc_macro2::{TokenStream, TokenTree};
use std::fs;
use std::path::{Path, PathBuf};
use syn::visit::{self, Visit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Construct,
    Enter,
}

/// The owner/caller evidence is deliberately beside each narrow exception.
/// A new site requires reviewing its callers and this gate's negative fixtures.
struct RuntimeEntry {
    file: &'static str,
    function: &'static str,
    constructs: usize,
    enters: usize,
    reason: &'static str,
}

const ALLOWED_RUNTIME_ENTRIES: &[RuntimeEntry] = &[
    RuntimeEntry {
        file: "rabs-asupersync/src/daemon_runtime.rs",
        function: "run_daemon",
        constructs: 1,
        enters: 1,
        reason: "rabsd main owns the daemon runtime and all subsystem regions",
    },
    RuntimeEntry {
        file: "rabs-wkr/src/reconnect.rs",
        function: "run",
        constructs: 1,
        enters: 2,
        reason: "rabs-wkr main calls run synchronously; session and backoff share one runtime across reconnects",
    },
    RuntimeEntry {
        file: "rabsd/src/prepared_jobs.rs",
        function: "exchange",
        constructs: 1,
        enters: 1,
        reason: "prepared_jobs::run is a synchronous CLI branch in rabsd main, before daemon boot",
    },
    RuntimeEntry {
        file: "rabsd/src/prepared_jobs/wait.rs",
        function: "run",
        constructs: 1,
        enters: 1,
        reason: "prepared_jobs::run dispatches the CLI wait branch; all polls reuse its single client runtime",
    },
    RuntimeEntry {
        file: "rabsd/src/prepared_jobs/follow.rs",
        function: "run",
        constructs: 1,
        enters: 1,
        reason: "prepared_jobs::run dispatches the CLI follow branch; all polls reuse its single client runtime",
    },
    RuntimeEntry {
        file: "rabsd/src/worker_exec.rs",
        function: "accept_tls_worker",
        constructs: 1,
        enters: 1,
        reason: "synchronous worker-exec CLI or Drivers::start joined OS thread; rejects Cx::current before constructing the one operation runtime",
    },
    RuntimeEntry {
        file: "rabsd/src/coord/secure_worker_delivery.rs",
        function: "RecordPeer::send",
        constructs: 0,
        enters: 1,
        reason: "borrows accept_tls_worker's operation runtime; outbound/remaining rejects active Cx before any I/O",
    },
    RuntimeEntry {
        file: "rabsd/src/coord/secure_worker_delivery.rs",
        function: "RecordPeer::receive",
        constructs: 0,
        enters: 1,
        reason: "borrows the same operation runtime; remaining rejects active Cx before any I/O",
    },
    RuntimeEntry {
        file: "rabsd/src/coord/secure_worker_delivery/interrupt.rs",
        function: "OperatorPeer::send_interruptibly",
        constructs: 0,
        enters: 1,
        reason: "synchronous WorkerPeer adapter borrows RecordPeer's runtime and checks remaining before entry",
    },
    RuntimeEntry {
        file: "rabsd/src/coord/secure_worker_delivery/interrupt.rs",
        function: "OperatorPeer::receive_inner",
        constructs: 0,
        enters: 1,
        reason: "synchronous WorkerPeer adapter reuses RecordPeer's runtime; remaining rejects active Cx on every read",
    },
];

#[derive(Debug)]
struct Site {
    function: String,
    kind: Kind,
    nested: bool,
}

#[derive(Default)]
struct Scanner {
    scope: Vec<String>,
    async_depth: usize,
    sites: Vec<Site>,
}

fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("test"))
    })
}

fn call_kind(path: &syn::Path) -> Option<Kind> {
    let last = path.segments.last()?.ident.to_string();
    let owner = path
        .segments
        .iter()
        .rev()
        .nth(1)
        .map(|s| s.ident.to_string());
    match last.as_str() {
        "block_on" => Some(Kind::Enter),
        "new_current_thread" | "new_multi_thread" => Some(Kind::Construct),
        "new" | "current_thread" | "multi_thread"
            if matches!(owner.as_deref(), Some("Runtime" | "RuntimeBuilder")) =>
        {
            Some(Kind::Construct)
        }
        _ => None,
    }
}

impl Scanner {
    fn record(&mut self, kind: Kind, opaque: bool) {
        self.sites.push(Site {
            function: self.scope.join("::"),
            kind,
            nested: opaque || self.async_depth > 0,
        });
    }

    /// A macro cannot hide a runtime entry. Its unexpanded tokens do not prove a
    /// synchronous context, so runtime-shaped calls inside macros fail closed.
    fn macro_tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            if let TokenTree::Group(group) = token {
                self.macro_tokens(group.stream());
            }
            let TokenTree::Ident(name) = token else {
                continue;
            };
            let name = name.to_string();
            if name == "block_on" {
                self.record(Kind::Enter, true);
            } else if matches!(name.as_str(), "new_current_thread" | "new_multi_thread")
                || (matches!(name.as_str(), "Runtime" | "RuntimeBuilder")
                    && matches!(tokens.get(index + 3), Some(TokenTree::Ident(method))
                        if matches!(method.to_string().as_str(), "new" | "current_thread" | "multi_thread")))
            {
                self.record(Kind::Construct, true);
            }
        }
    }
}

impl<'ast> Visit<'ast> for Scanner {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        if test_only(&node.attrs) {
            return;
        }
        self.scope.push(node.sig.ident.to_string());
        self.async_depth += usize::from(node.sig.asyncness.is_some());
        visit::visit_item_fn(self, node);
        self.async_depth -= usize::from(node.sig.asyncness.is_some());
        self.scope.pop();
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        if test_only(&node.attrs) {
            return;
        }
        let owner = match node.self_ty.as_ref() {
            syn::Type::Path(ty) => ty.path.segments.last().unwrap().ident.to_string(),
            _ => "<impl>".to_owned(),
        };
        self.scope.push(owner);
        visit::visit_item_impl(self, node);
        self.scope.pop();
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        if test_only(&node.attrs) {
            return;
        }
        self.scope.push(node.sig.ident.to_string());
        self.async_depth += usize::from(node.sig.asyncness.is_some());
        visit::visit_impl_item_fn(self, node);
        self.async_depth -= usize::from(node.sig.asyncness.is_some());
        self.scope.pop();
    }

    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if test_only(&node.attrs) {
            return;
        }
        if let Some((_, items)) = &node.content {
            self.scope.push(node.ident.to_string());
            for item in items {
                self.visit_item(item);
            }
            self.scope.pop();
        }
    }

    fn visit_expr_async(&mut self, node: &'ast syn::ExprAsync) {
        self.async_depth += 1;
        visit::visit_expr_async(self, node);
        self.async_depth -= 1;
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        self.async_depth += usize::from(node.asyncness.is_some());
        visit::visit_expr_closure(self, node);
        self.async_depth -= usize::from(node.asyncness.is_some());
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "block_on" {
            self.record(Kind::Enter, false);
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref()
            && let Some(kind) = call_kind(&path.path)
        {
            self.record(kind, false);
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.macro_tokens(node.tokens.clone());
    }
}

fn scan(source: &str) -> Scanner {
    let parsed = syn::parse_file(source).expect("runtime gate must parse every source file");
    let mut scanner = Scanner::default();
    if !test_only(&parsed.attrs) {
        scanner.visit_file(&parsed);
    }
    scanner
}

/// Preserve the original exhaustive src/**/*.rs surface. Do not infer source
/// coverage from module paths: cfg_attr, #[path], includes and bin roots must not
/// let a decoy module hide another file. A standalone test helper must declare
/// #![cfg(test)] itself, so the exclusion is also enforced by the compiler.
fn scan_tree(workspace: &Path, mut directories: Vec<PathBuf>) -> Vec<(PathBuf, Vec<Site>)> {
    let mut scanned = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("cannot scan {}: {error}", directory.display()))
        {
            let file = entry.unwrap().path();
            if file.is_dir() {
                directories.push(file);
            } else if file.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(&file)
                    .unwrap_or_else(|error| panic!("cannot read {}: {error}", file.display()));
                scanned.push((
                    file.strip_prefix(workspace).unwrap().to_path_buf(),
                    scan(&source).sites,
                ));
            }
        }
    }
    scanned
}

fn workspace_sites() -> Vec<(PathBuf, Vec<Site>)> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut roots = Vec::new();
    for entry in fs::read_dir(workspace).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() && (name.starts_with("rabs-") || name == "rabsd") {
            roots.push(path.join("src"));
        }
    }
    assert!(roots.len() >= 10, "scanner must find the RABS crate roots");
    let files = scan_tree(workspace, roots);
    assert!(files.len() >= 10, "scanner must find the RABS sources");
    files
}

fn allowed(file: &Path, site: &Site) -> bool {
    !site.nested
        && ALLOWED_RUNTIME_ENTRIES
            .iter()
            .any(|entry| file == Path::new(entry.file) && site.function == entry.function)
}

#[test]
fn no_rabs_crate_contains_nested_runtime_patterns() {
    let mut offenders = Vec::new();
    let files = workspace_sites();
    println!("scanned {} Rust source files", files.len());
    for (file, sites) in files {
        for site in sites {
            if !allowed(&file, &site) {
                offenders.push(format!("{}: {site:?}", file.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "unreviewed or nested runtime entries (Asupersync blocker 44.8):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn allowlist_entries_point_at_live_pattern_sites() {
    let files = workspace_sites();
    for entry in ALLOWED_RUNTIME_ENTRIES {
        let sites: Vec<_> = files
            .iter()
            .filter(|(file, _)| file == Path::new(entry.file))
            .flat_map(|(_, sites)| sites)
            .filter(|site| site.function == entry.function && !site.nested)
            .collect();
        assert_eq!(
            (
                sites
                    .iter()
                    .filter(|site| site.kind == Kind::Construct)
                    .count(),
                sites.iter().filter(|site| site.kind == Kind::Enter).count(),
            ),
            (entry.constructs, entry.enters),
            "stale or expanded runtime entry {}::{}: {}",
            entry.file,
            entry.function,
            entry.reason,
        );
    }
}

#[test]
fn the_detector_catches_known_bad_patterns() {
    let scanned = scan(
        r#"
        fn handler(rt: &tokio::runtime::Runtime) {
            rt . block_on (async { fetch().await });
            tokio::runtime::Runtime :: new ().unwrap();
            tokio::runtime::Builder :: new_current_thread ().build();
            asupersync::runtime::RuntimeBuilder::current_thread().build();
            futures::executor::block_on(async {});
        }
        "#,
    );
    assert_eq!(scanned.sites.len(), 5);
    assert!(
        scanned
            .sites
            .iter()
            .all(|site| !allowed(Path::new("bad.rs"), site))
    );
}

#[test]
fn reviewed_function_cannot_hide_nested_calls_or_exempt_its_neighbors() {
    let file = Path::new("rabs-asupersync/src/daemon_runtime.rs");
    for source in [
        "fn run_daemon() { runtime.block_on(async { other.block_on(work()); }); }",
        "async fn run_daemon() { runtime.block_on(work()); }",
        "fn run_daemon() { let f = async || runtime.block_on(work()); }",
        "fn run_daemon() { runtime.block_on(async { Runtime::new(); }); }",
        "fn run_daemon() { call!(runtime.block_on(work())); }",
        "fn unreviewed() { runtime.block_on(work()); }",
    ] {
        let scanned = scan(source);
        assert!(
            scanned.sites.iter().any(|site| !allowed(file, site)),
            "{source}"
        );
    }
    let scanned = scan("fn run_daemon() { runtime.block_on(async {}); }");
    assert_eq!(scanned.sites.len(), 1);
    assert!(allowed(file, &scanned.sites[0]));
}

#[test]
fn test_modules_and_literals_do_not_hide_production_code() {
    let scanned = scan(
        r#"
        // runtime.block_on(async {});
        /* Runtime::new(); */
        const EXAMPLE: &str = "runtime.block_on(async {})";
        #[cfg(test)] mod tests { fn case() { runtime.block_on(async {}); } }
        #[cfg(test)] mod external_tests;
        #[cfg(any(test, unix))] mod production { fn case() { runtime.block_on(async {}); } }
        mod real_child;
        "#,
    );
    assert_eq!(scanned.sites.len(), 1);
    assert_eq!(scanned.sites[0].function, "production::case");
    assert!(
        scan("#![cfg(test)] fn fixture() { Runtime::new(); }")
            .sites
            .is_empty()
    );
    assert_eq!(
        scan("#![cfg(any(test, unix))] fn live() { Runtime::new(); }")
            .sites
            .len(),
        1
    );
}

#[test]
fn module_paths_and_bin_roots_cannot_hide_source_files() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("lib.rs"),
        "mod child; #[cfg(test)] mod missing_test; include!(\"included.rs\");",
    )
    .unwrap();
    fs::write(
        root.path().join("child.rs"),
        "#[path = \"real.rs\"] mod inner; #[cfg_attr(unix, path = \"conditional.rs\")] mod pick;",
    )
    .unwrap();
    fs::write(
        root.path().join("real.rs"),
        "async fn bad() { runtime.block_on(async {}); }",
    )
    .unwrap();
    fs::create_dir(root.path().join("child")).unwrap();
    fs::write(root.path().join("child/real.rs"), "// decoy").unwrap();
    fs::write(
        root.path().join("conditional.rs"),
        "fn bad() { RuntimeBuilder::current_thread(); }",
    )
    .unwrap();
    fs::create_dir(root.path().join("bin")).unwrap();
    fs::write(
        root.path().join("bin/tool.rs"),
        "mod companion; fn main() {}",
    )
    .unwrap();
    fs::write(
        root.path().join("bin/companion.rs"),
        "fn bad() { runtime.block_on(async {}); }",
    )
    .unwrap();
    fs::write(
        root.path().join("included.rs"),
        "fn bad() { Runtime::new(); }",
    )
    .unwrap();
    let files = scan_tree(root.path(), vec![root.path().to_path_buf()]);
    assert_eq!(files.len(), 8);
    let sites: Vec<_> = files.iter().flat_map(|(_, sites)| sites).collect();
    assert_eq!(sites.len(), 4);
    assert!(sites.iter().any(|site| site.nested));
    assert!(sites.iter().any(|site| site.kind == Kind::Construct));
}
