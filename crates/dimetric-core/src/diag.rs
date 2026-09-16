//! Structured diagnostics.
//!
//! Invariant I9: every failure carries a stable code, a source location and a
//! machine-readable payload. Pretty-printing is a display layer on top, never
//! the primary representation.
//!
//! One type covers scene validation, asset import and Lua errors alike, so a
//! tool — or an agent — writes one handler rather than three parsers.

use core::fmt;

use serde::{Deserialize, Serialize};

/// How much a diagnostic matters.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Context, not a problem.
    Note,
    /// Something is suspect but the operation completed.
    Warning,
    /// The operation failed.
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            Severity::Note => "note",
            Severity::Warning => "warning",
            Severity::Error => "error",
        })
    }
}

/// A stable diagnostic code such as `DIM0301`.
///
/// Codes never change meaning. A code may be retired, but it is not reused:
/// scripts and agents match on these, and a silently repurposed code turns a
/// working check into a wrong one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code(pub &'static str);

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for Code {
    /// Resolves against the registry in [`CODES`], so a deserialized code is
    /// always one the engine actually knows about. An unrecognised code is an
    /// error rather than a silently accepted string: matching on codes is the
    /// supported way to handle diagnostics, and a typo should not survive a
    /// round trip.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Code, D::Error> {
        let s = String::deserialize(d)?;
        CODES
            .iter()
            .find(|c| c.code.0 == s)
            .map(|c| c.code)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown diagnostic code {s:?}")))
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.0)
    }
}
impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// A registered code with its documentation.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CodeInfo {
    /// The code itself.
    pub code: Code,
    /// Severity the engine emits it at.
    pub severity: Severity,
    /// One-line description, used in generated docs.
    pub summary: &'static str,
}

macro_rules! codes {
    ($($konst:ident = $text:literal, $sev:ident, $summary:literal;)*) => {
        impl Code {
            $(
                #[doc = $summary]
                pub const $konst: Code = Code($text);
            )*
        }

        /// Every code the engine can emit, in numeric order.
        ///
        /// `cargo xtask gen-docs` renders this table into `docs/API.md`, so the
        /// documentation cannot drift from the implementation.
        pub static CODES: &[CodeInfo] = &[
            $(CodeInfo { code: Code($text), severity: Severity::$sev, summary: $summary },)*
        ];
    };
}

codes! {
    // 01xx — scene structure
    PARSE_FAILED      = "DIM0100", Error,   "File is not well-formed TOML";
    UNKNOWN_KIND      = "DIM0101", Error,   "Unknown node kind";
    DUPLICATE_ID      = "DIM0102", Error,   "Duplicate node id";
    DANGLING_PARENT   = "DIM0103", Error,   "Dangling parent reference";
    RESERVED_KEY      = "DIM0104", Error,   "Property shadows a reserved key";
    DUPLICATE_NAME    = "DIM0105", Error,   "Duplicate sibling name";
    CHILD_BEFORE_PARENT = "DIM0106", Error, "Child declared before its parent";
    INSTANCE_CYCLE    = "DIM0107", Error,   "Instance cycle";
    MISSING_ROOT      = "DIM0108", Error,   "Scene declares a root that does not exist";
    BAD_ID_FORM       = "DIM0109", Error,   "Malformed node id";
    BAD_HEADER        = "DIM0110", Error,   "Missing or unsupported format header";
    MISSING_KEY       = "DIM0111", Error,   "A block is missing a key it must have";

    // 02xx — values
    TYPE_MISMATCH     = "DIM0201", Error,   "Property type mismatch";
    OUT_OF_RANGE      = "DIM0202", Error,   "Value outside the schema's range";
    NOT_REPRESENTABLE = "DIM0203", Error,   "Scalar not exactly representable in fixed-point";
    BAD_REFERENCE     = "DIM0204", Error,   "Malformed typed reference";
    BAD_CHUNK_DATA    = "DIM0205", Error,   "Malformed run-length tile data";

    // 03xx — schema
    UNKNOWN_PROPERTY  = "DIM0301", Error,   "Unknown property";
    ORPHANED_OVERRIDE = "DIM0302", Warning, "Override targets a node the source no longer has";
    MISSING_REQUIRED  = "DIM0303", Error,   "Required property is absent";

    // 04xx — command bus
    NO_SUCH_NODE      = "DIM0401", Error,   "Command names a node that does not exist";
    ILLEGAL_REPARENT  = "DIM0402", Error,   "Reparenting would create a cycle";
    NOTHING_TO_UNDO   = "DIM0403", Error,   "Undo stack is empty";
    COMMAND_REJECTED  = "DIM0404", Error,   "Command is not valid in this run mode";

    // 05xx — scripting
    SCRIPT_SYNTAX     = "DIM0501", Error,   "Lua syntax error";
    SCRIPT_RUNTIME    = "DIM0502", Error,   "Lua runtime error";
    SCRIPT_SANDBOX    = "DIM0503", Error,   "Script reached for something the sandbox withholds";
    STALE_HANDLE      = "DIM0504", Error,   "Script used a handle to a destroyed node";
    SCRIPT_BAD_ARGUMENT = "DIM0505", Error, "Script passed an argument the binding cannot accept";

    // 06xx — assets
    ASSET_MISSING     = "DIM0601", Error,   "Referenced asset is not in the project";
    IMPORT_FAILED     = "DIM0602", Error,   "Asset import failed";
    UNSUPPORTED_ASSET = "DIM0603", Error,   "Unsupported source format";

    // 07xx — determinism
    REPLAY_DIVERGED   = "DIM0701", Error,   "Replay state hash diverged from the recorded log";
    PROBE_FAILED      = "DIM0702", Error,   "Replay probe assertion failed";
    LOG_MISMATCH      = "DIM0703", Error,   "Input log was recorded against a different scene or engine version";

    // 08xx — tooling
    NOT_IMPLEMENTED   = "DIM0801", Error,   "The command is implemented but something this build needs is missing";
    BAD_ARGUMENT      = "DIM0802", Error,   "A command-line argument could not be parsed";

    // 09xx — project settings
    SETTINGS_UNREADABLE = "DIM0901", Error, "project.toml exists but could not be read or parsed";
    SETTINGS_INVALID    = "DIM0902", Error, "A project setting is out of range or the wrong shape";
    BINDING_UNKNOWN     = "DIM0903", Error, "An input binding names an action the engine does not have";
}

