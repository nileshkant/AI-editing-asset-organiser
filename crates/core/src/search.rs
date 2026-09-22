use crate::{catalog::Sound, invalid, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::LazyLock};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchQuery {
    pub text: String,
    pub source_ids: Vec<String>,
    pub tags: Vec<String>,
    pub favorites_only: bool,
    pub min_duration: Option<f64>,
    pub max_duration: Option<f64>,
    pub offset: usize,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Interpretation {
    pub terms: Vec<String>,
    pub excluded: Vec<String>,
    pub min_duration: Option<f64>,
    pub max_duration: Option<f64>,
    pub corrected: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct SearchResults { pub items: Vec<Sound>, pub total: usize, pub interpretation: Interpretation }

static DURATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(under|below|less than|over|above|longer than)\s+(\d+(?:\.\d+)?)\s*(seconds?|secs?|s|minutes?|mins?)\b").unwrap());
const STOP: &[&str] = &["a","an","the","find","me","some","any","please","sound","sounds","effect","effects","sfx","audio","for","of","that","is","like","something","need","i","want","with","and"];
const GROUPS: &[&[&str]] = &[
    &["whoosh","swoosh","swish","sweep"], &["impact","hit","thud","slam"],
    &["hiss","hissing"], &["scratch","scrape","scratching","scraping"],
    &["rain","rainfall","raining"], &["train","railway","locomotive"],
    &["voice","voices","vocal","vocals","speech","talking"],
    &["quiet","soft","gentle"], &["loud","powerful"], &["bright","sharp","trebly"],
    &["dark","bass","bassy","rumble","rumbly"], &["footsteps","footstep","walking"],
    &["wind","windy"], &["water","liquid"], &["click","clicking","tap"],
    &["beep","bleep"], &["door","doors"], &["metal","metallic"],
];
fn canonical(word: &str) -> String { GROUPS.iter().find(|g|g.contains(&word)).map(|g|g[0]).unwrap_or(word).to_owned() }
fn tokens(text: &str) -> Vec<String> { text.to_lowercase().split(|c:char|!c.is_alphanumeric()).filter(|s|!s.is_empty()).map(canonical).collect() }

pub fn interpret(query: &SearchQuery) -> Result<Interpretation> {
    if query.text.chars().count()>512 || query.tags.len()>64 || query.source_ids.len()>128 || query.limit.unwrap_or(100)>500 { return Err(invalid("Search exceeds supported limits")); }
    for value in [query.min_duration,query.max_duration].into_iter().flatten() { if !value.is_finite() || value<0.0 { return Err(invalid("Duration must be a non-negative finite number")); } }
    let mut min=query.min_duration; let mut max=query.max_duration;
    let text=query.text.to_lowercase();
    for capture in DURATION.captures_iter(&text) {
        let mut value: f64=capture[2].parse().map_err(|_|invalid("Invalid duration"))?;
        if capture[3].starts_with('m') { value*=60.0; }
        if !value.is_finite() { return Err(invalid("Invalid duration")); }
        if ["under","below","less than"].contains(&&capture[1]) {max=Some(max.map_or(value,|v|v.min(value)));} else {min=Some(min.map_or(value,|v|v.max(value)));}
    }
    let remainder=DURATION.replace_all(&text, " ");
    let mut terms=vec![];let mut excluded=vec![];let mut negate=false;
    for term in tokens(&remainder) {
        if ["without","not","no","excluding"].contains(&term.as_str()) {negate=true;continue;}
        if STOP.contains(&term.as_str()) {continue;}
        if term=="short" && !negate {max=Some(max.unwrap_or(3.0));continue;}
        if negate {excluded.push(term);negate=false;} else {terms.push(term);}
    }
    if min.zip(max).is_some_and(|(a,b)| a>=b) { return Err(invalid("Duration filters have no overlap")); }
    Ok(Interpretation {terms,excluded,min_duration:min,max_duration:max,corrected:vec![]})
}

pub fn search(sounds: Vec<Sound>, query: &SearchQuery, online_sources: &[String]) -> Result<SearchResults> {
    let mut parsed=interpret(query)?;
    let available: Vec<Sound> = sounds.into_iter().filter(|s|s.status=="ready" && s.profile.is_some() && online_sources.contains(&s.source_id)).collect();
    let vocabulary: HashSet<String> = available.iter().flat_map(document).collect();
    for term in &mut parsed.terms {
        // Real words such as train and rain must not turn into one another.
        if vocabulary.contains(term) || GROUPS.iter().flat_map(|g|g.iter()).any(|w|*w==term.as_str()) || term.chars().count()<4 {continue;}
        let budget=if term.chars().count()>7 {2} else {1};
        let mut candidates: Vec<_>=vocabulary.iter().filter_map(|word| {let d=strsim::damerau_levenshtein(term,word);(d<=budget).then_some((d,word))}).collect();
        candidates.sort();
        if let Some((_,replacement))=candidates.first() {parsed.corrected.push(format!("{term} -> {replacement}"));*term=(*replacement).clone();}
    }
    let required_tags: Vec<String>=query.tags.iter().map(|t|t.replace('_'," ").trim().to_lowercase()).collect();
    let mut ranked: Vec<(usize,Sound)>=available.into_iter().filter_map(|sound| {
        let duration=sound.profile.as_ref()?.duration;
        if query.favorites_only && !sound.favorite || !query.source_ids.is_empty() && !query.source_ids.contains(&sound.source_id) || parsed.min_duration.is_some_and(|v|duration<=v) || parsed.max_duration.is_some_and(|v|duration>=v) {return None;}
        let title=tokens(&sound.title);let tags=tokens(&sound.user_tags.join(" "));let doc=document(&sound);
        if parsed.excluded.iter().any(|t|doc.contains(t)) || parsed.terms.iter().any(|t|!doc.contains(t)) {return None;}
        let all_tags: Vec<_>=sound.user_tags.iter().chain(sound.profile.as_ref()?.tags.iter()).map(|t|t.replace('_'," ").to_lowercase()).collect();
        if !required_tags.iter().all(|t|all_tags.contains(t)) {return None;}
        let score=parsed.terms.iter().map(|t|if title.contains(t) {8} else if tags.contains(t) {5} else {1}).sum();
        Some((score,sound))
    }).collect();
    ranked.sort_by(|(a,x),(b,y)|b.cmp(a).then_with(||x.title.cmp(&y.title)).then_with(||x.id.cmp(&y.id)));
    let total=ranked.len();
    let items=ranked.into_iter().skip(query.offset).take(query.limit.unwrap_or(100)).map(|(_,mut s)|{if let Some(p)=s.profile.as_mut(){p.waveform.clear();}s}).collect();
    Ok(SearchResults {items,total,interpretation:parsed})
}
fn document(sound: &Sound) -> Vec<String> {
    let mut out=tokens(&sound.title);out.extend(tokens(&sound.user_tags.join(" ")));out.extend(tokens(&sound.comment));
    if let Some(p)=&sound.profile {out.extend(tokens(&p.tags.join(" ")));out.extend(tokens(&p.description));}out
}

#[cfg(test)] mod tests {
    use super::*;
    use crate::catalog::Profile;
    fn sound(title:&str,duration:f64)->Sound {Sound{id:title.into(),source_id:"s".into(),relative_path:format!("{title}.wav"),title:title.into(),content_hash:"hash".into(),status:"ready".into(),profile:Some(Profile{duration,sample_rate:48000,channels:1,frames:48000,peak:0.5,rms:0.1,channel_peaks:vec![0.5],channel_rms:vec![0.1],channel_layout:"mono".into(),description:"Measured profile".into(),tags:vec![],waveform:vec![[-0.5,0.5]]}),user_tags:vec![],comment:String::new(),favorite:false}}
    fn find(text:&str,items:Vec<Sound>)->SearchResults{search(items,&SearchQuery{text:text.into(),..Default::default()},&["s".into()]).unwrap()}
    #[test]fn natural_duration(){let r=find("find me a whoosh under 3 seconds",vec![sound("Whoosh",2.0),sound("Long whoosh",4.0)]);assert_eq!(r.total,1);assert_eq!(r.interpretation.max_duration,Some(3.0));}
    #[test]fn typo(){let r=find("whooshh",vec![sound("Whoosh",1.0)]);assert_eq!(r.total,1);assert_eq!(r.interpretation.corrected.len(),1);}
    #[test]fn transposition(){assert_eq!(find("scartch",vec![sound("Scratch",1.0)]).total,1);}
    #[test]fn synonyms(){assert_eq!(find("swoosh",vec![sound("Whoosh",1.0)]).total,1);}
    #[test]fn no_train_rain_confusion(){assert_eq!(find("train",vec![sound("Rain",1.0)]).total,0);}
    #[test]fn exclusions(){assert_eq!(find("rain without vocals",vec![sound("Rain with speech",1.0),sound("Rain",1.0)]).total,1);}
    #[test]fn missing_hidden(){let mut s=sound("Whoosh",1.0);s.status="missing".into();assert_eq!(find("",vec![s]).total,0);}
    #[test]fn strict_boundary(){assert_eq!(find("under 3 seconds",vec![sound("Whoosh",3.0)]).total,0);}
    #[test]fn annotations_searchable(){let mut s=sound("0001",1.0);s.user_tags=vec!["space laser".into()];s.comment="use for reveal".into();assert_eq!(find("laser reveal",vec![s]).total,1);}
    #[test]fn short_default(){assert_eq!(find("short hit",vec![sound("impact",2.0),sound("hit long",5.0)]).total,1);}
    #[test]fn invalid_query(){assert!(interpret(&SearchQuery{min_duration:Some(f64::NAN),..Default::default()}).is_err());assert!(interpret(&SearchQuery{text:"over 5 seconds under 2 seconds".into(),..Default::default()}).is_err());}
    #[test]fn pagination_and_waveform_omission(){let r=search(vec![sound("a",1.0),sound("b",1.0)],&SearchQuery{offset:1,limit:Some(1),..Default::default()},&["s".into()]).unwrap();assert_eq!(r.total,2);assert_eq!(r.items[0].title,"b");assert!(r.items[0].profile.as_ref().unwrap().waveform.is_empty());}
    #[test]fn offline_hidden(){assert_eq!(search(vec![sound("a",1.0)],&SearchQuery::default(),&[]).unwrap().total,0);}
    #[test]fn exact_filters(){let mut s=sound("a",1.0);s.favorite=true;s.user_tags=vec!["high pitch".into()];let q=SearchQuery{favorites_only:true,tags:vec!["high_pitch".into()],..Default::default()};assert_eq!(search(vec![s],&q,&["s".into()]).unwrap().total,1);}
}
