//! Scoped glossary passes. Tidy up and Look up names online can work on all
//! terms, one category, or a hand-picked set. Both passes take a glossary and
//! answer with one, so a scoped run is:
//! 1. [`resolve`]: cut the scoped sub-glossary out of the full one;
//! 2. run the pass on that sub-glossary only;
//! 3. [`overlay`]: lay the answer back over the full glossary.
//!
//! The overlay's promise is that terms outside the scope never change.
//! Inside the scope, the pass's answer wins: it may rewrite, drop (Tidy up
//! merges duplicates), move or add terms.
//!
//! Each source term must end up in exactly one category. `normalize_pass`
//! replaces whole categories and the lookup answers with a whole glossary, so
//! nothing stops a pass from putting one source in two categories, or next
//! to a copy that lives outside the scope. [`one_category_per_source`] and
//! the overlay's placement rules stop that.

use std::collections::BTreeMap;

use super::meta::GlossaryScope;
use super::model::{Glossary, CATEGORIES};
use crate::error::{AppError, AppResult};

/// The `&'static` category key for `key`, or a plain error for one the app
/// doesn't know (`Glossary::category` panics on those).
fn category_key(key: &str) -> AppResult<&'static str> {
    CATEGORIES
        .iter()
        .copied()
        .find(|c| *c == key)
        .ok_or_else(|| AppError::Other(format!("There's no glossary category called \"{key}\".")))
}

/// The part of `original` that `scope` covers (`None` = all terms). Terms
/// named in a `terms` scope that aren't in the glossary are skipped; the
/// result may be empty, and the caller says what that means for its pass.
pub fn resolve(original: &Glossary, scope: Option<&GlossaryScope>) -> AppResult<Glossary> {
    let mut sub = Glossary::new(&original.world_type);
    match scope {
        None | Some(GlossaryScope::All) => return Ok(original.clone()),
        Some(GlossaryScope::Category { category }) => {
            let c = category_key(category)?;
            *sub.category_mut(c) = original.category(c).clone();
        }
        Some(GlossaryScope::Terms { terms }) => {
            for t in terms {
                let c = category_key(&t.category)?;
                if let Some(v) = original.category(c).get(&t.source) {
                    sub.category_mut(c).insert(t.source.clone(), v.clone());
                }
            }
        }
    }
    Ok(sub)
}

/// Make every source appear in one category only. A source found in several
/// keeps the category `home` names for it when it's one of them (where the
/// term was before the pass), else the first in `CATEGORIES` order; the other
/// copies are dropped.
pub fn one_category_per_source(g: &mut Glossary, home: &BTreeMap<String, &'static str>) {
    let mut seen: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    for c in CATEGORIES {
        for source in g.category(c).keys() {
            seen.entry(source.clone()).or_default().push(c);
        }
    }
    for (source, cats) in seen.into_iter().filter(|(_, cats)| cats.len() > 1) {
        let keep = home.get(&source).copied().filter(|h| cats.contains(h)).unwrap_or(cats[0]);
        for c in cats.into_iter().filter(|c| *c != keep) {
            g.category_mut(c).remove(&source);
        }
    }
}

