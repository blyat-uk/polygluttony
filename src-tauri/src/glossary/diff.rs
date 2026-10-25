//! Old-vs-new glossary diff (O13). Pure port of `glossary/diff.py`. Counts are
//! precomputed fields because these types cross the IPC boundary.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::model::{Glossary, CATEGORIES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/types/generated/")]
pub enum DiffStatus {
    Added,
    Removed,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct TermDiff {
    pub source: String,
    pub old: Option<String>,
    pub new: Option<String>,
    pub status: DiffStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct CategoryDiff {
    /// Category key ("characters"), as used in `GlossaryDoc::terms`.
    pub key: String,
    /// Display label ("Characters", "Cultivation", …).
    pub name: String,
    pub terms: Vec<TermDiff>,
    pub added: u32,
    pub removed: u32,
    pub modified: u32,
    pub unchanged: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/types/generated/")]
pub struct GlossaryDiff {
    pub categories: Vec<CategoryDiff>,
    pub total_added: u32,
    pub total_removed: u32,
    pub total_modified: u32,
    pub has_changes: bool,
}

fn label(category: &str) -> String {
    let mut s = category.to_string();
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s
}

impl GlossaryDiff {
    /// `old = None` means a fresh build — everything is `Added`
    /// (`diff.py:120-161`). Categories empty on both sides are omitted.
    pub fn compute(old: Option<&Glossary>, new: &Glossary) -> GlossaryDiff {
        let mut categories = Vec::new();
        let (mut ta, mut tr, mut tm) = (0u32, 0u32, 0u32);
        for c in CATEGORIES {
            let old_terms = old.map(|g| g.category(c));
            let new_terms = new.category(c);
            // BTreeSet union keeps sources sorted (diff.py sorts explicitly).
            let mut sources: std::collections::BTreeSet<&String> = new_terms.keys().collect();
            if let Some(o) = old_terms {
                sources.extend(o.keys());
            }
            if sources.is_empty() {
                continue;
            }
            let mut terms = Vec::new();
            let (mut a, mut r, mut m, mut u) = (0u32, 0u32, 0u32, 0u32);
            for source in sources {
                let o = old_terms.and_then(|t| t.get(source)).cloned();
                let n = new_terms.get(source).cloned();
                let status = match (&o, &n) {
                    (None, _) => {
                        a += 1;
                        DiffStatus::Added
                    }
                    (_, None) => {
                        r += 1;
                        DiffStatus::Removed
                    }
                    (Some(ov), Some(nv)) if ov != nv => {
                        m += 1;
                        DiffStatus::Modified
                    }
                    _ => {
                        u += 1;
                        DiffStatus::Unchanged
                    }
                };
                terms.push(TermDiff { source: source.clone(), old: o, new: n, status });
            }
            ta += a;
            tr += r;
            tm += m;
            categories.push(CategoryDiff {
                key: c.to_string(),
                name: label(c),
                terms,
                added: a,
                removed: r,
                modified: m,
                unchanged: u,
            });
        }
        GlossaryDiff {
            categories,
            total_added: ta,
            total_removed: tr,
            total_modified: tm,
            has_changes: ta + tr + tm > 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::model::Glossary;

    #[test]
    fn compute_classifies_all_statuses() {
        let mut old = Glossary::new("xianxia");
        old.characters.insert("林动".into(), "Lin Dong".into()); // unchanged
        old.characters.insert("应欢欢".into(), "Ying HuanHuan".into()); // modified
        old.items.insert("祖符".into(), "Ancestral Symbol".into()); // removed
        let mut new = Glossary::new("xianxia");
        new.characters.insert("林动".into(), "Lin Dong".into());
        new.characters.insert("应欢欢".into(), "Ying Huanhuan".into());
        new.locations.insert("青阳镇".into(), "Qingyang Town".into()); // added

        let d = GlossaryDiff::compute(Some(&old), &new);
        assert!(d.has_changes);
        assert_eq!(d.total_added, 1);
        assert_eq!(d.total_removed, 1);
        assert_eq!(d.total_modified, 1);

        let chars = d.categories.iter().find(|c| c.name == "Characters").unwrap();
        assert_eq!(chars.modified, 1);
        assert_eq!(chars.unchanged, 1);
        let modified = chars.terms.iter().find(|t| t.source == "应欢欢").unwrap();
        assert_eq!(modified.status, DiffStatus::Modified);
        assert_eq!(modified.old.as_deref(), Some("Ying HuanHuan"));
        assert_eq!(modified.new.as_deref(), Some("Ying Huanhuan"));

        // Removed term appears with new=None.
        let items = d.categories.iter().find(|c| c.name == "Items").unwrap();
        assert_eq!(items.terms[0].status, DiffStatus::Removed);
        assert!(items.terms[0].new.is_none());
    }

    #[test]
    fn compute_with_no_old_marks_everything_added() {
        let mut new = Glossary::new("modern");
        new.characters.insert("a".into(), "A".into());
        let d = GlossaryDiff::compute(None, &new);
        assert_eq!(d.total_added, 1);
        assert!(d.has_changes);
        // Empty categories (no terms on either side) are omitted entirely.
        assert_eq!(d.categories.len(), 1);
    }

    #[test]
    fn identical_glossaries_have_no_changes() {
        let mut g = Glossary::new("modern");
        g.characters.insert("a".into(), "A".into());
        let d = GlossaryDiff::compute(Some(&g), &g.clone());
        assert!(!d.has_changes);
        assert_eq!(d.total_added + d.total_removed + d.total_modified, 0);
        assert_eq!(d.categories[0].unchanged, 1);
    }
}
