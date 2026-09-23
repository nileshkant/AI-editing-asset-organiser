use crate::{catalog::Sound, invalid, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, HashSet}, sync::LazyLock};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct FacetItem {
    pub value: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SearchFacets {
    pub tags: Vec<FacetItem>,
    pub layouts: Vec<FacetItem>,
    pub durations: Vec<FacetItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SearchQuery {
    pub text: String,
    /// Accepts both snake_case (internal/saved) and camelCase (Tauri v2 IPC).
    #[serde(alias = "sourceIds")]
    pub source_ids: Vec<String>,
    pub tags: Vec<String>,
    /// Accepts both snake_case and camelCase from Tauri v2 IPC.
    #[serde(alias = "favoritesOnly")]
    pub favorites_only: bool,
    #[serde(alias = "minDuration")]
    pub min_duration: Option<f64>,
    #[serde(alias = "maxDuration")]
    pub max_duration: Option<f64>,
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Interpretation {
    pub terms: Vec<String>,
    #[serde(default)]
    pub phrases: Vec<String>,
    pub excluded: Vec<String>,
    pub min_duration: Option<f64>,
    pub max_duration: Option<f64>,
    pub corrected: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchResults {
    pub items: Vec<Sound>,
    pub total: usize,
    pub interpretation: Interpretation,
    #[serde(default)]
    pub facets: SearchFacets,
}

static DURATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(under|below|less than|over|above|longer than)\s+(\d+(?:\.\d+)?)\s*(seconds?|secs?|s|minutes?|mins?)\b").unwrap()
});

const STOP: &[&str] = &[
    "a", "an", "the", "find", "me", "some", "any", "please", "sound", "sounds",
    "effect", "effects", "sfx", "audio", "for", "of", "that", "is", "like",
    "something", "need", "i", "want", "with", "and",
];

const GROUPS: &[&[&str]] = &[
    &["whoosh", "swoosh", "swish", "sweep"],
    &["impact", "hit", "thud", "slam"],
    &["hiss", "hissing"],
    &["scratch", "scrape", "scratching", "scraping"],
    &["rain", "rainfall", "raining", "raindrop", "raindrops"],
    &["train", "railway", "locomotive"],
    &["voice", "voices", "vocal", "vocals", "speech", "talking"],
    &["quiet", "soft", "gentle"],
    &["loud", "powerful"],
    &["bright", "sharp", "trebly"],
    &["dark", "bass", "bassy", "rumble", "rumbly"],
    &["footsteps", "footstep", "walking"],
    &["wind", "windy"],
    &["water", "liquid"],
    &["click", "clicking", "tap"],
    &["beep", "bleep"],
    &["door", "doors"],
    &["metal", "metallic"],
];

pub fn canonical(word: &str) -> String {
    GROUPS.iter().find(|g| g.contains(&word)).map(|g| g[0]).unwrap_or(word).to_owned()
}

pub fn deaccent(text: &str) -> String {
    text.chars().map(|c| match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }).collect()
}

pub fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(canonical)
        .collect()
}

