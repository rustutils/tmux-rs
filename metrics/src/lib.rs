//! Counts unsafe code in the tmux-rs workspace.
//!
//! The numbers track Track 2 progress (see PLAN.md). The main ones are the
//! number of `unsafe fn`, the lines in files that forbid unsafe code, and
//! the false safe functions: functions without `unsafe` in their signature
//! that contain an unsafe block with no `SAFETY:` comment. A safe signature
//! has to be sound for every caller, so such a function either hides an
//! obligation from its callers or lacks the comment that says why it does
//! not.
//!
//! Each Rust file also has an unsafe limit, recorded in
//! `metrics/unsafe-limits.txt`. A test fails when a file goes above its
//! limit, so the count can only go down.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use proc_macro2::{TokenStream, TokenTree};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

/// Directories that are never scanned.
const SKIP_DIRS: &[&str] = &["target", ".git", "fuzz", "tests", "regress", "docs"];

/// The limit file, relative to the workspace root.
pub const LIMITS_FILE: &str = "metrics/unsafe-limits.txt";

/// Known false safe functions that cannot be marked unsafe, relative to the
/// workspace root.
pub const EXCEPTIONS_FILE: &str = "metrics/false-safe-exceptions.txt";

/// Counts for one Rust file.
#[derive(Debug, Default, Clone)]
pub struct FileStats {
    /// Path relative to the workspace root, with forward slashes.
    pub path: String,
    /// Lines in the file.
    pub lines: usize,
    /// `unsafe` keywords, including those inside macro invocations.
    pub unsafe_tokens: usize,
    /// `*mut` types and casts.
    pub mut_ptrs: usize,
    /// `static mut` items.
    pub static_muts: usize,
    /// Functions and methods declared `unsafe fn`.
    pub unsafe_fns: usize,
    /// Functions and methods in total.
    pub fns: usize,
    /// Whether the file is covered by `#![forbid(unsafe_code)]`, its own or
    /// that of a parent module.
    pub forbids_unsafe: bool,
    /// False safe functions, outside test code.
    pub false_safe: Vec<FalseSafe>,
}

/// A function without `unsafe` in its signature that contains an unsafe
/// block without a `SAFETY:` comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalseSafe {
    /// `path:Name` or `path:Type::name`, the key used in the exception file.
    pub key: String,
    /// Line of the function.
    pub line: usize,
}

/// Totals over the workspace.
#[derive(Debug, Default)]
pub struct Totals {
    pub files: usize,
    pub lines: usize,
    pub unsafe_tokens: usize,
    pub mut_ptrs: usize,
    pub static_muts: usize,
    pub unsafe_fns: usize,
    pub fns: usize,
    pub forbid_lines: usize,
    pub forbid_files: usize,
    pub false_safe: usize,
    pub bridge_calls: usize,
}

/// Finds the workspace root from this crate's manifest directory.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("metrics crate has a parent directory")
        .to_path_buf()
}

/// Lists the Rust files to scan, relative to `root`, sorted.
pub fn rust_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if entry.file_type()?.is_dir() {
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            walk(root, &path, out)?;
        } else if name.ends_with(".rs") {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    Ok(())
}

/// Path with forward slashes, as used in the limit and exception files.
fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Scans one file. `rel` is the path relative to the workspace root.
pub fn scan_source(rel: &str, src: &str) -> FileStats {
    let mut stats = FileStats {
        path: rel.to_string(),
        lines: src.lines().count(),
        ..FileStats::default()
    };
    if let Ok(tokens) = TokenStream::from_str(src) {
        count_tokens(tokens, &mut stats, &mut None);
    }
    if let Ok(file) = syn::parse_file(src) {
        stats.forbids_unsafe = has_forbid_unsafe(&file.attrs);
        let lines: Vec<&str> = src.lines().collect();
        let mut v = FnVisitor {
            path: rel,
            lines: &lines,
            stats: &mut stats,
            stack: Vec::new(),
            self_ty: Vec::new(),
            in_test: 0,
        };
        v.visit_file(&file);
    }
    stats
}

