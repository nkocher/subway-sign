use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Raw station record from the JSON database.
#[derive(Debug, Clone, Deserialize)]
pub struct Station {
    pub name: String,
    pub stop_ids: Vec<String>,
    pub routes: Vec<String>,
    pub lat: f64,
    pub lon: f64,
    pub borough: String,
    pub platform_count: u32,
}

/// Embedded station database JSON (compiled into the binary).
const STATION_DB_JSON: &str =
    include_str!("../../assets/mta_stations_complete.json");

/// Loaded station database + fuzzy index.
struct StationDb {
    stations: Vec<Station>,
    /// Lookup index: normalized name → index into `stations`.
    index: HashMap<String, usize>,
    /// Reverse lookup: base stop ID (without N/S suffix) → station name.
    stop_id_to_name: HashMap<String, String>,
}

static STATION_DB: OnceLock<StationDb> = OnceLock::new();

fn get_db() -> &'static StationDb {
    STATION_DB.get_or_init(|| {
        let stations: Vec<Station> =
            serde_json::from_str(STATION_DB_JSON).expect("embedded station DB is valid JSON");

        let mut index = HashMap::new();
        for (i, station) in stations.iter().enumerate() {
            // Exact lowercase
            index.insert(station.name.to_lowercase(), i);
            // Normalized form
            let normalized = normalize_station_name(&station.name);
            index.entry(normalized).or_insert(i);
        }

        // Build reverse index: base stop ID → station name
        let mut stop_id_to_name = HashMap::new();
        for station in &stations {
            for sid in &station.stop_ids {
                let base = sid.trim_end_matches(['N', 'S']);
                stop_id_to_name
                    .entry(base.to_string())
                    .or_insert_with(|| station.name.clone());
            }
        }

        StationDb { stations, index, stop_id_to_name }
    })
}

/// Normalize a station name for fuzzy matching.
///
/// - Remove ordinal suffixes (1st → 1, 2nd → 2, etc.)
/// - Standardize spacing/punctuation around dashes
/// - Common abbreviations (street → st, avenue → av, square → sq)
/// - Lowercase
fn normalize_station_name(name: &str) -> String {
    static RE_ORDINAL: OnceLock<Regex> = OnceLock::new();
    static RE_DASH_SPACES: OnceLock<Regex> = OnceLock::new();
    static RE_MULTI_SPACE: OnceLock<Regex> = OnceLock::new();

    let re_ordinal =
        RE_ORDINAL.get_or_init(|| Regex::new(r"(\d+)(st|nd|rd|th)\b").unwrap());
    let re_dash_spaces =
        RE_DASH_SPACES.get_or_init(|| Regex::new(r"\s*-\s*").unwrap());
    let re_multi_space =
        RE_MULTI_SPACE.get_or_init(|| Regex::new(r"\s+").unwrap());

    let mut s = name.to_lowercase();
    // Remove ordinal suffixes
    s = re_ordinal.replace_all(&s, "$1").to_string();
    // Standardize dashes
    s = re_dash_spaces.replace_all(&s, "-").to_string();
    // Collapse whitespace
    s = re_multi_space.replace_all(&s, " ").to_string();
    // Common abbreviations
    s = s.replace("street", "st");
    s = s.replace("avenue", "av");
    s = s.replace("square", "sq");
    s.trim().to_string()
}

/// Get all stop IDs for a station name with fuzzy matching.
///
/// Tries matching in order: exact → dash-normalized → full-normalized → substring.
pub fn get_stop_ids_for_station(station_name: &str) -> Vec<String> {
    let db = get_db();
    if station_name.is_empty() {
        return Vec::new();
    }

    let name_lower = station_name.to_lowercase().trim().to_string();

    // Exact match
    if let Some(&idx) = db.index.get(&name_lower) {
        return db.stations[idx].stop_ids.clone();
    }

    // Dash normalization
    let normalized_dash = name_lower.replace(" - ", "-").replace("  ", " ");
    if let Some(&idx) = db.index.get(&normalized_dash) {
        return db.stations[idx].stop_ids.clone();
    }

    // Full normalization
    let normalized = normalize_station_name(station_name);
    if let Some(&idx) = db.index.get(&normalized) {
        return db.stations[idx].stop_ids.clone();
    }

    // Substring match
    let normalized_query = normalized.replace('-', " ");
    for (indexed_name, &idx) in &db.index {
        let indexed_normalized = indexed_name.replace('-', " ");
        if normalized_query.contains(&indexed_normalized)
            || indexed_normalized.contains(&normalized_query)
        {
            return db.stations[idx].stop_ids.clone();
        }
    }

    Vec::new()
}

