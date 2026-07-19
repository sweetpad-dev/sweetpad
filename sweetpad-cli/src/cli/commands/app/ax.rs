//! Native macOS UI inspection and driving via the Accessibility API
//! (CLI_DESIGN §9i): snapshot a running app's element tree, match one element
//! by label/role, and press it or set its value.
//!
//! Same split as [`super::macwin`]: a small `unsafe extern` block over the
//! plain-C `AXUIElement` API — no Objective-C runtime and no binding crate —
//! with the tree model, matching and formatting above it as pure, unit-tested
//! code. The CoreFoundation declarations are repeated here rather than shared
//! with `macwin` so each module's FFI surface reads on its own; they resolve
//! to the same symbols.
//!
//! Reads work while the app is occluded or the display is asleep: the
//! accessibility hierarchy is derived from the view tree, not the display
//! pipeline that gates `cacheDisplay` (see the capture notes in `macwin`).
//! What an app *exposes* is still up to the app — an unlabeled SwiftUI view
//! shows up as a bare `AXGroup` with no title to match on.

use std::ffi::{CString, c_void};

use crate::cli::CliError;

type CFTypeRef = *const c_void;
type CFIndex = isize;
/// `AXError`; 0 is `kAXErrorSuccess`.
type AXError = i32;

const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
const KAX_ERROR_API_DISABLED: AXError = -25211;
const KAX_ERROR_NOT_IMPLEMENTED: AXError = -25208;
const KAX_ERROR_CANNOT_COMPLETE: AXError = -25204;

/// Ceiling on a single snapshot, so a pathological hierarchy (or a cycle an
/// app exposes by mistake) can't spin forever. Finder's whole tree is ~350
/// nodes, so this is far above anything real.
const NODE_BUDGET: usize = 20_000;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: CFTypeRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, idx: CFIndex) -> CFTypeRef;
    fn CFStringCreateWithCString(alloc: CFTypeRef, s: *const i8, encoding: u32) -> CFTypeRef;
    fn CFStringGetCString(s: CFTypeRef, buf: *mut i8, size: CFIndex, encoding: u32) -> u8;
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(b: CFTypeRef) -> u8;
    fn CFRelease(cf: CFTypeRef);
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        count: CFIndex,
        key_cb: *const c_void,
        val_cb: *const c_void,
    ) -> CFTypeRef;
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    fn AXUIElementSetAttributeValue(
        element: CFTypeRef,
        attribute: CFTypeRef,
        value: CFTypeRef,
    ) -> AXError;
    fn AXUIElementCopyActionNames(element: CFTypeRef, names: *mut CFTypeRef) -> AXError;
    fn AXUIElementPerformAction(element: CFTypeRef, action: CFTypeRef) -> AXError;
}

/// Whether this process (attributed to the hosting terminal app) already
/// holds the Accessibility permission.
pub fn has_accessibility_access() -> bool {
    unsafe { AXIsProcessTrustedWithOptions(std::ptr::null()) != 0 }
}

/// Trigger the one-time OS permission prompt (registers the hosting app in
/// System Settings → Privacy & Security → Accessibility). Returns
/// immediately; the grant lands on a future run.
pub fn request_accessibility_access() {
    let key = CFStr::new("AXTrustedCheckOptionPrompt");
    let options = unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            &raw const key.0,
            &raw const kCFBooleanTrue,
            1,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    };
    if options.is_null() {
        return;
    }
    unsafe {
        AXIsProcessTrustedWithOptions(options);
        CFRelease(options);
    }
}

/// The error for a missing Accessibility grant, naming the exact settings
/// pane. Mirrors `macwin::permission_error` for Screen Recording.
pub fn permission_error() -> CliError {
    CliError::new(
        "driving a UI needs the Accessibility permission — grant it to your terminal app \
         in System Settings → Privacy & Security → Accessibility, then rerun (macOS asks \
         you to quit and reopen the terminal app for it to take effect)",
    )
}

/// A CFString held for the length of a call, released on drop.
struct CFStr(CFTypeRef);