/// The previous token, for recognising `*mut` and `static mut`.
#[derive(PartialEq)]
enum Prev {
    Star,
    Static,
}

fn count_tokens(tokens: TokenStream, stats: &mut FileStats, prev: &mut Option<Prev>) {
    for tt in tokens {
        match tt {
            TokenTree::Group(g) => {
                *prev = None;
                count_tokens(g.stream(), stats, prev);
                *prev = None;
            }
            TokenTree::Ident(id) => {
                let s = id.to_string();
                if s == "unsafe" {
                    stats.unsafe_tokens += 1;
                }
                if s == "mut" {
                    match prev {
                        Some(Prev::Star) => stats.mut_ptrs += 1,
                        Some(Prev::Static) => stats.static_muts += 1,
                        None => {}
                    }
                }
                *prev = (s == "static").then_some(Prev::Static);
            }
            TokenTree::Punct(p) => {
                *prev = (p.as_char() == '*').then_some(Prev::Star);
            }
            TokenTree::Literal(_) => *prev = None,
        }
    }
}

fn has_forbid_unsafe(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("forbid") && {
            let mut found = false;
            let _ = a.parse_nested_meta(|m| {
                if m.path.is_ident("unsafe_code") {
                    found = true;
                }
                Ok(())
            });
            found
        }
    })
}

fn is_test_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if a.path().is_ident("test") {
            return true;
        }
        if a.path().is_ident("cfg") {
            let mut test = false;
            let _ = a.parse_nested_meta(|m| {
                if m.path.is_ident("test") {
                    test = true;
                }
                Ok(())
            });
            return test;
        }
        false
    })
}

/// One function on the visitor's stack.
struct FnFrame {
    key: String,
    line: usize,
    is_unsafe: bool,
    flagged: bool,
}

struct FnVisitor<'a> {
    path: &'a str,
    lines: &'a [&'a str],
    stats: &'a mut FileStats,
    stack: Vec<FnFrame>,
    self_ty: Vec<String>,
    in_test: usize,
}

impl FnVisitor<'_> {
    fn enter_fn(
        &mut self,
        name: String,
        sig: &syn::Signature,
        attrs: &[syn::Attribute],
        span_line: usize,
    ) -> bool {
        self.stats.fns += 1;
        if sig.unsafety.is_some() {
            self.stats.unsafe_fns += 1;
        }
        let test = is_test_attr(attrs);
        if test {
            self.in_test += 1;
        }
        let key = match self.self_ty.last() {
            Some(ty) => format!("{}:{}::{}", self.path, ty, name),
            None => format!("{}:{}", self.path, name),
        };
        self.stack.push(FnFrame {
            key,
            line: span_line,
            is_unsafe: sig.unsafety.is_some(),
            flagged: false,
        });
        test
    }

    fn leave_fn(&mut self, test: bool) {
        self.stack.pop();
        if test {
            self.in_test -= 1;
        }
    }

    /// Whether the unsafe block starting on `line` (1-based) has a
    /// `SAFETY:` comment on that line or in the comment lines directly
    /// above it.
    fn has_safety_comment(&self, line: usize) -> bool {
        let Some(idx) = line.checked_sub(1) else {
            return false;
        };
        if self.lines.get(idx).is_some_and(|l| l.contains("SAFETY:")) {
            return true;
        }
        let mut i = idx;
        while i > 0 {
            i -= 1;
            let l = self.lines[i].trim();
            if !l.starts_with("//") {
                break;
            }
            if l.contains("SAFETY:") {
                return true;
            }
        }
        false
    }
}