pub fn interpret(query: &SearchQuery) -> Result<Interpretation> {
    if query.text.chars().count() > 512 || query.tags.len() > 64 || query.source_ids.len() > 128 || query.limit.unwrap_or(100) > 500 {
        return Err(invalid("Search exceeds supported limits"));
    }
    for value in [query.min_duration, query.max_duration].into_iter().flatten() {
        if !value.is_finite() || value < 0.0 {
            return Err(invalid("Duration must be a non-negative finite number"));
        }
    }
    let mut min = query.min_duration;
    let mut max = query.max_duration;
    let text = query.text.to_lowercase();

    for capture in DURATION.captures_iter(&text) {
        let mut value: f64 = capture[2].parse().map_err(|_| invalid("Invalid duration"))?;
        if capture[3].starts_with('m') { value *= 60.0; }
        if !value.is_finite() { return Err(invalid("Invalid duration")); }
        if ["under", "below", "less than"].contains(&&capture[1]) {
            max = Some(max.map_or(value, |v| v.min(value)));
        } else {
            min = Some(min.map_or(value, |v| v.max(value)));
        }
    }
    let remainder_raw = DURATION.replace_all(&text, " ");

    let chars: Vec<char> = remainder_raw.chars().collect();
    let mut i = 0;
    let mut phrases = Vec::new();
    let mut excluded = Vec::new();
    let mut terms = Vec::new();
    let mut negate = false;

    while i < chars.len() {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }

        let mut is_prefix_negated = false;
        if chars[i] == '-' {
            is_prefix_negated = true;
            i += 1;
        }

        if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
            let quote = chars[i];
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != quote {
                i += 1;
            }
            let phrase: String = chars[start..i].iter().collect();
            if i < chars.len() && chars[i] == quote {
                i += 1;
            }
            let clean = phrase.trim().to_lowercase();
            if !clean.is_empty() {
                if is_prefix_negated || negate {
                    if !excluded.contains(&clean) {
                        excluded.push(clean);
                    }
                    negate = false;
                } else if !phrases.contains(&clean) {
                    phrases.push(clean);
                }
            }
        } else {
            let start = i;
            while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '"' && chars[i] != '\'' {
                i += 1;
            }
            let word_str: String = chars[start..i].iter().collect();
            let word_tokens = tokens(&word_str);
            for term in word_tokens {
                if ["without", "not", "no", "excluding"].contains(&term.as_str()) {
                    negate = true;
                    continue;
                }
                if STOP.contains(&term.as_str()) {
                    continue;
                }
                if term == "short" && !negate && !is_prefix_negated {
                    max = Some(max.unwrap_or(3.0));
                    continue;
                }
                if negate || is_prefix_negated {
                    if !excluded.contains(&term) {
                        excluded.push(term);
                    }
                    negate = false;
                } else if !terms.contains(&term) {
                    terms.push(term);
                }
            }
        }
    }

    if min.zip(max).is_some_and(|(a, b)| a >= b) {
        return Err(invalid("Duration filters have no overlap"));
    }
    Ok(Interpretation {
        terms,
        phrases,
        excluded,
        min_duration: min,
        max_duration: max,
        corrected: vec![],
    })
}