impl CFStr {
    fn new(s: &str) -> Self {
        // Interior nuls can't reach here: every caller passes either a static
        // attribute name or text already validated by `set_value`.
        let c = CString::new(s).unwrap_or_else(|_| CString::new("").expect("empty is valid"));
        CFStr(unsafe {
            CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8)
        })
    }
}

impl Drop for CFStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

/// An owned `AXUIElementRef` (or any CF value), released on drop.
struct CFOwned(CFTypeRef);

impl Drop for CFOwned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}

/// Read a CF value as a Rust `String`, if that is what it actually is.
/// Non-string attributes (sizes, positions, element refs) yield `None`
/// rather than a misread.
unsafe fn cf_string(value: CFTypeRef) -> Option<String> {
    if value.is_null() || unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return None;
    }
    // AX labels are short; a fixed buffer avoids a two-pass length dance.
    let mut buf = vec![0u8; 4096];
    let capacity = CFIndex::try_from(buf.len()).unwrap_or(CFIndex::MAX);
    let ok = unsafe {
        CFStringGetCString(
            value,
            buf.as_mut_ptr().cast::<i8>(),
            capacity,
            KCF_STRING_ENCODING_UTF8,
        )
    };
    if ok == 0 {
        return None;
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    buf.truncate(end);
    String::from_utf8(buf).ok()
}

/// One string-valued attribute of an element.
unsafe fn attr_string(element: CFTypeRef, name: &str) -> Option<String> {
    let key = CFStr::new(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, key.0, &raw mut out) };
    if err != 0 || out.is_null() {
        return None;
    }
    let owned = CFOwned(out);
    unsafe { cf_string(owned.0) }
}

/// One boolean-valued attribute; `None` when absent or not a boolean.
unsafe fn attr_bool(element: CFTypeRef, name: &str) -> Option<bool> {
    let key = CFStr::new(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, key.0, &raw mut out) };
    if err != 0 || out.is_null() {
        return None;
    }
    let owned = CFOwned(out);
    if unsafe { CFGetTypeID(owned.0) } != unsafe { CFBooleanGetTypeID() } {
        return None;
    }
    Some(unsafe { CFBooleanGetValue(owned.0) } != 0)
}

/// The action names an element advertises (`AXPress`, `AXShowMenu`, …).
unsafe fn action_names(element: CFTypeRef) -> Vec<String> {
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyActionNames(element, &raw mut out) };
    if err != 0 || out.is_null() {
        return Vec::new();
    }
    let owned = CFOwned(out);
    if unsafe { CFGetTypeID(owned.0) } != unsafe { CFArrayGetTypeID() } {
        return Vec::new();
    }
    let n = unsafe { CFArrayGetCount(owned.0) };
    (0..n)
        .filter_map(|i| unsafe { cf_string(CFArrayGetValueAtIndex(owned.0, i)) })
        .collect()
}

/// Keep an `AXIdentifier` only when a developer assigned it.
///
/// AppKit gives every view an auto-generated identifier of the form
/// `_NS:945` — an internal serial number that changes between runs and, since
/// identifiers are preferred over titles for display and matching, would
/// otherwise mask every real label (`AXWindow "_NS:34"` for a titled window).
fn developer_identifier(raw: Option<String>) -> Option<String> {
    raw.filter(|id| !id.is_empty() && !id.starts_with("_NS:"))
}

/// One element of a running app's accessibility tree. Pure data — the live
/// `AXUIElementRef` is not retained, so a snapshot can be rendered, matched
/// and serialized without holding the app hostage; [`act`] re-descends by
/// [`Node::path`] to reach the live element again.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Node {
    /// `AXRole` — `AXButton`, `AXWindow`, `AXTextField`, …
    pub role: String,
    /// The best human label: `AXTitle`, else `AXDescription`, else a
    /// string-valued `AXValue`.
    pub label: Option<String>,
    /// `AXIdentifier` — a stable, developer-assigned handle when the app
    /// sets one (`accessibilityIdentifier`). Matched like a label, and the
    /// thing to prefer in scripts because it survives copy changes.
    pub identifier: Option<String>,
    /// `AXEnabled`; a disabled element can be found but not pressed.
    pub enabled: bool,
    pub actions: Vec<String>,
    /// Child indices from the application element down to this one. Stable
    /// only as long as the UI doesn't restructure, which is why [`act`]
    /// re-checks role and label on arrival.
    pub path: Vec<usize>,
    pub children: Vec<Node>,
}

