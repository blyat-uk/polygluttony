//! Which lines of an episode go to the model, and which are written as they
//! are. Typeset releases carry thousands of lines no translation needs:
//! karaoke effects split into syllables, vector drawings, lines with no words,
//! and the same sign repeated frame by frame (often once per layer too).
//!
//! Every line keeps its id (1-based index, the pipeline's): stored results,
//! Resume and Review all key on it. A line is one of
//!
//! - **Translate**: sent to the model.
//! - **Repeat** of a Translate line: the same words in the same style and by
//!   the same speaker, on screen without a break, and either typeset (placed
//!   with `\pos`, `\move`, `\org` or a clip: a sign tracked frame by frame)
//!   or starting at the same moment (a layered copy, such as a border). It
//!   gets that line's translation with its own tags (each frame has its own
//!   `\pos`). Spoken lines that say the same thing, even back to back ("No!"
//!   "No!"), are never repeats: each is sent with its own context.
//! - **Keep**: written as it is, never sent: no words, a drawing, a karaoke
//!   effect (Effect `fx`: generated syllables), or a style the folder keeps
//!   as it is. A whole lyric line with `\k` timing is translated like any
//!   other line; its style decides.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use regex::Regex;

use crate::ass::parse::DialogueLine;
use crate::ass::tags::{reapply, strip_for_text, strip_positional};

/// Two lines with the same words run on when the second starts at most this
/// long after the first ends (a frame at 24 fps is ~4 cs; tracked signs often
/// leave a frame's gap).
pub const RUN_GAP_CS: i64 = 10;

/// `\p1` and up: the line is a vector drawing (`\p0` ends one).
static DRAWING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{[^}]*\\p[1-9]").unwrap());

/// Placed or masked by hand: the tags a tracked sign carries on every frame.
static TYPESET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{[^}]*\\(?:pos|move|org|i?clip)\(").unwrap());

/// Why a line is written as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// Nothing to read once the tags are gone.
    NoText,
    /// A vector drawing (`\p1`).
    Drawing,
    /// A karaoke effect: a syllable the templater generated (Effect `fx`).
    Song,
    /// Its style is one the folder keeps as it is.
    Style,
}

/// What happens to one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Translate,
    /// The same sign as Translate line `of`; gets its translation.
    Repeat {
        of: u32,
    },
    Keep(Kept),
}

/// Why a line is kept, without the folder's style choice: `None` for a line
/// with words for the model.
pub fn always_kept(line: &DialogueLine) -> Option<Kept> {
    if DRAWING.is_match(&line.text) {
        return Some(Kept::Drawing);
    }
    if line.effect.trim().eq_ignore_ascii_case("fx") {
        return Some(Kept::Song);
    }
    let words = strip_for_text(&line.text)
        .replace("\\N", " ")
        .replace("\\n", " ")
        .replace("\\h", " ");
    if words.trim().is_empty() {
        return Some(Kept::NoText);
    }
    None
}

/// One sign on screen: its first line, when it started and ends so far, and
/// whether that first line is typeset.
struct Run {
    first: u32,
    start: i64,
    end: i64,
    typeset: bool,
}

/// The plan of one episode: a role per line, by id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinePlan {
    roles: Vec<Role>,
}

impl LinePlan {
    /// Plan `lines` with the folder's kept styles (style names as the file
    /// spells them; surrounding spaces don't count).
    pub fn new(lines: &[DialogueLine], kept_styles: &BTreeSet<String>) -> Self {
        let kept: BTreeSet<&str> = kept_styles.iter().map(|s| s.trim()).collect();
        let mut roles: Vec<Role> = lines
            .iter()
            .map(|d| match always_kept(d) {
                Some(k) => Role::Keep(k),
                None if kept.contains(d.style.trim()) => Role::Keep(Kept::Style),
                None => Role::Translate,
            })
            .collect();

        // Repeats: in time order, per (style, speaker, words), a line that
        // starts while the run is on screen (or within RUN_GAP_CS of its end)
        // joins it when it starts with the run's first line, or when both are
        // typeset; otherwise it starts a run of its own.
        let mut order: Vec<usize> = (0..lines.len())
            .filter(|i| roles[*i] == Role::Translate)
            .collect();
        order.sort_by_key(|i| (lines[*i].start_cs, *i));
        let mut runs: HashMap<(&str, &str, String), Run> = HashMap::new();
        for i in order {
            let d = &lines[i];
            let typeset = TYPESET.is_match(&d.text);
            let key = (
                d.style.trim(),
                d.name.trim(),
                strip_positional(&d.text).stripped,
            );
            match runs.get_mut(&key) {
                Some(run)
                    if d.start_cs <= run.end + RUN_GAP_CS
                        && (d.start_cs == run.start || (typeset && run.typeset)) =>
                {
                    roles[i] = Role::Repeat { of: run.first };
                    run.end = run.end.max(d.end_cs);
                }
                _ => {
                    let run = Run {
                        first: i as u32 + 1,
                        start: d.start_cs,
                        end: d.end_cs,
                        typeset,
                    };
                    runs.insert(key, run);
                }
            }
        }
        Self { roles }
    }

