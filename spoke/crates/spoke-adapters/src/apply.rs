use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, Table};

use spoke_core::ir::{Op, Plan};
use spoke_core::paths;

/// One planned file effect, after comparing against what is on disk.
#[derive(Debug, Clone)]
pub struct Change {
    pub path: PathBuf,
    pub kind: &'static str,
    pub why: String,
    pub changed: bool,
}

const BLOCK_BEGIN: &str = "<!-- spoke:begin";
const BLOCK_END: &str = "<!-- spoke:end";

fn render_block(id: &str, contents: &str) -> String {
    format!(
        "{} {} -->\n{}\n{} {} -->",
        BLOCK_BEGIN,
        id,
        contents.trim_end(),
        BLOCK_END,
        id
    )
}

fn splice_block(existing: &str, id: &str, contents: &str) -> String {
    let begin = format!("{} {} -->", BLOCK_BEGIN, id);
    let end_marker = format!("{} {} -->", BLOCK_END, id);
    let block = render_block(id, contents);

    if let Some(begin_at) = existing.find(&begin) {
        if let Some(end_rel) = existing[begin_at..].find(&end_marker) {
            let end_at = begin_at + end_rel + end_marker.len();
            return format!("{}{}{}", &existing[..begin_at], block, &existing[end_at..]);
        }
    }

    if existing.trim().is_empty() {
        return format!("{block}\n");
    }
    let mut out = existing.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(&block);
    out.push('\n');
    out
}

fn insert_json(object: &mut Map<String, Value>, path: &[String], value: Value) {
    if path.len() == 1 {
        object.insert(path[0].clone(), value);
        return;
    }
    let entry = object
        .entry(path[0].clone())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    insert_json(
        entry.as_object_mut().expect("just ensured object"),
        &path[1..],
        value,
    );
}

fn merge_json(existing: &str, pointer: &[String], value: &Value) -> Result<String> {
    if pointer.is_empty() {
        bail!("empty JSON pointer");
    }
    let mut root: Value = if existing.trim().is_empty() {
        Value::Object(Map::new())
    } else {
        serde_json::from_str(existing).context("parsing existing JSON")?
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("existing JSON root is not an object"))?;
    insert_json(object, pointer, value.clone());
    let mut out = serde_json::to_string_pretty(&root).context("serializing merged JSON")?;
    out.push('\n');
    Ok(out)
}

fn insert_toml(table: &mut Table, path: &[String], item: Item) {
    if path.len() == 1 {
        table.insert(&path[0], item);
        return;
    }
    if !table.contains_key(&path[0]) || !table[&path[0]].is_table() {
        table.insert(&path[0], Item::Table(Table::new()));
    }
    let inner = table
        .get_mut(&path[0])
        .expect("just inserted")
        .as_table_mut()
        .expect("just ensured table");
    insert_toml(inner, &path[1..], item);
}

fn merge_toml(existing: &str, pointer: &[String], table_toml: &str) -> Result<String> {
    if pointer.is_empty() {
        bail!("empty TOML pointer");
    }
    let mut doc: DocumentMut = if existing.trim().is_empty() {
        DocumentMut::new()
    } else {
        existing.parse().context("parsing existing TOML")?
    };

    let snippet: DocumentMut = format!("[__spoke__]\n{}\n", table_toml.trim_end())
        .parse()
        .context("parsing generated TOML table")?;
    let item = snippet
        .as_table()
        .get("__spoke__")
        .cloned()
        .ok_or_else(|| anyhow!("generated TOML table missing"))?;

    insert_toml(doc.as_table_mut(), pointer, item);
    Ok(doc.to_string())
}

fn parse_json_or_object(existing: &str) -> Result<Value> {
    if existing.trim().is_empty() {
        Ok(Value::Object(Map::new()))
    } else {
        serde_json::from_str(existing).context("parsing existing JSON")
    }
}

/// Navigate to the array at `pointer`, creating objects/arrays on the way.
fn json_array_at<'a>(root: &'a mut Value, pointer: &[String]) -> Result<&'a mut Vec<Value>> {
    let mut current = root;
    for key in &pointer[..pointer.len().saturating_sub(1)] {
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        current = current
            .as_object_mut()
            .expect("just ensured object")
            .entry(key.clone())
            .or_insert_with(|| Value::Object(Map::new()));
    }
    let last = pointer
        .last()
        .ok_or_else(|| anyhow!("empty JSON pointer"))?;
    if !current.is_object() {
        *current = Value::Object(Map::new());
    }
    let slot = current
        .as_object_mut()
        .expect("just ensured object")
        .entry(last.clone())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    Ok(slot.as_array_mut().expect("just ensured array"))
}

fn merge_json_array_append(existing: &str, pointer: &[String], value: &Value) -> Result<String> {
    let mut root = parse_json_or_object(existing)?;
    let items = value
        .as_array()
        .ok_or_else(|| anyhow!("MergeJsonArrayAppend value must be an array"))?;
    {
        let array = json_array_at(&mut root, pointer)?;
        for item in items {
            if !array.contains(item) {
                array.push(item.clone());
            }
        }
    }
    let mut out = serde_json::to_string_pretty(&root).context("serializing JSON")?;
    out.push('\n');
    Ok(out)
}