impl<'ast> Visit<'ast> for FnVisitor<'_> {
    fn visit_item_mod(&mut self, m: &'ast syn::ItemMod) {
        let test = is_test_attr(&m.attrs);
        if test {
            self.in_test += 1;
        }
        visit::visit_item_mod(self, m);
        if test {
            self.in_test -= 1;
        }
    }

    fn visit_item_impl(&mut self, i: &'ast syn::ItemImpl) {
        let ty = match &*i.self_ty {
            syn::Type::Path(p) => p
                .path
                .segments
                .last()
                .map_or_else(|| "?".to_string(), |s| s.ident.to_string()),
            _ => "?".to_string(),
        };
        self.self_ty.push(ty);
        visit::visit_item_impl(self, i);
        self.self_ty.pop();
    }

    fn visit_item_trait(&mut self, t: &'ast syn::ItemTrait) {
        self.self_ty.push(t.ident.to_string());
        visit::visit_item_trait(self, t);
        self.self_ty.pop();
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        // A free function inside a method body is not a method.
        let saved = std::mem::take(&mut self.self_ty);
        let test = self.enter_fn(
            f.sig.ident.to_string(),
            &f.sig,
            &f.attrs,
            f.sig.span().start().line,
        );
        visit::visit_item_fn(self, f);
        self.leave_fn(test);
        self.self_ty = saved;
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        let test = self.enter_fn(
            f.sig.ident.to_string(),
            &f.sig,
            &f.attrs,
            f.sig.span().start().line,
        );
        visit::visit_impl_item_fn(self, f);
        self.leave_fn(test);
    }

    fn visit_trait_item_fn(&mut self, f: &'ast syn::TraitItemFn) {
        let test = self.enter_fn(
            f.sig.ident.to_string(),
            &f.sig,
            &f.attrs,
            f.sig.span().start().line,
        );
        visit::visit_trait_item_fn(self, f);
        self.leave_fn(test);
    }

    fn visit_expr_unsafe(&mut self, e: &'ast syn::ExprUnsafe) {
        let line = e.unsafe_token.span.start().line;
        let documented = self.has_safety_comment(line);
        if self.in_test == 0
            && !documented
            && let Some(frame) = self.stack.last_mut()
            && !frame.is_unsafe
            && !frame.flagged
        {
            frame.flagged = true;
            self.stats.false_safe.push(FalseSafe {
                key: frame.key.clone(),
                line: frame.line,
            });
        }
        visit::visit_expr_unsafe(self, e);
    }
}

/// Scans every file under `root`.
pub fn scan(root: &Path) -> io::Result<Vec<FileStats>> {
    let files = rust_files(root)?;
    let mut out = Vec::with_capacity(files.len());
    for rel in &files {
        let src = fs::read_to_string(root.join(rel))?;
        out.push(scan_source(&path_key(rel), &src));
    }
    mark_forbid_coverage(&mut out);
    Ok(out)
}

/// A file is also covered when a parent module forbids unsafe code: the
/// crate root (lib.rs or main.rs), or `dir/mod.rs` or `dir.rs` for a
/// directory the file is in.
fn mark_forbid_coverage(stats: &mut [FileStats]) {
    let forbidding: Vec<String> = stats
        .iter()
        .filter(|s| s.forbids_unsafe)
        .map(|s| s.path.clone())
        .collect();
    let mut scopes: Vec<String> = Vec::new();
    for p in &forbidding {
        if let Some(dir) = p
            .strip_suffix("/lib.rs")
            .or_else(|| p.strip_suffix("/main.rs"))
            .or_else(|| p.strip_suffix("/mod.rs"))
        {
            scopes.push(format!("{dir}/"));
        } else if let Some(stem) = p.strip_suffix(".rs") {
            scopes.push(format!("{stem}/"));
        }
    }
    for s in stats.iter_mut() {
        if scopes
            .iter()
            .any(|scope| s.path.starts_with(scope.as_str()))
        {
            s.forbids_unsafe = true;
        }
    }
}