/// `original` with the scoped terms (`sub`) replaced by the pass's answer
/// (`result`). Outside the scope nothing changes:
/// - a source the pass wasn't given, but that lives outside the scope, is
///   ignored (the pass may not add a second copy or overwrite that term);
/// - a scoped term the pass moved onto a category where an out-of-scope term
///   has the same source stays in its own category, with the pass's value.
///
/// A scoped term that ALREADY shared its source with an out-of-scope term
/// keeps its place: that duplicate is the user's, not the pass's to resolve.
pub fn overlay(original: &Glossary, sub: &Glossary, result: &Glossary) -> Glossary {
    let mut out = original.clone();
    let mut home: BTreeMap<String, &'static str> = BTreeMap::new();
    for c in CATEGORIES {
        for source in sub.category(c).keys() {
            out.category_mut(c).remove(source);
            home.entry(source.clone()).or_insert(c);
        }
    }

    let mut placed = result.clone();
    one_category_per_source(&mut placed, &home);
    for c in CATEGORIES {
        for (source, value) in placed.category(c) {
            let target = match home.get(source) {
                // Given to the pass: its answer lands where the pass put it,
                // unless that slot belongs to an out-of-scope term.
                Some(&h) if out.category(c).contains_key(source) => h,
                Some(_) => c,
                // New from the pass: only when no term outside has it.
                None if out.has_key(source) => continue,
                None => c,
            };
            out.category_mut(target).insert(source.clone(), value.clone());
        }
    }
    out.world_type = original.world_type.clone();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glossary::meta::TermRef;

    fn g(terms: &[(&str, &str, &str)]) -> Glossary {
        let mut g = Glossary::new("xianxia");
        for (c, s, t) in terms {
            g.category_mut(c).insert(s.to_string(), t.to_string());
        }
        g
    }

    fn original() -> Glossary {
        g(&[
            ("characters", "林动", "lin dong"),
            ("characters", "应欢欢", "ying huanhuan"),
            ("locations", "青阳镇", "qingyang town"),
            ("skills", "大荒囚天指", "Great Desolate Finger"),
        ])
    }

    fn term(category: &str, source: &str) -> TermRef {
        TermRef { category: category.into(), source: source.into() }
    }

    /// Every term outside the scope, compared value by value.
    fn assert_untouched(before: &Glossary, after: &Glossary, sub: &Glossary) {
        for c in CATEGORIES {
            for (s, v) in before.category(c) {
                if !sub.category(c).contains_key(s) {
                    assert_eq!(after.category(c).get(s), Some(v), "{c}/{s} is out of scope");
                }
            }
        }
    }

    // ── resolve ────────────────────────────────────────────────────────────

    #[test]
    fn resolve_all_is_the_whole_glossary() {
        let o = original();
        assert_eq!(resolve(&o, None).unwrap(), o);
        assert_eq!(resolve(&o, Some(&GlossaryScope::All)).unwrap(), o);
    }

    #[test]
    fn resolve_category_takes_just_that_category() {
        let o = original();
        let sub =
            resolve(&o, Some(&GlossaryScope::Category { category: "characters".into() })).unwrap();
        assert_eq!(sub.count(), 2);
        assert_eq!(sub.characters, o.characters);
        assert_eq!(sub.world_type, "xianxia");
    }

    #[test]
    fn resolve_terms_groups_by_category_and_skips_missing() {
        let o = original();
        let scope = GlossaryScope::Terms {
            terms: vec![
                term("characters", "林动"),
                term("locations", "青阳镇"),
                term("characters", "不存在"),  // not in the glossary
                term("locations", "林动"),     // exists, but not in this category
            ],
        };
        let sub = resolve(&o, Some(&scope)).unwrap();
        assert_eq!(sub.count(), 2);
        assert_eq!(sub.characters.get("林动").unwrap(), "lin dong");
        assert_eq!(sub.locations.get("青阳镇").unwrap(), "qingyang town");
    }

    #[test]
    fn resolve_rejects_unknown_categories_in_plain_words() {
        let o = original();
        let err = resolve(&o, Some(&GlossaryScope::Category { category: "spells".into() }))
            .unwrap_err()
            .to_string();
        assert_eq!(err, "There's no glossary category called \"spells\".");
        let terms = GlossaryScope::Terms { terms: vec![term("Characters", "林动")] };
        assert!(resolve(&o, Some(&terms)).is_err());
    }

    // ── one_category_per_source ─────────────────────────────────────────────

    #[test]
    fn duplicate_source_keeps_its_home_category() {
        let mut x = g(&[("characters", "星", "Star"), ("locations", "星", "Star Peak")]);
        let home = BTreeMap::from([("星".to_string(), "locations")]);
        one_category_per_source(&mut x, &home);
        assert_eq!(x.count(), 1);
        assert_eq!(x.locations.get("星").unwrap(), "Star Peak");
    }

    #[test]
    fn duplicate_source_without_home_keeps_the_first_category() {
        let mut x = g(&[
            ("items", "祖符", "Symbol"),
            ("skills", "祖符", "Symbol Art"),
            ("organizations", "祖符", "Symbol Hall"),
        ]);
        // A home the pass didn't keep the term in doesn't count.
        let home = BTreeMap::from([("祖符".to_string(), "characters")]);
        one_category_per_source(&mut x, &home);
        assert_eq!(x.count(), 1);
        assert_eq!(x.skills.get("祖符").unwrap(), "Symbol Art", "skills comes before items");
    }

    #[test]
    fn unique_sources_are_left_alone() {
        let mut x = original();
        one_category_per_source(&mut x, &BTreeMap::new());
        assert_eq!(x, original());
    }

    // ── overlay ─────────────────────────────────────────────────────────────

    #[test]
    fn category_overlay_changes_only_that_category() {
        let o = original();
        let sub = resolve(&o, Some(&GlossaryScope::Category { category: "characters".into() }))
            .unwrap();
        // Tidy up rewrote one name and merged the other away.
        let result = g(&[("characters", "林动", "Lin Dong")]);
        let out = overlay(&o, &sub, &result);
        assert_untouched(&o, &out, &sub);
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong");
        assert!(!out.characters.contains_key("应欢欢"), "the pass's removal stands");
        assert_eq!(out.count(), 3);
    }

    #[test]
    fn terms_overlay_leaves_unlisted_terms_of_the_same_category() {
        let o = original();
        let sub =
            resolve(&o, Some(&GlossaryScope::Terms { terms: vec![term("characters", "林动")] }))
                .unwrap();
        let result = g(&[("characters", "林动", "Lin Dong")]);
        let out = overlay(&o, &sub, &result);
        assert_untouched(&o, &out, &sub);
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong");
        assert_eq!(out.characters.get("应欢欢").unwrap(), "ying huanhuan");
    }

    #[test]
    fn overlay_ignores_answers_about_out_of_scope_terms() {
        let o = original();
        let sub =
            resolve(&o, Some(&GlossaryScope::Terms { terms: vec![term("characters", "林动")] }))
                .unwrap();
        // The lookup also "fixed" terms it wasn't asked about, one of them
        // in another category.
        let result = g(&[
            ("characters", "林动", "Lin Dong"),
            ("characters", "应欢欢", "WRONG"),
            ("items", "青阳镇", "WRONG"),
        ]);
        let out = overlay(&o, &sub, &result);
        assert_untouched(&o, &out, &sub);
        assert!(!out.items.contains_key("青阳镇"), "no second copy of an out-of-scope source");
        assert_eq!(out.count(), 4);
    }

    #[test]
    fn overlay_keeps_new_terms_and_moves() {
        let o = original();
        let sub = resolve(&o, Some(&GlossaryScope::Category { category: "characters".into() }))
            .unwrap();
        let result = g(&[
            ("characters", "林动", "Lin Dong"),
            ("organizations", "应欢欢", "Ying Huanhuan"), // moved
            ("characters", "小貂", "Little Marten"),     // new
        ]);
        let out = overlay(&o, &sub, &result);
        assert_untouched(&o, &out, &sub);
        assert!(!out.characters.contains_key("应欢欢"));
        assert_eq!(out.organizations.get("应欢欢").unwrap(), "Ying Huanhuan");
        assert_eq!(out.characters.get("小貂").unwrap(), "Little Marten");
    }

    /// The pass answered with one scoped source in two categories: it lands
    /// once, in the category it came from.
    #[test]
    fn overlay_puts_each_source_in_one_category() {
        let o = original();
        let sub = resolve(&o, Some(&GlossaryScope::All)).unwrap();
        let mut result = o.clone();
        result.items.insert("林动".into(), "Lin Dong (item?)".into());
        result.characters.insert("林动".into(), "Lin Dong".into());
        let out = overlay(&o, &sub, &result);
        assert_eq!(out.characters.get("林动").unwrap(), "Lin Dong");
        assert!(!out.items.contains_key("林动"));
        let copies = CATEGORIES.iter().filter(|c| out.category(c).contains_key("林动")).count();
        assert_eq!(copies, 1);
    }

    /// A scoped term moved onto a slot an out-of-scope term holds stays home.
    #[test]
    fn overlay_never_overwrites_an_out_of_scope_slot() {
        let o = g(&[("characters", "星", "Xing"), ("locations", "星", "Star Peak")]);
        let sub =
            resolve(&o, Some(&GlossaryScope::Terms { terms: vec![term("characters", "星")] }))
                .unwrap();
        let result = g(&[("locations", "星", "Star")]);
        let out = overlay(&o, &sub, &result);
        assert_eq!(out.locations.get("星").unwrap(), "Star Peak", "out-of-scope slot untouched");
        assert_eq!(out.characters.get("星").unwrap(), "Star", "kept home with the new value");
    }

    /// A duplicate the user already had isn't the pass's to resolve: the
    /// scoped copy is updated, not deleted.
    #[test]
    fn overlay_keeps_a_scoped_term_that_already_had_an_outside_twin() {
        let o = g(&[("characters", "星", "Xing"), ("locations", "星", "Star Peak")]);
        let sub =
            resolve(&o, Some(&GlossaryScope::Category { category: "characters".into() })).unwrap();
        let result = g(&[("characters", "星", "Xing (MC)")]);
        let out = overlay(&o, &sub, &result);
        assert_eq!(out.characters.get("星").unwrap(), "Xing (MC)");
        assert_eq!(out.locations.get("星").unwrap(), "Star Peak");
    }

    #[test]
    fn overlay_with_an_unchanged_answer_changes_nothing() {
        let o = original();
        let sub = resolve(&o, Some(&GlossaryScope::Category { category: "skills".into() })).unwrap();
        assert_eq!(overlay(&o, &sub, &sub), o);
        let all = resolve(&o, None).unwrap();
        assert_eq!(overlay(&o, &all, &all), o);
    }
}