impl Node {
    /// `AXButton "Save"` — one line of a tree listing or an error's candidate
    /// list.
    #[must_use]
    pub fn describe(&self) -> String {
        match self.best_label() {
            Some(l) if !l.is_empty() => format!("{} {l:?}", self.role),
            _ => self.role.clone(),
        }
    }

    /// The identifier when set, else the label — what `--label` matches and
    /// what a listing shows.
    #[must_use]
    pub fn best_label(&self) -> Option<&str> {
        self.identifier
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(self.label.as_deref())
    }

    /// Depth-first iterator over this node and all descendants.
    pub fn walk(&self) -> impl Iterator<Item = &Node> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let node = stack.pop()?;
            // Push in reverse so siblings come out front-to-back.
            stack.extend(node.children.iter().rev());
            Some(node)
        })
    }

    /// Total nodes in this subtree.
    #[must_use]
    pub fn count(&self) -> usize {
        self.walk().count()
    }
}

/// What to match when selecting one element out of a snapshot.
#[derive(Debug, Default, Clone)]
pub struct Query {
    /// Matched against identifier and label: exact and case-insensitive
    /// first, then a case-insensitive substring if nothing matched exactly.
    pub label: Option<String>,
    /// `AXRole`, matched case-insensitively and tolerant of the `AX` prefix
    /// being left off (`button` finds `AXButton`).
    pub role: Option<String>,
    /// Which match to take when several tie, 1-based front-to-back. Without
    /// it, an ambiguous query is an error rather than a silent pick.
    pub nth: Option<usize>,
}

impl Query {
    fn matches(&self, node: &Node, exact: bool) -> bool {
        if let Some(want) = &self.role
            && !role_matches(&node.role, want)
        {
            return false;
        }
        if let Some(want) = &self.label {
            let hit = [node.identifier.as_deref(), node.label.as_deref()]
                .into_iter()
                .flatten()
                .any(|have| label_matches(have, want, exact));
            if !hit {
                return false;
            }
        }
        // A role-only query is legitimate; an empty query matches everything
        // and is rejected by the caller before it gets here.
        true
    }

    /// Whether this query names nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.label.is_none() && self.role.is_none()
    }
}

/// `button` matches `AXButton`; so does `AXButton`.
fn role_matches(have: &str, want: &str) -> bool {
    let have = have.strip_prefix("AX").unwrap_or(have);
    let want = want.strip_prefix("AX").unwrap_or(want);
    have.eq_ignore_ascii_case(want)
}

fn label_matches(have: &str, want: &str, exact: bool) -> bool {
    if exact {
        return have.eq_ignore_ascii_case(want);
    }
    have.to_lowercase().contains(&want.to_lowercase())
}

/// Find the one element a query names.
///
/// Exact label matches are tried before substring ones, so `--label Save`
/// prefers a "Save" button over a "Save As…" one instead of calling the pair
/// ambiguous. Within a tier, matching nothing or several is an error rather
/// than a pick — `--nth` is how a caller says which of a genuine tie it
/// meant.
pub fn find<'a>(root: &'a Node, query: &Query) -> Result<&'a Node, String> {
    for exact in [true, false] {
        let hits: Vec<&Node> = root.walk().filter(|n| query.matches(n, exact)).collect();
        if hits.is_empty() {
            continue;
        }
        let len = hits.len();
        match query.nth {
            Some(0) => return Err("--nth is 1-based (1 is the first)".to_string()),
            Some(n) if n <= len => return Ok(hits[n - 1]),
            Some(n) => {
                return Err(format!(
                    "--nth {n} but only {len} element{} match{}: {}",
                    if len == 1 { "" } else { "s" },
                    if len == 1 { "es" } else { "" },
                    candidates(&hits),
                ));
            }
            None if len == 1 => return Ok(hits[0]),
            None => {
                // Suggest the axis the caller hasn't already used; telling
                // someone who passed --role to narrow with --role is noise.
                let narrow = match (&query.label, &query.role) {
                    (Some(_), None) => " or narrow with --role",
                    (None, Some(_)) => " or narrow with --label",
                    _ => "",
                };
                return Err(format!(
                    "{len} elements match; pass --nth 1..{len}{narrow}: {}",
                    candidates(&hits),
                ));
            }
        }
    }
    Err(format!(
        "nothing matches {}; `sweetpad app ui tree` shows what the app exposes",
        describe_query(query),
    ))
}

