use std::path::PathBuf;

use serde_json::Value;

/// What a harness can express. Used to warn instead of silently dropping features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub instructions: bool,
    pub skills: bool,
    pub commands: bool,
    pub hooks: bool,
    pub mcp: bool,
    pub permissions: bool,
    pub env: bool,
}

impl Capabilities {
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        for (name, supported) in [
            ("instructions", self.instructions),
            ("skills", self.skills),
            ("commands", self.commands),
            ("hooks", self.hooks),
            ("mcp", self.mcp),
            ("permissions", self.permissions),
            ("env", self.env),
        ] {
            parts.push(format!("{name}:{}", if supported { "yes" } else { "no" }));
        }
        parts.join(" ")
    }
}

/// A single projection operation. Adapters produce these; the apply engine executes them.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Insert or replace a marked block inside a file, preserving everything else.
    WriteBlock {
        path: PathBuf,
        id: String,
        contents: String,
        why: String,
    },
    /// Surgically merge a JSON value at `pointer`, preserving unknown keys.
    MergeJson {
        path: PathBuf,
        pointer: Vec<String>,
        value: Value,
        why: String,
    },
    /// Surgically merge a TOML table at `pointer` (body without headers), preserving formatting.
    MergeToml {
        path: PathBuf,
        pointer: Vec<String>,
        table_toml: String,
        why: String,
    },
    /// Install a directory of files (a skill) under `root`/<name>/.
    WriteFiles {
        root: PathBuf,
        name: String,
        files: Vec<(String, String)>,
        why: String,
    },
    /// Append elements to a JSON array at `pointer` (creates the array if absent).
    /// Existing entries are never replaced or reordered.
    MergeJsonArrayAppend {
        path: PathBuf,
        pointer: Vec<String>,
        value: Value,
        why: String,
    },
    /// Remove array elements whose serialized form contains `marker`.
    RemoveJsonArrayItems {
        path: PathBuf,
        pointer: Vec<String>,
        marker: String,
        why: String,
    },
    /// Remove a directory tree (an uninstalled skill). Applied as a move into the backup store.
    DeleteDir { path: PathBuf, why: String },
}

impl Op {
    pub fn path(&self) -> &PathBuf {
        match self {
            Op::WriteBlock { path, .. }
            | Op::MergeJson { path, .. }
            | Op::MergeToml { path, .. }
            | Op::MergeJsonArrayAppend { path, .. }
            | Op::RemoveJsonArrayItems { path, .. }
            | Op::DeleteDir { path, .. } => path,
            Op::WriteFiles { root, .. } => root,
        }
    }

    pub fn why(&self) -> &str {
        match self {
            Op::WriteBlock { why, .. }
            | Op::MergeJson { why, .. }
            | Op::MergeToml { why, .. }
            | Op::WriteFiles { why, .. }
            | Op::MergeJsonArrayAppend { why, .. }
            | Op::RemoveJsonArrayItems { why, .. }
            | Op::DeleteDir { why, .. } => why,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Op::WriteBlock { .. } => "block",
            Op::MergeJson { .. } => "json",
            Op::MergeToml { .. } => "toml",
            Op::WriteFiles { .. } => "files",
            Op::MergeJsonArrayAppend { .. } => "json+",
            Op::RemoveJsonArrayItems { .. } => "json-",
            Op::DeleteDir { .. } => "rmdir",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub ops: Vec<Op>,
    pub warnings: Vec<String>,
}

impl Plan {
    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    pub fn extend(&mut self, other: Plan) {
        self.ops.extend(other.ops);
        self.warnings.extend(other.warnings);
    }
}