/// Look up a registered code.
pub fn code_info(code: Code) -> Option<&'static CodeInfo> {
    CODES.iter().find(|c| c.code == code)
}

/// Where a diagnostic came from.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Span {
    /// Project-relative file path.
    pub file: String,
    /// 1-based line, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    /// 1-based column, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
}

impl Span {
    /// A whole-file span.
    pub fn file(path: impl Into<String>) -> Span {
        Span {
            file: path.into(),
            line: None,
            column: None,
        }
    }

    /// A span at a specific line.
    pub fn at(path: impl Into<String>, line: u32) -> Span {
        Span {
            file: path.into(),
            line: Some(line),
            column: None,
        }
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
            if let Some(col) = self.column {
                write!(f, ":{col}")?;
            }
        }
        Ok(())
    }
}

/// One structured failure or warning.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Stable code.
    pub code: Code,
    /// How much it matters.
    pub severity: Severity,
    /// Human-readable summary. Never parse this; match on `code` instead.
    pub message: String,
    /// Where it happened.
    ///
    /// Boxed because a `Diagnostic` is returned by value from a great many
    /// fallible functions, and a span carries a path string. Boxing it keeps
    /// the whole type comfortably small enough that returning one in a `Result`
    /// costs nothing worth thinking about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<Box<Span>>,
    /// The node involved, as a scene path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_path: Option<String>,
    /// Machine-readable detail. Keys are stable per code.
    #[serde(skip_serializing_if = "serde_json::Map::is_empty", default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

impl Diagnostic {
    /// Start a diagnostic at the code's registered severity.
    pub fn new(code: Code, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: code_info(code).map_or(Severity::Error, |i| i.severity),
            code,
            message: message.into(),
            span: None,
            node_path: None,
            fields: serde_json::Map::new(),
        }
    }

    /// Attach a source location.
    pub fn with_span(mut self, span: Span) -> Diagnostic {
        self.span = Some(Box::new(span));
        self
    }

    /// The source location, if there is one.
    pub fn span(&self) -> Option<&Span> {
        self.span.as_deref()
    }

    /// Attach the scene path of the node involved.
    pub fn with_node(mut self, path: impl Into<String>) -> Diagnostic {
        self.node_path = Some(path.into());
        self
    }

    /// Attach a machine-readable field.
    pub fn with_field(mut self, key: &str, value: impl Into<serde_json::Value>) -> Diagnostic {
        self.fields.insert(key.to_string(), value.into());
        self
    }

    /// Override the severity.
    pub fn with_severity(mut self, severity: Severity) -> Diagnostic {
        self.severity = severity;
        self
    }

    /// True when this stops the operation.
    #[inline]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.severity, self.code)?;
        if let Some(span) = &self.span {
            write!(f, " {span}")?;
        }
        write!(f, ": {}", self.message)?;
        if let Some(path) = &self.node_path {
            write!(f, " (at {path})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostic {}

/// A batch of diagnostics.
///
/// Validation collects rather than bailing on the first problem: a scene with
/// six typos should report six typos, not make the author run the loader six
/// times.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl Diagnostics {
    /// An empty batch.
    pub fn new() -> Diagnostics {
        Diagnostics(Vec::new())
    }

    /// Add one.
    pub fn push(&mut self, d: Diagnostic) {
        self.0.push(d);
    }

    /// Merge another batch in.
    pub fn extend(&mut self, other: Diagnostics) {
        self.0.extend(other.0);
    }

    /// True when nothing was reported.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How many entries.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True when at least one entry is an error.
    pub fn has_errors(&self) -> bool {
        self.0.iter().any(Diagnostic::is_error)
    }

    /// Every entry.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter()
    }

    /// Just the errors.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.0.iter().filter(|d| d.is_error())
    }

    /// Turn an error-carrying batch into a `Result`, keeping warnings.
    pub fn into_result(self) -> Result<Diagnostics, Diagnostics> {
        if self.has_errors() {
            Err(self)
        } else {
            Ok(self)
        }
    }
}

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl FromIterator<Diagnostic> for Diagnostics {
    fn from_iter<I: IntoIterator<Item = Diagnostic>>(iter: I) -> Diagnostics {
        Diagnostics(iter.into_iter().collect())
    }
}