/// The first few matches, for an ambiguity error.
fn candidates(hits: &[&Node]) -> String {
    const SHOWN: usize = 5;
    let mut list: Vec<String> = hits.iter().take(SHOWN).map(|n| n.describe()).collect();
    if hits.len() > SHOWN {
        list.push(format!("… and {} more", hits.len() - SHOWN));
    }
    list.join(", ")
}

fn describe_query(query: &Query) -> String {
    match (&query.label, &query.role) {
        (Some(l), Some(r)) => format!("--label {l:?} --role {r}"),
        (Some(l), None) => format!("--label {l:?}"),
        (None, Some(r)) => format!("--role {r}"),
        (None, None) => "an empty query".to_string(),
    }
}

/// Render a snapshot as an indented outline, one element per line.
#[must_use]
pub fn outline(root: &Node) -> Vec<String> {
    let mut lines = Vec::new();
    outline_into(root, 0, &mut lines);
    lines
}

fn outline_into(node: &Node, depth: usize, out: &mut Vec<String>) {
    let disabled = if node.enabled { "" } else { " (disabled)" };
    out.push(format!(
        "{}{}{disabled}",
        "  ".repeat(depth),
        node.describe()
    ));
    for child in &node.children {
        outline_into(child, depth + 1, out);
    }
}

/// A snapshot as JSON, mirroring the outline's shape. Empty fields are
/// dropped so a tree of unlabeled groups stays readable.
#[must_use]
pub fn to_json(node: &Node) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("role".into(), node.role.clone().into());
    if let Some(l) = node.label.as_deref().filter(|s| !s.is_empty()) {
        obj.insert("label".into(), l.into());
    }
    if let Some(i) = node.identifier.as_deref().filter(|s| !s.is_empty()) {
        obj.insert("identifier".into(), i.into());
    }
    if !node.enabled {
        obj.insert("enabled".into(), false.into());
    }
    if !node.actions.is_empty() {
        obj.insert("actions".into(), node.actions.clone().into());
    }
    obj.insert("path".into(), node.path.clone().into());
    if !node.children.is_empty() {
        obj.insert(
            "children".into(),
            node.children.iter().map(to_json).collect::<Vec<_>>().into(),
        );
    }
    serde_json::Value::Object(obj)
}

/// Snapshot a running app's accessibility tree, to `max_depth` levels below
/// the application element.
pub fn snapshot(pid: i32, max_depth: usize) -> Result<Node, CliError> {
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return Err(CliError::new(format!(
            "no accessibility interface for pid {pid} (not a GUI app?)"
        )));
    }
    let app = CFOwned(app);
    let mut budget = NODE_BUDGET;
    let node = unsafe { read_node(app.0, Vec::new(), max_depth, &mut budget) };
    // An app that exposes nothing at all reads as a role-less root; say so
    // rather than printing an empty tree.
    if node.role.is_empty() && node.children.is_empty() {
        return Err(CliError::new(format!(
            "pid {pid} exposes no accessibility tree — it may still be launching, or be a \
             process with no UI"
        )));
    }
    Ok(node)
}