pub fn search(sounds: Vec<Sound>, query: &SearchQuery, online_sources: &[String]) -> Result<SearchResults> {
    let mut parsed = interpret(query)?;
    let available: Vec<Sound> = sounds
        .into_iter()
        .filter(|s| s.status == "ready" && s.profile.is_some() && online_sources.contains(&s.source_id))
        .collect();

    let needs_typo_correction = parsed.terms.iter().any(|t| {
        t.chars().count() >= 4 && !GROUPS.iter().flat_map(|g| g.iter()).any(|w| *w == t.as_str())
    });

    if needs_typo_correction {
        let vocabulary: HashSet<String> = available.iter().flat_map(document).collect();
        for term in &mut parsed.terms {
            // Real words such as train and rain must not turn into one another.
            if vocabulary.contains(term) || GROUPS.iter().flat_map(|g| g.iter()).any(|w| *w == term.as_str()) || term.chars().count() < 4 {
                continue;
            }
            let budget = if term.chars().count() > 7 { 2 } else { 1 };
            let mut candidates: Vec<_> = vocabulary
                .iter()
                .filter_map(|word| {
                    let d = strsim::damerau_levenshtein(term, word);
                    (d <= budget).then_some((d, word))
                })
                .collect();
            candidates.sort();
            if let Some((_, replacement)) = candidates.first() {
                parsed.corrected.push(format!("{term} -> {replacement}"));
                *term = (*replacement).clone();
            }
        }
    }

    let unquoted_query = query.text.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_lowercase();
    let unquoted_deaccent = deaccent(&unquoted_query);
    let required_tags: Vec<String> = query.tags.iter().map(|t| t.replace('_', " ").trim().to_lowercase()).collect();

    let mut ranked: Vec<(usize, Sound)> = available
        .into_iter()
        .filter_map(|sound| {
            let duration = sound.profile.as_ref()?.duration;
            if query.favorites_only && !sound.favorite
                || !query.source_ids.is_empty() && !query.source_ids.contains(&sound.source_id)
                || parsed.min_duration.is_some_and(|v| duration <= v)
                || parsed.max_duration.is_some_and(|v| duration >= v)
            {
                return None;
            }

            // Required explicit tags filter
            if !required_tags.is_empty() {
                let all_tags: Vec<_> = sound
                    .user_tags
                    .iter()
                    .chain(sound.profile.as_ref()?.tags.iter())
                    .map(|t| t.replace('_', " ").to_lowercase())
                    .collect();
                if !required_tags.iter().all(|t| all_tags.contains(t)) {
                    return None;
                }
            }

            let doc = document(&sound);
            let doc_deaccent: Vec<String> = doc.iter().map(|s| deaccent(s)).collect();

            // Required unquoted terms: all must match
            for term in &parsed.terms {
                let term_deaccent = deaccent(term);
                let matches_doc = doc.contains(term) || doc_deaccent.contains(&term_deaccent);
                if !matches_doc {
                    let is_cjk = term.chars().any(|c| c >= '\u{2E80}');
                    if is_cjk {
                        let title_lower = sound.title.to_lowercase();
                        if title_lower.contains(term) || deaccent(&title_lower).contains(&term_deaccent) {
                            continue;
                        }
                    }
                    return None;
                }
            }

            let title_lower = sound.title.to_lowercase();
            let title_deaccent = deaccent(&title_lower);
            let tags_lower = sound.user_tags.join(" ").to_lowercase();
            let tags_deaccent = deaccent(&tags_lower);

            // Exclusions: check tokens and full string
            if !parsed.excluded.is_empty() {
                let comment_lower = sound.comment.to_lowercase();
                let profile_text = sound
                    .profile
                    .as_ref()
                    .map(|p| format!("{} {}", p.tags.join(" "), p.description).to_lowercase())
                    .unwrap_or_default();
                let full_doc = format!("{title_lower} {tags_lower} {comment_lower} {profile_text}");
                let full_doc_deaccent = format!("{title_deaccent} {tags_deaccent} {} {}", deaccent(&comment_lower), deaccent(&profile_text));

                if parsed.excluded.iter().any(|t| {
                    let t_deaccent = deaccent(t);
                    doc.contains(t) || doc_deaccent.contains(&t_deaccent) || full_doc.contains(t) || full_doc_deaccent.contains(&t_deaccent)
                }) {
                    return None;
                }
            }

            // Quoted phrases: must match exact phrase in document
            if !parsed.phrases.is_empty() {
                let comment_lower = sound.comment.to_lowercase();
                let profile_text = sound
                    .profile
                    .as_ref()
                    .map(|p| format!("{} {}", p.tags.join(" "), p.description).to_lowercase())
                    .unwrap_or_default();
                let full_doc = format!("{title_lower} {tags_lower} {comment_lower} {profile_text}");
                let full_doc_deaccent = format!("{title_deaccent} {tags_deaccent} {} {}", deaccent(&comment_lower), deaccent(&profile_text));

                for phrase in &parsed.phrases {
                    let phrase_deaccent = deaccent(phrase);
                    if !full_doc.contains(phrase) && !full_doc_deaccent.contains(&phrase_deaccent) {
                        return None;
                    }
                }
            }

            // Lexical ranking calculation
            let mut score = 0usize;

            // Exact full title match (+1000)
            let is_exact_title = (!unquoted_query.is_empty() && (title_lower == unquoted_query || title_deaccent == unquoted_deaccent))
                || parsed.phrases.iter().any(|p| title_lower == *p || title_deaccent == deaccent(p));
            let is_prefix_title = (!unquoted_query.is_empty() && (title_lower.starts_with(&unquoted_query) || title_deaccent.starts_with(&unquoted_deaccent)))
                || parsed.phrases.iter().any(|p| title_lower.starts_with(p) || title_deaccent.starts_with(&deaccent(p)));
            let is_substring_title = (!unquoted_query.is_empty() && (title_lower.contains(&unquoted_query) || title_deaccent.contains(&unquoted_deaccent)))
                || parsed.phrases.iter().any(|p| title_lower.contains(p) || title_deaccent.contains(&deaccent(p)));

            if is_exact_title {
                score += 1000;
            } else if is_prefix_title {
                score += 400;
            } else if is_substring_title {
                score += 200;
            }

            // Exact user tag match (+250)
            for tag in &sound.user_tags {
                let t = tag.to_lowercase();
                if !unquoted_query.is_empty() && (t == unquoted_query || deaccent(&t) == unquoted_deaccent) {
                    score += 250;
                }
            }

            // Quoted phrase matches (+100 - +300)
            for phrase in &parsed.phrases {
                let phrase_deaccent = deaccent(phrase);
                if title_lower.contains(phrase) || title_deaccent.contains(&phrase_deaccent) {
                    score += 300;
                } else if tags_lower.contains(phrase) || tags_deaccent.contains(&phrase_deaccent) {
                    score += 150;
                } else {
                    score += 80;
                }
            }

            // Token relevance
            let title_toks = tokens(&sound.title);
            let tag_toks = tokens(&sound.user_tags.join(" "));
            let profile_tag_toks = sound.profile.as_ref().map(|p| tokens(&p.tags.join(" "))).unwrap_or_default();
            let comment_toks = tokens(&sound.comment);
            let desc_toks = sound.profile.as_ref().map(|p| tokens(&p.description)).unwrap_or_default();

            for term in &parsed.terms {
                if title_toks.contains(term) { score += 30; }
                if tag_toks.contains(term) { score += 20; }
                if profile_tag_toks.contains(term) { score += 10; }
                if comment_toks.contains(term) { score += 4; }
                if desc_toks.contains(term) { score += 2; }
            }

            if score == 0 {
                score = 1;
            }

            Some((score, sound))
        })
        .collect();

    // Deterministic ranking: score desc, title length asc, title asc, id asc
    ranked.sort_by(|(score_a, sound_a), (score_b, sound_b)| {
        score_b
            .cmp(score_a)
            .then_with(|| sound_a.title.len().cmp(&sound_b.title.len()))
            .then_with(|| sound_a.title.to_lowercase().cmp(&sound_b.title.to_lowercase()))
            .then_with(|| sound_a.id.cmp(&sound_b.id))
    });

    // Compute facets across all matching sounds
    let mut tag_counts: HashMap<String, usize> = HashMap::new();
    let mut layout_counts: HashMap<String, usize> = HashMap::new();
    let mut short_count = 0;
    let mut med_count = 0;
    let mut long_count = 0;

    for (_, sound) in &ranked {
        for tag in sound.user_tags.iter().chain(sound.profile.as_ref().map(|p| p.tags.as_slice()).unwrap_or(&[])) {
            let clean = tag.replace('_', " ").trim().to_lowercase();
            if !clean.is_empty() {
                *tag_counts.entry(clean).or_insert(0) += 1;
            }
        }
        if let Some(p) = &sound.profile {
            let layout = if p.channel_layout.is_empty() {
                if p.channels == 1 {
                    "mono".into()
                } else if p.channels == 2 {
                    "stereo".into()
                } else {
                    format!("{} ch", p.channels)
                }
            } else {
                p.channel_layout.clone()
            };
            *layout_counts.entry(layout).or_insert(0) += 1;

            if p.duration < 3.0 {
                short_count += 1;
            } else if p.duration <= 15.0 {
                med_count += 1;
            } else {
                long_count += 1;
            }
        }
    }

    let mut tags_vec: Vec<FacetItem> = tag_counts.into_iter().map(|(value, count)| FacetItem { value, count }).collect();
    tags_vec.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));
    tags_vec.truncate(30);

    let mut layouts_vec: Vec<FacetItem> = layout_counts.into_iter().map(|(value, count)| FacetItem { value, count }).collect();
    layouts_vec.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.value.cmp(&b.value)));

    let mut durations_vec = Vec::new();
    if short_count > 0 { durations_vec.push(FacetItem { value: "under 3s".into(), count: short_count }); }
    if med_count > 0 { durations_vec.push(FacetItem { value: "3s - 15s".into(), count: med_count }); }
    if long_count > 0 { durations_vec.push(FacetItem { value: "over 15s".into(), count: long_count }); }

    let facets = SearchFacets {
        tags: tags_vec,
        layouts: layouts_vec,
        durations: durations_vec,
    };

    let total = ranked.len();
    let items = ranked
        .into_iter()
        .skip(query.offset)
        .take(query.limit.unwrap_or(100))
        .map(|(_, mut s)| {
            if let Some(p) = s.profile.as_mut() {
                p.waveform.clear();
            }
            s
        })
        .collect();

    Ok(SearchResults {
        items,
        total,
        interpretation: parsed,
        facets,
    })
}