fn remove_json_array_items(existing: &str, pointer: &[String], marker: &str) -> Result<String> {
    if existing.trim().is_empty() {
        return Ok(existing.to_string());
    }
    let mut root = parse_json_or_object(existing)?;
    // Navigate without creating anything; a missing path means nothing to remove.
    let mut current = &mut root;
    for key in pointer {
        let Some(next) = current.get_mut(key) else {
            return Ok(existing.to_string());
        };
        current = next;
    }
    if let Some(array) = current.as_array_mut() {
        array.retain(|item| {
            !serde_json::to_string(item)
                .unwrap_or_default()
                .contains(marker)
        });
    }
    let mut out = serde_json::to_string_pretty(&root).context("serializing JSON")?;
    out.push('\n');
    Ok(out)
}

fn read_or_empty(path: &Path) -> Result<String> {
    if path.exists() {
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
    } else {
        Ok(String::new())
    }
}

/// Every file a write-like op would produce, with its full intended contents.
fn desired_files(op: &Op) -> Result<Vec<(PathBuf, String)>> {
    match op {
        Op::WriteBlock {
            path,
            id,
            contents,
            ..
        } => {
            let existing = read_or_empty(path)?;
            Ok(vec![(path.clone(), splice_block(&existing, id, contents))])
        }
        Op::MergeJson {
            path,
            pointer,
            value,
            ..
        } => {
            let existing = read_or_empty(path)?;
            Ok(vec![(path.clone(), merge_json(&existing, pointer, value)?)])
        }
        Op::MergeToml {
            path,
            pointer,
            table_toml,
            ..
        } => {
            let existing = read_or_empty(path)?;
            Ok(vec![(path.clone(), merge_toml(&existing, pointer, table_toml)?)])
        }
        Op::WriteFiles {
            root, name, files, ..
        } => {
            let dir = root.join(name);
            Ok(files
                .iter()
                .map(|(rel, body)| (dir.join(rel), body.clone()))
                .collect())
        }
        Op::MergeJsonArrayAppend {
            path,
            pointer,
            value,
            ..
        } => {
            let existing = read_or_empty(path)?;
            Ok(vec![(
                path.clone(),
                merge_json_array_append(&existing, pointer, value)?,
            )])
        }
        Op::RemoveJsonArrayItems {
            path,
            pointer,
            marker,
            ..
        } => {
            let existing = read_or_empty(path)?;
            Ok(vec![(
                path.clone(),
                remove_json_array_items(&existing, pointer, marker)?,
            )])
        }
        Op::DeleteDir { .. } => Ok(Vec::new()),
    }
}

/// The path an op is reported against (the skill directory for WriteFiles).
fn report_path(op: &Op) -> PathBuf {
    match op {
        Op::WriteFiles { root, name, .. } => root.join(name),
        _ => op.path().clone(),
    }
}

fn would_change(op: &Op) -> Result<bool> {
    match op {
        Op::DeleteDir { path, .. } => Ok(path.exists()),
        _ => {
            for (path, contents) in desired_files(op)? {
                if read_or_empty(&path)? != contents {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

/// Report what would change, writing nothing.
pub fn preview(plan: &Plan) -> Result<Vec<Change>> {
    let mut changes = Vec::new();
    for op in &plan.ops {
        changes.push(Change {
            path: report_path(op),
            kind: op.kind(),
            why: op.why().to_string(),
            changed: would_change(op)?,
        });
    }
    Ok(changes)
}

/// Apply a plan: back up every file it will touch, then write the merged result.
/// `base` is the project directory, used only to keep backup filenames readable.
pub fn apply(plan: &Plan, home: &Path, base: &Path, unix_secs: u64) -> Result<Vec<Change>> {
    let backup_root = paths::backups_dir(home, unix_secs);
    let mut changes = Vec::new();

    for op in &plan.ops {
        // Deletions are backed up by moving the tree into the backup store.
        if let Op::DeleteDir { path, why } = op {
            if path.exists() {
                fs::create_dir_all(&backup_root)
                    .with_context(|| format!("creating {}", backup_root.display()))?;
                let backup = backup_root.join(paths::backup_file_name(path, base));
                if backup.exists() {
                    remove_path(&backup)?;
                }
                if let Some(parent) = backup.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::rename(path, &backup).with_context(|| {
                    format!("moving {} into {}", path.display(), backup.display())
                })?;
                changes.push(Change {
                    path: path.clone(),
                    kind: op.kind(),
                    why: why.clone(),
                    changed: true,
                });
            } else {
                changes.push(Change {
                    path: path.clone(),
                    kind: op.kind(),
                    why: why.clone(),
                    changed: false,
                });
            }
            continue;
        }

        let mut changed_any = false;
        for (path, contents) in desired_files(op)? {
            if read_or_empty(&path)? == contents {
                continue;
            }
            if path.exists() {
                fs::create_dir_all(&backup_root)
                    .with_context(|| format!("creating {}", backup_root.display()))?;
                let backup = backup_root.join(paths::backup_file_name(&path, base));
                if let Some(parent) = backup.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&path, &backup)
                    .with_context(|| format!("backing up {} -> {}", path.display(), backup.display()))?;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&path, contents).with_context(|| format!("writing {}", path.display()))?;
            changed_any = true;
        }

        changes.push(Change {
            path: report_path(op),
            kind: op.kind(),
            why: op.why().to_string(),
            changed: changed_any,
        });
    }

    Ok(changes)
}

fn remove_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path).with_context(|| format!("removing {}", path.display()))
    } else {
        fs::remove_file(path).with_context(|| format!("removing {}", path.display()))
    }
}