/// Build the pure [`Node`] for a live element and, within budget and depth,
/// its descendants.
unsafe fn read_node(
    element: CFTypeRef,
    path: Vec<usize>,
    max_depth: usize,
    budget: &mut usize,
) -> Node {
    *budget = budget.saturating_sub(1);
    let mut node = Node {
        role: unsafe { attr_string(element, "AXRole") }.unwrap_or_default(),
        label: unsafe { attr_string(element, "AXTitle") }
            .or_else(|| unsafe { attr_string(element, "AXDescription") })
            .or_else(|| unsafe { attr_string(element, "AXValue") }),
        identifier: developer_identifier(unsafe { attr_string(element, "AXIdentifier") }),
        // Containers omit AXEnabled entirely; absent means "not disabled".
        enabled: unsafe { attr_bool(element, "AXEnabled") }.unwrap_or(true),
        actions: unsafe { action_names(element) },
        path,
        children: Vec::new(),
    };
    if max_depth == 0 || *budget == 0 {
        return node;
    }

    let key = CFStr::new("AXChildren");
    let mut children: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(element, key.0, &raw mut children) };
    if err != 0 || children.is_null() {
        return node;
    }
    let children = CFOwned(children);
    if unsafe { CFGetTypeID(children.0) } != unsafe { CFArrayGetTypeID() } {
        return node;
    }
    let count = unsafe { CFArrayGetCount(children.0) };
    for i in 0..count {
        if *budget == 0 {
            break;
        }
        let child = unsafe { CFArrayGetValueAtIndex(children.0, i) };
        if child.is_null() || unsafe { CFGetTypeID(child) } != unsafe { AXUIElementGetTypeID() } {
            continue;
        }
        let Ok(index) = usize::try_from(i) else {
            continue;
        };
        let mut child_path = node.path.clone();
        child_path.push(index);
        node.children
            .push(unsafe { read_node(child, child_path, max_depth - 1, budget) });
    }
    node
}

/// What to do to a matched element.
pub enum Act<'a> {
    /// Perform an AX action, e.g. `AXPress`.
    Perform(&'a str),
    /// Set `AXValue` — how text lands in a field.
    SetValue(&'a str),
}

/// Re-descend to the element `target` described and act on it.
///
/// The snapshot is pure data, so the live element has to be found again by
/// index path. The role and label are re-checked on arrival: if the UI moved
/// under us between snapshot and act, that is a clear error rather than a
/// press landing on whatever now occupies the slot.
pub fn act(pid: i32, target: &Node, action: &Act) -> Result<(), CliError> {
    if !target.enabled {
        return Err(CliError::new(format!("{} is disabled", target.describe())));
    }
    let app = unsafe { AXUIElementCreateApplication(pid) };
    if app.is_null() {
        return Err(CliError::new(format!(
            "no accessibility interface for pid {pid}"
        )));
    }
    let app = CFOwned(app);

    // Walk down by index. `CFArrayGetValueAtIndex` follows the Get rule — the
    // child belongs to its parent array — so each array is retained in `held`
    // for the rest of the descent and `current` stays a plain borrow.
    let mut current: CFTypeRef = app.0;
    let mut held: Vec<CFOwned> = Vec::new();
    for (depth, index) in target.path.iter().enumerate() {
        let key = CFStr::new("AXChildren");
        let mut children: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(current, key.0, &raw mut children) };
        if err != 0 || children.is_null() {
            return Err(moved_error(target, depth));
        }
        let children = CFOwned(children);
        let Ok(slot) = CFIndex::try_from(*index) else {
            return Err(moved_error(target, depth));
        };
        if unsafe { CFGetTypeID(children.0) } != unsafe { CFArrayGetTypeID() }
            || slot >= unsafe { CFArrayGetCount(children.0) }
        {
            return Err(moved_error(target, depth));
        }
        let child = unsafe { CFArrayGetValueAtIndex(children.0, slot) };
        if child.is_null() {
            return Err(moved_error(target, depth));
        }
        held.push(children);
        current = child;
    }

    let landed_role = unsafe { attr_string(current, "AXRole") }.unwrap_or_default();
    if landed_role != target.role {
        return Err(CliError::new(format!(
            "the UI changed under us: expected {} at that position, found {}. Re-run \
             `sweetpad app ui tree` and try again",
            target.describe(),
            if landed_role.is_empty() {
                "nothing".to_string()
            } else {
                landed_role
            },
        )));
    }

    match action {
        Act::Perform(name) => {
            let action = CFStr::new(name);
            let err = unsafe { AXUIElementPerformAction(current, action.0) };
            ax_result(err, name, target)
        }
        Act::SetValue(text) => {
            let key = CFStr::new("AXValue");
            let value = CFStr::new(text);
            let err = unsafe { AXUIElementSetAttributeValue(current, key.0, value.0) };
            ax_result(err, "AXValue", target)
        }
    }
}