    /// The role of line `id` (1-based). Out of range reads as kept.
    pub fn role(&self, id: u32) -> Role {
        id.checked_sub(1)
            .and_then(|i| self.roles.get(i as usize))
            .copied()
            .unwrap_or(Role::Keep(Kept::NoText))
    }

    pub fn translates(&self, id: u32) -> bool {
        self.role(id) == Role::Translate
    }

    /// The ids sent to the model, in file order.
    pub fn translate_ids(&self) -> Vec<u32> {
        self.ids(|r| r == Role::Translate)
    }

    /// How many lines go to the model.
    pub fn translate_count(&self) -> usize {
        self.roles.iter().filter(|r| **r == Role::Translate).count()
    }

    /// Lines that get another line's translation.
    pub fn repeat_count(&self) -> usize {
        self.roles
            .iter()
            .filter(|r| matches!(r, Role::Repeat { .. }))
            .count()
    }

    /// Lines written as they are.
    pub fn kept_count(&self) -> usize {
        self.roles
            .iter()
            .filter(|r| matches!(r, Role::Keep(_)))
            .count()
    }

    fn ids(&self, keep: impl Fn(Role) -> bool) -> Vec<u32> {
        self.roles
            .iter()
            .enumerate()
            .filter(|(_, r)| keep(**r))
            .map(|(i, _)| i as u32 + 1)
            .collect()
    }

    /// Only the translations of lines that go to the model: stored ones from
    /// another plan (an older version, other kept styles) for a repeat or a
    /// kept line don't apply.
    pub fn own(&self, translations: &BTreeMap<u32, String>) -> BTreeMap<u32, String> {
        translations
            .iter()
            .filter(|(id, _)| self.translates(**id))
            .map(|(id, t)| (*id, t.clone()))
            .collect()
    }

    /// The lines of the translated file: a translated line with its
    /// translation, a repeat with its run's translation and its own tags, a
    /// kept line as it is. A line whose translation is missing (and its
    /// repeats) stays out, as the pipeline always did.
    pub fn output_lines(
        &self,
        lines: &[DialogueLine],
        translations: &BTreeMap<u32, String>,
    ) -> Vec<DialogueLine> {
        lines
            .iter()
            .enumerate()
            .filter_map(|(idx, d)| {
                let text = match self.role(idx as u32 + 1) {
                    Role::Translate => translations.get(&(idx as u32 + 1))?.clone(),
                    Role::Repeat { of } => {
                        let words = strip_positional(translations.get(&of)?).stripped;
                        reapply(&strip_positional(&d.text), &words)
                    }
                    Role::Keep(_) => d.text.clone(),
                };
                Some(DialogueLine { text, ..d.clone() })
            })
            .collect()
    }
}

/// What one file holds per style, for the folder's "Lines to translate".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileTally {
    /// Style → its lines in the file.
    pub styles: BTreeMap<String, StyleTally>,
    /// Karaoke lines, always kept.
    pub songs: u32,
    /// Lines with no words, and drawings, always kept.
    pub no_text: u32,
}

/// One style's lines in a file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleTally {
    /// Lines to translate with the style ticked.
    pub translate: u32,
    /// Every dialogue line in it.
    pub all: u32,
    /// Its first line to translate, tags stripped; empty when it has none.
    pub sample: String,
}