/// Get the full station database.
pub fn get_station_database() -> &'static [Station] {
    &get_db().stations
}

/// Find stations with names matching a query, ranked by word overlap.
pub fn find_similar_stations(query: &str, max_results: usize) -> Vec<String> {
    let db = get_db();
    let query_lower = query.to_lowercase();
    let query_replaced = query_lower.replace('-', " ");
    let query_words: std::collections::HashSet<&str> =
        query_replaced.split_whitespace().collect();

    let mut matches: Vec<(f64, &str)> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for station in &db.stations {
        if !seen.insert(&station.name) {
            continue;
        }
        let name_lower = station.name.to_lowercase();
        let name_replaced = name_lower.replace('-', " ");
        let name_words: std::collections::HashSet<&str> =
            name_replaced.split_whitespace().collect();

        let common = query_words.intersection(&name_words).count();
        if common > 0 {
            let score =
                common as f64 / query_words.len().max(name_words.len()) as f64;
            if score > 0.2 {
                matches.push((score, &station.name));
            }
        }
    }

    matches.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    matches
        .into_iter()
        .take(max_results)
        .map(|(_, name)| name.to_string())
        .collect()
}

/// Look up station name from a stop ID (e.g., "635N" → "Times Sq-42 St").
///
/// Strips the N/S direction suffix before matching.
pub fn station_name_for_stop_id(stop_id: &str) -> Option<&'static str> {
    let db = get_db();
    let base = stop_id.trim_end_matches(['N', 'S']);
    db.stop_id_to_name.get(base).map(|s| s.as_str())
}

/// Look up routes served at a station by name.
pub fn get_routes_for_station(station_name: &str) -> Vec<String> {
    let db = get_db();
    let name_lower = station_name.to_lowercase();
    for station in &db.stations {
        if station.name.to_lowercase() == name_lower {
            return station.routes.clone();
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_station_name() {
        assert_eq!(normalize_station_name("42nd Street"), "42 st");
        assert_eq!(normalize_station_name("Times Sq - 42 St"), "times sq-42 st");
        assert_eq!(normalize_station_name("1st Avenue"), "1 av");
        assert_eq!(normalize_station_name("  103 St  "), "103 st");
    }

    #[test]
    fn test_station_db_loads() {
        let db = get_station_database();
        assert!(!db.is_empty(), "station database should have entries");
        // Spot check a well-known station
        let has_times_sq = db.iter().any(|s| s.name.contains("Times Sq"));
        assert!(has_times_sq, "should contain Times Sq");
    }

    #[test]
    fn test_exact_lookup() {
        let ids = get_stop_ids_for_station("Times Sq-42 St");
        assert!(!ids.is_empty(), "Times Sq-42 St should have stop IDs");
    }

    #[test]
    fn test_fuzzy_lookup() {
        // "times square 42 street" should match via normalization
        let ids = get_stop_ids_for_station("times square 42 street");
        assert!(!ids.is_empty(), "fuzzy match should find Times Sq-42 St");
    }

    #[test]
    fn test_unknown_station() {
        let ids = get_stop_ids_for_station("Nonexistent Station XYZ");
        assert!(ids.is_empty());
    }

    #[test]
    fn test_find_similar_stations() {
        let results = find_similar_stations("42", 5);
        assert!(!results.is_empty(), "should find stations containing '42'");
        assert!(results.len() <= 5);
    }

    #[test]
    fn test_get_routes_for_station() {
        let routes = get_routes_for_station("Times Sq-42 St");
        assert!(!routes.is_empty(), "Times Sq should have routes");
    }

    #[test]
    fn test_empty_query() {
        assert!(get_stop_ids_for_station("").is_empty());
    }
}
