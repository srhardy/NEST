//! The roost — where finished runs rest.
//!
//! Every run writes a receipt to `<home>/.nest/roost/<id>.json`. The
//! roost is the activity log: `nest roost` lists it, the shell's
//! Activity tab will render it.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::hatchling::Receipt;

/// Write a receipt into the roost; returns its path.
pub fn record(roost_dir: &Path, receipt: &Receipt) -> Result<PathBuf> {
    fs::create_dir_all(roost_dir)
        .with_context(|| format!("cannot create roost {}", roost_dir.display()))?;
    let path = roost_dir.join(format!("{}.json", receipt.id));
    let json = serde_json::to_string_pretty(receipt)?;
    fs::write(&path, json).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

/// All receipts in the roost, newest first.
pub fn receipts(roost_dir: &Path) -> Result<Vec<Receipt>> {
    if !roost_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(roost_dir)
        .with_context(|| format!("cannot read roost {}", roost_dir.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "json") {
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(receipt) = serde_json::from_str::<Receipt>(&text) {
                    out.push(receipt);
                }
            }
        }
    }
    out.sort_by_key(|r| std::cmp::Reverse(r.started_ms));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hatchling::Status;

    fn receipt(id: &str, started: u64) -> Receipt {
        Receipt {
            id: id.into(),
            agent: "bot".into(),
            perch: "echo".into(),
            task: "t".into(),
            status: Status::Done,
            steps: 0,
            exit_code: Some(0),
            started_ms: started,
            finished_ms: started + 1,
            hard_won: false,
            failure_kind: None,
            peeps_path: None,
            tail: vec![],
        }
    }

    #[test]
    fn records_and_lists_newest_first() {
        let tmp = tempfile::tempdir().unwrap();
        let roost = tmp.path().join("roost");
        record(&roost, &receipt("a", 100)).unwrap();
        record(&roost, &receipt("b", 200)).unwrap();
        let all = receipts(&roost).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "b");
        assert!(roost.join("a.json").is_file());
    }
}