fn moved_error(target: &Node, depth: usize) -> CliError {
    CliError::new(format!(
        "the UI changed under us: {} is no longer at that position (level {depth}). Re-run \
         `sweetpad app ui tree` and try again",
        target.describe(),
    ))
}

/// Turn an `AXError` into a diagnosis that names the likely cause.
fn ax_result(err: AXError, what: &str, target: &Node) -> Result<(), CliError> {
    match err {
        0 => Ok(()),
        KAX_ERROR_API_DISABLED => Err(permission_error()),
        KAX_ERROR_NOT_IMPLEMENTED => Err(CliError::new(format!(
            "{} doesn't support {what}{}",
            target.describe(),
            if target.actions.is_empty() {
                String::new()
            } else {
                format!(" (it offers {})", target.actions.join(", "))
            },
        ))),
        KAX_ERROR_CANNOT_COMPLETE => Err(CliError::new(format!(
            "{} didn't respond to {what} — the app may be busy or not accepting input",
            target.describe(),
        ))),
        other => Err(CliError::new(format!(
            "{what} on {} failed (AXError {other})",
            target.describe(),
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small tree standing in for a snapshot: a window with a couple of
    /// buttons, a text field and a disabled control.
    fn tree() -> Node {
        Node {
            role: "AXApplication".into(),
            label: Some("Demo".into()),
            enabled: true,
            path: vec![],
            children: vec![Node {
                role: "AXWindow".into(),
                label: Some("Main".into()),
                enabled: true,
                path: vec![0],
                children: vec![
                    Node {
                        role: "AXButton".into(),
                        label: Some("Save".into()),
                        enabled: true,
                        actions: vec!["AXPress".into()],
                        path: vec![0, 0],
                        ..Node::default()
                    },
                    Node {
                        role: "AXButton".into(),
                        label: Some("Save As…".into()),
                        enabled: true,
                        actions: vec!["AXPress".into()],
                        path: vec![0, 1],
                        ..Node::default()
                    },
                    Node {
                        role: "AXTextField".into(),
                        identifier: Some("search-field".into()),
                        label: Some("Search".into()),
                        enabled: true,
                        path: vec![0, 2],
                        ..Node::default()
                    },
                    Node {
                        role: "AXButton".into(),
                        label: Some("Delete".into()),
                        enabled: false,
                        path: vec![0, 3],
                        ..Node::default()
                    },
                ],
                ..Node::default()
            }],
            ..Node::default()
        }
    }

    fn label(query: &str) -> Query {
        Query {
            label: Some(query.into()),
            ..Query::default()
        }
    }

    #[test]
    fn walk_is_depth_first_front_to_back() {
        let labels: Vec<String> = tree().walk().map(Node::describe).collect();
        assert_eq!(
            labels,
            vec![
                "AXApplication \"Demo\"",
                "AXWindow \"Main\"",
                "AXButton \"Save\"",
                "AXButton \"Save As…\"",
                "AXTextField \"search-field\"",
                "AXButton \"Delete\"",
            ]
        );
    }

    #[test]
    fn exact_label_beats_substring() {
        // "Save" is a substring of "Save As…" too; the exact tier must win
        // rather than the pair reading as ambiguous.
        let tree = tree();
        let found = find(&tree, &label("Save")).expect("exact match");
        assert_eq!(found.path, vec![0, 0]);
    }

    #[test]
    fn label_match_is_case_insensitive() {
        assert_eq!(
            find(&tree(), &label("save")).expect("match").path,
            vec![0, 0]
        );
    }

    #[test]
    fn substring_matches_when_nothing_is_exact() {
        let tree = tree();
        let found = find(&tree, &label("Save As")).expect("substring match");
        assert_eq!(found.path, vec![0, 1]);
    }

    #[test]
    fn identifier_matches_and_is_preferred_as_the_label() {
        let tree = tree();
        let found = find(&tree, &label("search-field")).expect("identifier match");
        assert_eq!(found.role, "AXTextField");
        assert_eq!(found.best_label(), Some("search-field"));
    }

    #[test]
    fn role_may_omit_the_ax_prefix() {
        let query = Query {
            label: Some("Save".into()),
            role: Some("button".into()),
            ..Query::default()
        };
        assert_eq!(find(&tree(), &query).expect("match").path, vec![0, 0]);
    }

    #[test]
    fn role_narrows_an_otherwise_ambiguous_label() {
        // Substring "Save" hits two buttons; a role that only one has picks it.
        let query = Query {
            label: Some("a".into()),
            role: Some("AXTextField".into()),
            ..Query::default()
        };
        assert_eq!(find(&tree(), &query).expect("match").role, "AXTextField");
    }

    #[test]
    fn ambiguity_is_an_error_that_lists_candidates() {
        let query = Query {
            role: Some("AXButton".into()),
            ..Query::default()
        };
        let err = find(&tree(), &query).expect_err("three buttons tie");
        assert!(err.contains("3 elements match"), "{err}");
        assert!(err.contains("--nth 1..3"), "{err}");
        assert!(err.contains("AXButton \"Save\""), "{err}");
    }

    #[test]
    fn nth_picks_out_of_a_tie_front_to_back() {
        let query = Query {
            role: Some("AXButton".into()),
            nth: Some(2),
            ..Query::default()
        };
        assert_eq!(find(&tree(), &query).expect("second").path, vec![0, 1]);
    }

    #[test]
    fn nth_past_the_end_reports_how_many_there_were() {
        let query = Query {
            role: Some("AXButton".into()),
            nth: Some(9),
            ..Query::default()
        };
        let err = find(&tree(), &query).expect_err("only three");
        assert!(err.contains("only 3 elements match"), "{err}");
    }

    #[test]
    fn nth_is_one_based() {
        let query = Query {
            role: Some("AXButton".into()),
            nth: Some(0),
            ..Query::default()
        };
        assert!(find(&tree(), &query).expect_err("zero").contains("1-based"));
    }

    #[test]
    fn no_match_points_at_the_tree_command() {
        let err = find(&tree(), &label("Nonexistent")).expect_err("no such element");
        assert!(
            err.contains("nothing matches --label \"Nonexistent\""),
            "{err}"
        );
        assert!(err.contains("app ui tree"), "{err}");
    }

    #[test]
    fn outline_indents_by_depth() {
        let lines = outline(&tree());
        assert_eq!(lines[0], "AXApplication \"Demo\"");
        assert_eq!(lines[1], "  AXWindow \"Main\"");
        assert_eq!(lines[2], "    AXButton \"Save\"");
    }

    #[test]
    fn outline_marks_disabled_elements() {
        let lines = outline(&tree());
        assert!(
            lines.iter().any(|l| l.contains("\"Delete\" (disabled)")),
            "{lines:?}"
        );
    }

    #[test]
    fn json_drops_empty_fields_but_always_carries_role_and_path() {
        let value = to_json(&tree());
        let button = &value["children"][0]["children"][0];
        assert_eq!(button["role"], "AXButton");
        assert_eq!(button["label"], "Save");
        assert_eq!(button["path"], serde_json::json!([0, 0]));
        // Enabled is the norm, so it is only serialized when false.
        assert!(button.get("enabled").is_none());
        assert!(button.get("identifier").is_none());
    }

    #[test]
    fn json_marks_disabled_elements() {
        let value = to_json(&tree());
        assert_eq!(value["children"][0]["children"][3]["enabled"], false);
    }

    #[test]
    fn empty_query_is_recognized() {
        assert!(Query::default().is_empty());
        assert!(!label("x").is_empty());
    }

    #[test]
    fn appkit_serial_identifiers_are_not_developer_identifiers() {
        // `_NS:945` is AppKit's own numbering; keeping it would hide the
        // element's real title everywhere a label is shown or matched.
        assert_eq!(developer_identifier(Some("_NS:945".into())), None);
        assert_eq!(developer_identifier(Some(String::new())), None);
        assert_eq!(
            developer_identifier(Some("search-field".into())),
            Some("search-field".into())
        );
    }

    #[test]
    fn count_covers_the_whole_subtree() {
        assert_eq!(tree().count(), 6);
    }
}
