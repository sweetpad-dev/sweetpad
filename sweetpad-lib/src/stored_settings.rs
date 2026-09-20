//! The vocabulary of the stored build-settings layer, shared by the two
//! project document formats.
//!
//! The stored layer is what the project file itself assigns, before xcconfig
//! files, SDK defaults and `$(inherited)` chains are applied — one input to
//! [`crate::resolver`], and the layer `sweetpad pbxproj settings set/unset`
//! edits. Where it lives differs by format: a `buildSettings` dict per
//! `XCBuildConfiguration` in a pbxproj, one `build-settings` map per scope
//! with the configuration folded into the key in a `project.xcproj`. The
//! request and the report do not, so they live here and
//! [`crate::settings_pbxproj`] and [`crate::settings_xcproj`] both speak them.

/// Which configurations to operate on: the project-level ones (inherited by
/// every target) or a single target's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Project,
    Target(String),
}

impl Scope {
    /// The target name, when target-scoped.
    #[must_use]
    pub fn target(&self) -> Option<&str> {
        match self {
            Scope::Project => None,
            Scope::Target(name) => Some(name),
        }
    }
}

/// A raw setting value as stored: a plain string or an array of strings.
/// Xcode treats a whitespace-separated string and an array as the same list at
/// resolve time; the stored shape is preserved here so reports show exactly
/// what the file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    String(String),
    List(Vec<String>),
}

impl Setting {
    /// The value as its element list — arrays as-is, strings whitespace-split
    /// (how xcodebuild consumes list-typed settings). `KEY += v` appends to
    /// this normalized form.
    #[must_use]
    pub fn elements(&self) -> Vec<String> {
        match self {
            Setting::String(s) => s.split_whitespace().map(str::to_string).collect(),
            Setting::List(items) => items.clone(),
        }
    }

    /// Human rendering: the string itself, or elements joined with a space.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Setting::String(s) => s.clone(),
            Setting::List(items) => items.join(" "),
        }
    }

    /// The canonical stored form: a plain string for zero or one element, an
    /// array for more (the shape Xcode itself writes for multi-value
    /// settings).
    #[must_use]
    pub fn canonical(elements: &[String]) -> Setting {
        match elements {
            [] => Setting::String(String::new()),
            [one] => Setting::String(one.clone()),
            many => Setting::List(many.to_vec()),
        }
    }
}

/// How one key changes. `Assign` replaces the value outright (its `Vec` is the
/// element list — a repeated `KEY=` on the command line builds it up);
/// `Append` extends the existing value's normalized element list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Assign(Vec<String>),
    Append(Vec<String>),
}

/// One folded `KEY=…`/`KEY+=…` request. The key is the setting's own key,
/// conditional suffix included (`CODE_SIGN_IDENTITY[sdk=iphoneos*]`), and
/// never carries the configuration — that is what `configurations` selects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub key: String,
    pub op: Op,
}

/// One applied (or attempted) edit, for the report: what `key` was in
/// `configuration` before and after. `new: None` records an unset; an unset of
/// an absent key yields `old: None, new: None` (the no-op case callers note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// `None` for project scope, the target name otherwise.
    pub target: Option<String>,
    pub configuration: String,
    pub key: String,
    pub old: Option<Setting>,
    pub new: Option<Setting>,
}

/// The stored settings of one configuration, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSettings {
    pub configuration: String,
    pub settings: Vec<(String, Setting)>,
}
