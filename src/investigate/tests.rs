use super::model::{PboGroup, ScoredCandidate};
use super::scoring::score_candidate;

fn candidate(title: &str, id: &str) -> ScoredCandidate {
    ScoredCandidate {
        id: id.to_string(),
        title: title.to_string(),
        search_score: 0.5,
        subscriptions: 1000,
        views: 5000,
        favorited: 100,
        star_rating: 4.5,
        total_votes: 50,
        tags: vec!["Mod".to_string()],
        creator: String::new(),
        creator_name: String::new(),
        time_updated: 0,
        short_description: String::new(),
        children: Vec::new(),
        file_type: 0,
    }
}

fn group(term: &str, pbos: &[&str]) -> PboGroup {
    PboGroup {
        search_term: term.to_string(),
        extra_terms: Vec::new(),
        pbos: pbos.iter().map(|s| s.to_string()).collect(),
        prefix: None,
        author_handle: None,
        cfg_author: None,
        cfg_url: None,
        cfg_name: None,
        cfg_children: Vec::new(),
    }
}

#[test]
fn pbo_name_boost_surfaces_descriptive_candidate() {
    // sps_blackhornet.pbo → search term "SPS". The correct mod
    // "SPS BlackHornet PRS" must outrank the similarly-named but
    // wrong "SPS AI AXMC Sniper Rifle Series" because its title
    // contains the descriptive PBO word "blackhornet".
    let g = group("SPS", &["sps_blackhornet.pbo"]);
    let correct = candidate("SPS BlackHornet PRS", "2457052493");
    let wrong = candidate("SPS AI AXMC Sniper Rifle Series", "1510335080");
    assert!(score_candidate(&correct, &g) > score_candidate(&wrong, &g));
}

#[test]
fn pbo_name_boost_splits_camelcase_stems() {
    // FranksMarkers.pbo → the stem splits into ["franks", "markers"].
    // "NATO Markers+" contains "markers" and must outrank
    // "Crye Gen 3 Uniforms (NATO Retexture)" which contains neither.
    let g = group("NATO", &["FranksMarkers.pbo"]);
    let markers = candidate("NATO Markers+", "1340701737");
    let uniforms = candidate("Crye Gen 3 Uniforms (NATO Retexture)", "724064220");
    assert!(score_candidate(&markers, &g) > score_candidate(&uniforms, &g));
}

#[test]
fn short_term_rejects_prefix_noise() {
    // "sty" must not match "Bodycam Style Aiming" — "sty" is a
    // prefix of "style", not a word or suffix in the title.
    let g = group("sty", &["sty_equipment.pbo"]);
    let noise = candidate("Bodycam Style Aiming", "3514524021");
    let g2 = group("sty", &["sty_equipment.pbo"]);
    let unrelated = candidate("S.T.Y. Equipment", "999");
    assert!(score_candidate(&noise, &g) < score_candidate(&unrelated, &g2));
}
