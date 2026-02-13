/// MTA GTFS-RT feed URL mapping.
///
/// Each feed covers a group of routes. The feed IDs map to:
/// `https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs{suffix}`

/// Returns the feed URL suffix for a given route.
pub fn feed_id_for_route(route: &str) -> Option<&'static str> {
    match route {
        // IRT: 1, 2, 3, 4, 5, 6, GS
        "1" | "2" | "3" | "4" | "5" | "6" | "GS" => Some(""),
        // IND/BMT: A, C, E
        "A" | "C" | "E" => Some("-ace"),
        // IND: B, D, F, M
        "B" | "D" | "F" | "M" => Some("-bdfm"),
        // BMT: G
        "G" => Some("-g"),
        // BMT: J, Z
        "J" | "Z" => Some("-jz"),
        // BMT: N, Q, R, W
        "N" | "Q" | "R" | "W" => Some("-nqrw"),
        // BMT: L
        "L" => Some("-l"),
        // IRT: 7
        "7" => Some("-7"),
        // SIR (Staten Island Railway)
        "SI" | "SIR" => Some("-si"),
        _ => None,
    }
}

/// Base URL for MTA GTFS-RT feeds.
pub const MTA_FEED_BASE_URL: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/nyct%2Fgtfs";

/// Returns the full feed URL for a route.
pub fn feed_url_for_route(route: &str) -> Option<String> {
    feed_id_for_route(route).map(|suffix| format!("{}{}", MTA_FEED_BASE_URL, suffix))
}

/// Returns deduplicated feed URLs needed for a set of routes.
pub fn feed_urls_for_routes(routes: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();
    for route in routes {
        if let Some(suffix) = feed_id_for_route(route) {
            if seen.insert(suffix) {
                urls.push(format!("{}{}", MTA_FEED_BASE_URL, suffix));
            }
        }
    }
    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feed_id_for_known_routes() {
        assert_eq!(feed_id_for_route("1"), Some(""));
        assert_eq!(feed_id_for_route("A"), Some("-ace"));
        assert_eq!(feed_id_for_route("N"), Some("-nqrw"));
        assert_eq!(feed_id_for_route("L"), Some("-l"));
        assert_eq!(feed_id_for_route("7"), Some("-7"));
    }

    #[test]
    fn test_feed_id_for_unknown_route() {
        assert_eq!(feed_id_for_route("X"), None);
    }

    #[test]
    fn test_feed_urls_deduplication() {
        let routes: Vec<String> = vec!["1".into(), "2".into(), "3".into(), "A".into()];
        let urls = feed_urls_for_routes(&routes);
        // 1, 2, 3 share the same feed; A is separate
        assert_eq!(urls.len(), 2);
    }
}