/// Reads the exception file: one key per line, `#` starts a comment.
pub fn read_exceptions(root: &Path) -> Vec<String> {
    fs::read_to_string(root.join(EXCEPTIONS_FILE))
        .unwrap_or_default()
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// False safe functions that are not in the exception file.
pub fn false_safe(stats: &[FileStats], exceptions: &[String]) -> Vec<(String, FalseSafe)> {
    stats
        .iter()
        .flat_map(|s| {
            s.false_safe
                .iter()
                .filter(|f| !exceptions.contains(&f.key))
                .map(|f| (s.path.clone(), f.clone()))
        })
        .collect()
}

/// Names of the functions defined in src/global.rs, the bridge to stores
/// that unconverted code uses (see Track 2 in PLAN.md).
fn bridge_functions(root: &Path) -> Vec<String> {
    let Ok(src) = fs::read_to_string(root.join("src/global.rs")) else {
        return Vec::new();
    };
    let Ok(file) = syn::parse_file(&src) else {
        return Vec::new();
    };
    file.items
        .iter()
        .filter_map(|i| match i {
            syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// Calls of bridge functions outside src/global.rs.
fn bridge_calls(root: &Path, files: &[FileStats]) -> usize {
    let names = bridge_functions(root);
    if names.is_empty() {
        return 0;
    }
    let mut n = 0;
    for f in files {
        if f.path == "src/global.rs" {
            continue;
        }
        let Ok(src) = fs::read_to_string(root.join(&f.path)) else {
            continue;
        };
        let Ok(tokens) = TokenStream::from_str(&src) else {
            continue;
        };
        n += count_calls(tokens, &names);
    }
    n
}

fn count_calls(tokens: TokenStream, names: &[String]) -> usize {
    let mut n = 0;
    let mut prev_name = false;
    for tt in tokens {
        match tt {
            TokenTree::Ident(id) => {
                prev_name = names.iter().any(|x| id == x);
            }
            TokenTree::Group(g) => {
                if prev_name && g.delimiter() == proc_macro2::Delimiter::Parenthesis {
                    n += 1;
                }
                prev_name = false;
                n += count_calls(g.stream(), names);
            }
            _ => prev_name = false,
        }
    }
    n
}

/// Adds up the per-file counts.
pub fn totals(root: &Path, stats: &[FileStats], exceptions: &[String]) -> Totals {
    let mut t = Totals {
        files: stats.len(),
        false_safe: false_safe(stats, exceptions).len(),
        bridge_calls: bridge_calls(root, stats),
        ..Totals::default()
    };
    for s in stats {
        t.lines += s.lines;
        t.unsafe_tokens += s.unsafe_tokens;
        t.mut_ptrs += s.mut_ptrs;
        t.static_muts += s.static_muts;
        t.unsafe_fns += s.unsafe_fns;
        t.fns += s.fns;
        if s.forbids_unsafe {
            t.forbid_lines += s.lines;
            t.forbid_files += 1;
        }
    }
    t
}

/// Reads the limit file: lines of `<limit> <path>`, `#` starts a comment.
pub fn read_limits(root: &Path) -> io::Result<BTreeMap<String, usize>> {
    let text = fs::read_to_string(root.join(LIMITS_FILE))?;
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let n = parts.next().and_then(|n| n.parse().ok());
        let path = parts.next().map(str::trim);
        if let (Some(n), Some(path)) = (n, path) {
            out.insert(path.to_string(), n);
        }
    }
    Ok(out)
}

/// Files whose unsafe count is above their limit, as `(path, count,
/// limit)`. A file not in the limit file has a limit of 0.
pub fn over_limit(
    stats: &[FileStats],
    limits: &BTreeMap<String, usize>,
) -> Vec<(String, usize, usize)> {
    stats
        .iter()
        .filter_map(|s| {
            let limit = limits.get(&s.path).copied().unwrap_or(0);
            (s.unsafe_tokens > limit).then(|| (s.path.clone(), s.unsafe_tokens, limit))
        })
        .collect()
}

/// Renders the limit file from the current counts.
pub fn render_limits(stats: &[FileStats]) -> String {
    let mut out = String::from(
        "# Unsafe limit per Rust file: the number of `unsafe` keywords it may\n\
         # contain. A test fails when a file goes above its limit. Lower the\n\
         # limits with `just unsafe-limits`. Raising one needs a reason in the\n\
         # commit message and `just unsafe-limits --allow-raise`.\n\
         # Files not listed have a limit of 0.\n",
    );
    for s in stats.iter().filter(|s| s.unsafe_tokens > 0) {
        out.push_str(&format!("{} {}\n", s.unsafe_tokens, s.path));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(src: &str) -> Vec<String> {
        scan_source("x.rs", src)
            .false_safe
            .into_iter()
            .map(|f| f.key)
            .collect()
    }

    #[test]
    fn counts_tokens() {
        let s = scan_source(
            "x.rs",
            "static mut A: *mut u8 = 0 as *mut u8;\n\
             unsafe fn f() {}\n\
             fn g() { unsafe { f() } }\n\
             macro_rules! m { () => { unsafe { } } }\n\
             // unsafe in a comment\n\
             const S: &str = \"unsafe *mut\";\n",
        );
        assert_eq!(s.static_muts, 1);
        assert_eq!(s.mut_ptrs, 2);
        assert_eq!(s.unsafe_tokens, 3);
        assert_eq!(s.unsafe_fns, 1);
        assert_eq!(s.fns, 2);
    }

    #[test]
    fn finds_false_safe_functions() {
        let src = "
fn plain() {}
fn hides() { unsafe { x() } }
unsafe fn honest() { unsafe { x() } }
fn documented() {
    // SAFETY: x has no preconditions.
    unsafe { x() }
}
fn closure() { let f = || unsafe { x() }; }
struct T;
impl Drop for T {
    fn drop(&mut self) { unsafe { x() } }
}
#[cfg(test)]
mod tests {
    fn helper() { unsafe { x() } }
}
#[test]
fn a_test() { unsafe { x() } }
";
        assert_eq!(keys(src), ["x.rs:hides", "x.rs:closure", "x.rs:T::drop"]);
    }

    #[test]
    fn nested_function_is_its_own() {
        let src = "
unsafe fn outer() {
    fn inner() { unsafe { x() } }
}
";
        assert_eq!(keys(src), ["x.rs:inner"]);
    }

    #[test]
    fn forbid_attribute() {
        assert!(scan_source("x.rs", "#![forbid(unsafe_code)]\nfn f() {}").forbids_unsafe);
        assert!(!scan_source("x.rs", "#![deny(unsafe_code)]\nfn f() {}").forbids_unsafe);
    }

    #[test]
    fn forbid_covers_child_modules() {
        let mut stats = vec![
            FileStats {
                path: "src/a.rs".into(),
                forbids_unsafe: true,
                ..FileStats::default()
            },
            FileStats {
                path: "src/a/b.rs".into(),
                ..FileStats::default()
            },
            FileStats {
                path: "src/ab.rs".into(),
                ..FileStats::default()
            },
        ];
        mark_forbid_coverage(&mut stats);
        assert!(stats[1].forbids_unsafe);
        assert!(!stats[2].forbids_unsafe);
    }

    #[test]
    fn counts_bridge_calls() {
        let tokens =
            TokenStream::from_str("with_paste(|p| p.top()); let with_paste = 1; with_paste(f)")
                .unwrap();
        assert_eq!(count_calls(tokens, &["with_paste".to_string()]), 2);
    }

    /// No function hides unsafe code behind a safe signature, apart from
    /// the listed exceptions.
    #[test]
    fn no_false_safe_functions() {
        let root = workspace_root();
        let stats = scan(&root).unwrap();
        let found = false_safe(&stats, &read_exceptions(&root));
        assert!(
            found.is_empty(),
            "functions with a safe signature and an unsafe block without a \
             SAFETY comment: {found:?}\n\
             Mark them unsafe fn, or add a SAFETY comment saying why every \
             caller is fine."
        );
    }

    /// The per-file unsafe limits in metrics/unsafe-limits.txt hold.
    #[test]
    fn unsafe_limits_hold() {
        let root = workspace_root();
        let stats = scan(&root).unwrap();
        let limits = read_limits(&root).expect("metrics/unsafe-limits.txt exists");
        let over = over_limit(&stats, &limits);
        assert!(
            over.is_empty(),
            "files above their unsafe limit (path, count, limit): {over:?}\n\
             Remove the new unsafe code, or raise the limit with a reason \
             (just unsafe-limits --allow-raise)."
        );
    }
}