/// Count a file's lines per style as if every style were ticked: a style's
/// repeats never depend on another style, so the lines to translate with any
/// styles kept are the sum over the others.
pub fn tally(lines: &[DialogueLine]) -> FileTally {
    let plan = LinePlan::new(lines, &BTreeSet::new());
    let mut out = FileTally::default();
    for (idx, d) in lines.iter().enumerate() {
        let style = out.styles.entry(d.style.trim().to_string()).or_default();
        style.all += 1;
        match plan.role(idx as u32 + 1) {
            Role::Translate => {
                style.translate += 1;
                if style.sample.is_empty() {
                    style.sample = strip_for_text(&d.text)
                        .replace("\\N", " ")
                        .trim()
                        .to_string();
                }
            }
            Role::Repeat { .. } | Role::Keep(Kept::Style) => {}
            Role::Keep(Kept::Song) => out.songs += 1,
            Role::Keep(Kept::NoText | Kept::Drawing) => out.no_text += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(
        start: i64,
        end: i64,
        style: &str,
        name: &str,
        effect: &str,
        text: &str,
    ) -> DialogueLine {
        DialogueLine {
            layer: 0,
            start_cs: start,
            end_cs: end,
            style: style.into(),
            name: name.into(),
            margin_l: 0,
            margin_r: 0,
            margin_v: 0,
            effect: effect.into(),
            text: text.into(),
        }
    }

    fn sign(start: i64, x: f32, text: &str) -> DialogueLine {
        line(
            start,
            start + 4,
            "Signs",
            "",
            "",
            &format!("{{=8}}{{\\an6\\pos({x},139)}}{text}"),
        )
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn a_tracked_sign_is_translated_once() {
        // Six frames of one sign, as Cifer's Frieren ep 1 has them.
        let lines: Vec<DialogueLine> = (0..6)
            .map(|f| {
                sign(
                    9002 + f * 4,
                    1500.0 + f as f32 * 0.1,
                    "At the far northern edge of the continent,",
                )
            })
            .collect();
        let plan = LinePlan::new(&lines, &none());
        assert_eq!(plan.translate_ids(), vec![1]);
        assert_eq!(plan.repeat_count(), 5);
        for id in 2..=6 {
            assert_eq!(plan.role(id), Role::Repeat { of: 1 });
        }
    }

    #[test]
    fn layered_copies_of_a_sign_are_repeats() {
        // A border layer and a text layer, same time, same words.
        let mut under = sign(100, 10.0, "Holy City of Strahl");
        under.layer = 1;
        let over = sign(100, 10.0, "Holy City of Strahl");
        let plan = LinePlan::new(&[under, over], &none());
        assert_eq!(plan.translate_ids(), vec![1]);
        assert_eq!(plan.role(2), Role::Repeat { of: 1 });
    }

    #[test]
    fn the_same_sign_later_on_is_translated_again() {
        let lines = vec![sign(100, 1.0, "Episode 2"), sign(5000, 1.0, "Episode 2")];
        assert_eq!(LinePlan::new(&lines, &none()).translate_ids(), vec![1, 2]);
    }

    #[test]
    fn a_short_gap_still_runs_on_but_a_longer_one_does_not() {
        let near = vec![
            sign(100, 1.0, "Sign"),
            sign(100 + 4 + RUN_GAP_CS, 1.0, "Sign"),
        ];
        assert_eq!(LinePlan::new(&near, &none()).translate_ids(), vec![1]);
        let far = vec![
            sign(100, 1.0, "Sign"),
            sign(100 + 4 + RUN_GAP_CS + 1, 1.0, "Sign"),
        ];
        assert_eq!(LinePlan::new(&far, &none()).translate_ids(), vec![1, 2]);
    }

    #[test]
    fn spoken_lines_that_say_the_same_thing_are_each_translated() {
        let lines = vec![
            line(100, 200, "Default", "Fern", "", "I see."),
            line(900, 1000, "Default", "Stark", "", "I see."),
            line(2000, 2100, "Default", "Fern", "", "I see."),
        ];
        assert_eq!(
            LinePlan::new(&lines, &none()).translate_ids(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn back_to_back_lines_by_two_speakers_are_not_repeats() {
        let lines = vec![
            line(100, 200, "Default", "Fern", "", "No."),
            line(200, 300, "Default", "Stark", "", "No."),
        ];
        assert_eq!(LinePlan::new(&lines, &none()).translate_ids(), vec![1, 2]);
    }

    #[test]
    fn back_to_back_spoken_lines_with_no_speaker_are_not_repeats() {
        // Most fansubs leave the actor field empty.
        let lines = vec![
            line(100, 180, "Default", "", "", "No!"),
            line(180, 250, "Default", "", "", "{\\i1}No!{\\i0}"),
            line(250, 300, "Default", "", "", "{\\an8}No!"),
        ];
        assert_eq!(
            LinePlan::new(&lines, &none()).translate_ids(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn a_layered_copy_of_a_spoken_line_is_a_repeat() {
        // The same line twice at the same moment: a border layer under the text.
        let mut under = line(100, 200, "Default", "", "", "{\\bord6}Run!");
        under.layer = 0;
        let mut over = line(100, 200, "Default", "", "", "Run!");
        over.layer = 1;
        let plan = LinePlan::new(&[under, over], &none());
        assert_eq!(plan.translate_ids(), vec![1]);
        assert_eq!(plan.role(2), Role::Repeat { of: 1 });
    }

    #[test]
    fn frames_without_placement_tags_are_each_sent() {
        // Not typeset and not layered: nothing says it's one sign.
        let lines = vec![
            line(100, 104, "Signs", "", "", "{\\fad(100,0)}Sign"),
            line(104, 108, "Signs", "", "", "{\\fad(100,0)}Sign"),
        ];
        assert_eq!(LinePlan::new(&lines, &none()).translate_ids(), vec![1, 2]);
    }

    #[test]
    fn the_same_words_in_another_style_are_not_repeats() {
        let lines = vec![
            line(100, 200, "Default", "", "", "Heiter."),
            line(100, 200, "Signs", "", "", "Heiter."),
        ];
        assert_eq!(LinePlan::new(&lines, &none()).translate_ids(), vec![1, 2]);
    }

    #[test]
    fn runs_follow_time_not_file_order() {
        // The later frame comes first in the file: the earliest one translates.
        let lines = vec![sign(104, 1.0, "Sign"), sign(100, 1.0, "Sign")];
        let plan = LinePlan::new(&lines, &none());
        assert_eq!(plan.translate_ids(), vec![2]);
        assert_eq!(plan.role(1), Role::Repeat { of: 2 });
    }

    #[test]
    fn karaoke_drawings_and_empty_lines_are_kept() {
        let lines = vec![
            line(
                0,
                100,
                "OP-E",
                "",
                "fx",
                "{\\an5\\pos(666,1023)\\t(365,365,\\1c&HF1C472&)}It's",
            ),
            line(0, 100, "OP-E", "", "FX ", "{\\an5}sto"),
            line(0, 100, "OP-R", "", "fx", "{\\an5\\pos(1,1)}ka"),
            line(
                0,
                100,
                "Signs",
                "",
                "",
                "{\\an7\\pos(0,0)\\p1}m -2 -2 l 1922 -2 1922 1082 -2 1082{\\p0}",
            ),
            line(0, 100, "OP-E", "", "", ""),
            line(0, 100, "Signs", "", "", "{\\blur2}\\N\\h "),
            line(0, 100, "Default", "", "", "{\\pos(1,2)}Words."),
        ];
        let plan = LinePlan::new(&lines, &none());
        assert_eq!(plan.role(1), Role::Keep(Kept::Song));
        assert_eq!(plan.role(2), Role::Keep(Kept::Song));
        assert_eq!(plan.role(3), Role::Keep(Kept::Song));
        assert_eq!(plan.role(4), Role::Keep(Kept::Drawing));
        assert_eq!(plan.role(5), Role::Keep(Kept::NoText));
        assert_eq!(plan.role(6), Role::Keep(Kept::NoText));
        assert_eq!(plan.translate_ids(), vec![7]);
    }

    #[test]
    fn a_whole_lyric_line_with_karaoke_timing_is_translated() {
        // One line per lyric, timed with \k: words a viewer reads. Its style decides.
        let lines = vec![line(
            0,
            300,
            "OP",
            "",
            "",
            "{\\k37}{\\k12}风 {\\kf16}吹 {\\K27}过",
        )];
        assert_eq!(LinePlan::new(&lines, &none()).translate_ids(), vec![1]);
        let kept = BTreeSet::from(["OP".to_string()]);
        assert_eq!(
            LinePlan::new(&lines, &kept).role(1),
            Role::Keep(Kept::Style)
        );
    }

    #[test]
    fn pos_and_pbo_are_not_drawings() {
        let lines = vec![line(0, 100, "Signs", "", "", "{\\pos(1,2)\\pbo-3}Words")];
        assert_eq!(LinePlan::new(&lines, &none()).translate_ids(), vec![1]);
    }

    #[test]
    fn kept_styles_are_written_as_they_are() {
        let lines = vec![
            line(0, 100, "Default", "", "", "Hello."),
            line(0, 100, "Signs", "", "", "A sign"),
            line(0, 100, " Signs ", "", "", "Another sign"),
        ];
        let kept = BTreeSet::from(["Signs".to_string()]);
        let plan = LinePlan::new(&lines, &kept);
        assert_eq!(plan.translate_ids(), vec![1]);
        assert_eq!(plan.role(2), Role::Keep(Kept::Style));
        assert_eq!(plan.role(3), Role::Keep(Kept::Style));
    }

    #[test]
    fn the_output_fills_repeats_and_keeps_the_rest() {
        let lines = vec![
            sign(100, 1.0, "Holy City"),
            sign(104, 2.0, "Holy City"),
            line(0, 100, "OP-E", "", "fx", "{\\an5}sto"),
            line(200, 300, "Default", "", "", "Hello."),
            line(400, 500, "Default", "", "", "Missing."),
        ];
        let plan = LinePlan::new(&lines, &none());
        let translations = BTreeMap::from([
            (1, "{=8}{\\an6\\pos(1,139)}Cidade Santa".to_string()),
            (4, "Olá.".to_string()),
        ]);
        let out: Vec<String> = plan
            .output_lines(&lines, &translations)
            .into_iter()
            .map(|d| d.text)
            .collect();
        assert_eq!(
            out,
            vec![
                "{=8}{\\an6\\pos(1,139)}Cidade Santa".to_string(),
                "{=8}{\\an6\\pos(2,139)}Cidade Santa".to_string(),
                "{\\an5}sto".to_string(),
                "Olá.".to_string(),
            ]
        );
    }

    #[test]
    fn a_repeat_of_an_untranslated_line_stays_out() {
        let lines = vec![sign(100, 1.0, "Sign"), sign(104, 2.0, "Sign")];
        let plan = LinePlan::new(&lines, &none());
        assert!(plan.output_lines(&lines, &BTreeMap::new()).is_empty());
    }

    #[test]
    fn own_drops_translations_of_lines_that_are_not_sent() {
        let lines = vec![
            sign(100, 1.0, "Sign"),
            sign(104, 2.0, "Sign"),
            line(0, 1, "OP", "", "fx", "la"),
        ];
        let plan = LinePlan::new(&lines, &none());
        let stored = BTreeMap::from([
            (1, "a".to_string()),
            (2, "b".to_string()),
            (3, "c".to_string()),
        ]);
        assert_eq!(plan.own(&stored), BTreeMap::from([(1, "a".to_string())]));
    }

    #[test]
    fn tally_counts_lines_to_translate_per_style() {
        let lines = vec![
            line(100, 200, "Default", "Fern", "", "{\\i1}I see.{\\i0}"),
            line(300, 400, "Default", "Stark", "", "Yeah."),
            sign(100, 1.0, "Holy City"),
            sign(104, 2.0, "Holy City"),
            line(0, 100, "OP-E", "", "fx", "{\\an5}sto"),
            line(0, 100, "OP-E", "", "", ""),
        ];
        let t = tally(&lines);
        let style = |translate, all, sample: &str| StyleTally {
            translate,
            all,
            sample: sample.into(),
        };
        assert_eq!(t.styles["Default"], style(2, 2, "I see."));
        assert_eq!(t.styles["Signs"], style(1, 2, "Holy City"));
        assert_eq!(t.styles["OP-E"], style(0, 2, ""));
        assert_eq!((t.songs, t.no_text), (1, 1));
    }
}