fn document(sound: &Sound) -> Vec<String> {
    let mut out = tokens(&sound.title);
    out.extend(tokens(&sound.user_tags.join(" ")));
    out.extend(tokens(&sound.comment));
    if let Some(p) = &sound.profile {
        out.extend(tokens(&p.tags.join(" ")));
        out.extend(tokens(&p.description));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Profile;

    fn sound(title: &str, duration: f64) -> Sound {
        Sound {
            id: title.into(),
            source_id: "s".into(),
            relative_path: format!("{title}.wav"),
            title: title.into(),
            content_hash: "hash".into(),
            status: "ready".into(),
            profile: Some(Profile {
                duration,
                sample_rate: 48000,
                channels: 1,
                frames: 48000,
                peak: 0.5,
                rms: 0.1,
                channel_peaks: vec![0.5],
                channel_rms: vec![0.1],
                channel_layout: "mono".into(),
                description: "Measured profile".into(),
                tags: vec![],
                waveform: vec![[-0.5, 0.5]],
            }),
            user_tags: vec![],
            comment: String::new(),
            favorite: false,
        }
    }

    fn find(text: &str, items: Vec<Sound>) -> SearchResults {
        search(items, &SearchQuery { text: text.into(), ..Default::default() }, &["s".into()]).unwrap()
    }

    #[test]
    fn natural_duration() {
        let r = find("find me a whoosh under 3 seconds", vec![sound("Whoosh", 2.0), sound("Long whoosh", 4.0)]);
        assert_eq!(r.total, 1);
        assert_eq!(r.interpretation.max_duration, Some(3.0));
    }

    #[test]
    fn typo() {
        let r = find("whooshh", vec![sound("Whoosh", 1.0)]);
        assert_eq!(r.total, 1);
        assert_eq!(r.interpretation.corrected.len(), 1);
    }

    #[test]
    fn transposition() {
        assert_eq!(find("scartch", vec![sound("Scratch", 1.0)]).total, 1);
    }

    #[test]
    fn synonyms() {
        assert_eq!(find("swoosh", vec![sound("Whoosh", 1.0)]).total, 1);
    }

    #[test]
    fn no_train_rain_confusion() {
        assert_eq!(find("train", vec![sound("Rain", 1.0)]).total, 0);
    }

    #[test]
    fn exclusions() {
        assert_eq!(find("rain without vocals", vec![sound("Rain with speech", 1.0), sound("Rain", 1.0)]).total, 1);
    }

    #[test]
    fn prefix_negative_exclusion() {
        assert_eq!(find("rain -vocals", vec![sound("Rain with speech", 1.0), sound("Rain", 1.0)]).total, 1);
    }

    #[test]
    fn missing_hidden() {
        let mut s = sound("Whoosh", 1.0);
        s.status = "missing".into();
        assert_eq!(find("", vec![s]).total, 0);
    }

    #[test]
    fn strict_boundary() {
        assert_eq!(find("under 3 seconds", vec![sound("Whoosh", 3.0)]).total, 0);
    }

    #[test]
    fn annotations_searchable() {
        let mut s = sound("0001", 1.0);
        s.user_tags = vec!["space laser".into()];
        s.comment = "use for reveal".into();
        assert_eq!(find("laser reveal", vec![s]).total, 1);
    }

    #[test]
    fn short_default() {
        assert_eq!(find("short hit", vec![sound("impact", 2.0), sound("hit long", 5.0)]).total, 1);
    }

    #[test]
    fn invalid_query() {
        assert!(interpret(&SearchQuery { min_duration: Some(f64::NAN), ..Default::default() }).is_err());
        assert!(interpret(&SearchQuery { text: "over 5 seconds under 2 seconds".into(), ..Default::default() }).is_err());
    }

    #[test]
    fn pagination_and_waveform_omission() {
        let r = search(vec![sound("a", 1.0), sound("b", 1.0)], &SearchQuery { offset: 1, limit: Some(1), ..Default::default() }, &["s".into()]).unwrap();
        assert_eq!(r.total, 2);
        assert_eq!(r.items[0].title, "b");
        assert!(r.items[0].profile.as_ref().unwrap().waveform.is_empty());
    }

    #[test]
    fn offline_hidden() {
        assert_eq!(search(vec![sound("a", 1.0)], &SearchQuery::default(), &[]).unwrap().total, 0);
    }

    #[test]
    fn exact_filters() {
        let mut s = sound("a", 1.0);
        s.favorite = true;
        s.user_tags = vec!["high pitch".into()];
        let q = SearchQuery { favorites_only: true, tags: vec!["high_pitch".into()], ..Default::default() };
        assert_eq!(search(vec![s], &q, &["s".into()]).unwrap().total, 1);
    }

    #[test]
    fn quoted_phrase_and_exact_ranking() {
        let exact = sound("Laser Blast", 1.0);
        let partial = sound("Heavy Gun with Laser Blast Echo", 1.0);
        let separate = sound("Laser Cannon with Loud Blast", 1.0);
        let r = find(r#""laser blast""#, vec![separate, exact.clone(), partial]);
        // Quoted phrase requires "laser blast" contiguous: all 3 have both words, but exact title matches first!
        assert!(r.total >= 2);
        assert_eq!(r.items[0].title, "Laser Blast");
    }

    #[test]
    fn diacritics_normalization() {
        let flute = sound("Flûte enchantée", 2.0);
        let r = find("flute", vec![flute]);
        assert_eq!(r.total, 1);
    }
}
