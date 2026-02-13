use std::collections::{HashMap, HashSet};
use std::time::Instant;

use prost::Message;
use reqwest::Client;
use tokio::task::JoinSet;
use tracing::{debug, warn};

use crate::models::{Alert, Direction, Train};
use crate::mta::alerts::extract_priority_from_effect;
use crate::mta::feeds;

/// Generated protobuf types from gtfs-realtime.proto.
pub mod transit_realtime {
    include!(concat!(env!("OUT_DIR"), "/transit_realtime.rs"));
}

/// Cache TTL — entries older than this are eligible for cleanup.
const CACHE_TTL_SECONDS: u64 = 300;

/// Minimum interval between logging the same error source.
const ERROR_LOG_INTERVAL_SECS: u64 = 300;

/// MTA alerts feed URL.
const ALERTS_URL: &str =
    "https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/camsys%2Fsubway-alerts";

/// Cached feed data.
struct FeedCacheEntry {
    trains: Vec<Train>,
    fetched_at: Instant,
}

/// Backoff tracking for a feed.
struct BackoffState {
    failures: u32,
    retry_after: Instant,
}

/// MTA API client with connection pooling, caching, and exponential backoff.
///
/// Never panics — all errors are handled internally and logged.
/// Returns empty data or cached data on error.
pub struct MtaClient {
    http: Client,
    feed_cache: HashMap<String, FeedCacheEntry>,
    alerts_cache: Vec<Alert>,
    alerts_etag: Option<String>,
    backoff: HashMap<String, BackoffState>,
    last_error_log: HashMap<String, Instant>,
}

impl MtaClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .user_agent("NYC-SubwaySign-Rust/1.0")
            .gzip(true)
            .pool_max_idle_per_host(4)
            .timeout(std::time::Duration::from_secs(12))
            .build()
            .expect("failed to create HTTP client");

        MtaClient {
            http,
            feed_cache: HashMap::new(),
            alerts_cache: Vec::new(),
            alerts_etag: None,
            backoff: HashMap::new(),
            last_error_log: HashMap::new(),
        }
    }

    /// Fetch upcoming trains for given stops and routes in parallel.
    pub async fn fetch_trains(
        &mut self,
        stop_ids: &[String],
        routes: &HashSet<String>,
        max_count: usize,
    ) -> Vec<Train> {
        let feed_urls = feeds::feed_urls_for_routes(
            &routes.iter().cloned().collect::<Vec<_>>(),
        );

        let mut join_set = JoinSet::new();

        // Spawn parallel fetch tasks
        for url in &feed_urls {
            if !self.should_fetch(url) {
                continue; // In backoff — skip, use cache later
            }

            let http = self.http.clone();
            let url = url.clone();
            let stop_ids = stop_ids.to_vec();
            let routes = routes.clone();

            join_set.spawn(async move {
                let result = fetch_single_feed(&http, &url, &stop_ids, &routes).await;
                (url, result)
            });
        }

        let mut all_trains: Vec<Train> = Vec::new();

        // Collect results
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok((url, Ok((trains, _timestamp)))) => {
                    self.record_success(&url);
                    self.feed_cache.insert(
                        url,
                        FeedCacheEntry {
                            trains: trains.clone(),
                            fetched_at: Instant::now(),
                        },
                    );
                    all_trains.extend(trains);
                }
                Ok((url, Err(e))) => {
                    self.log_error(&format!("feed_{}", url), &format!("Error fetching {}: {}", url, e));
                    self.record_failure(&url);
                    // Use cached data as fallback
                    if let Some(cached) = self.feed_cache.get(&url) {
                        all_trains.extend(cached.trains.clone());
                    }
                }
                Err(e) => {
                    warn!("Feed fetch task panicked: {}", e);
                }
            }
        }

        // Also include cached data for feeds we skipped due to backoff
        for url in &feed_urls {
            if !self.should_fetch(url) {
                if let Some(cached) = self.feed_cache.get(url) {
                    all_trains.extend(cached.trains.clone());
                }
            }
        }

        // Cleanup stale cache entries
        self.cleanup_feed_cache();

        // Sort and deduplicate
        all_trains.sort_by(|a, b| {
            a.arrival_timestamp
                .partial_cmp(&b.arrival_timestamp)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let unique = deduplicate_trains(all_trains);
        unique.into_iter().take(max_count).collect()
    }

    /// Fetch service alerts for given routes.
    pub async fn fetch_alerts(&mut self, routes: &HashSet<String>) -> Vec<Alert> {
        let feed_id = "alerts";

        if !self.should_fetch(feed_id) {
            return self.alerts_cache.clone();
        }

        let mut req = self.http.get(ALERTS_URL);
        if let Some(ref etag) = self.alerts_etag {
            req = req.header("If-None-Match", etag);
        }

        let response = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                self.log_error("alerts", &format!("Error fetching alerts: {}", e));
                self.record_failure(feed_id);
                return self.alerts_cache.clone();
            }
        };

        // Handle 304 Not Modified
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            self.record_success(feed_id);
            return self.alerts_cache.clone();
        }

        // Store ETag
        if let Some(etag) = response.headers().get("etag") {
            self.alerts_etag = etag.to_str().ok().map(|s| s.to_string());
        }

        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => {
                self.log_error("alerts", &format!("Error reading alert response: {}", e));
                self.record_failure(feed_id);
                return self.alerts_cache.clone();
            }
        };

        let feed = match transit_realtime::FeedMessage::decode(bytes.as_ref()) {
            Ok(f) => f,
            Err(e) => {
                self.log_error("alerts", &format!("Error decoding alert protobuf: {}", e));
                self.record_failure(feed_id);
                return self.alerts_cache.clone();
            }
        };

        let mut alert_objects = Vec::new();
        let mut seen_texts: HashSet<String> = HashSet::new();

        for entity in &feed.entity {
            let Some(ref alert_proto) = entity.alert else {
                continue;
            };

            let mut affected_routes: HashSet<String> = HashSet::new();
            for informed in &alert_proto.informed_entity {
                if let Some(ref route_id) = informed.route_id {
                    affected_routes.insert(route_id.clone());
                }
            }

            let relevant: HashSet<String> = affected_routes
                .intersection(routes)
                .cloned()
                .collect();

            if relevant.is_empty() {
                continue;
            }

            let priority = alert_proto
                .effect
                .map(extract_priority_from_effect)
                .unwrap_or(10);

            if let Some(ref header_text) = alert_proto.header_text {
                if let Some(translation) = header_text.translation.first() {
                    let clean_text: String = translation
                        .text
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");

                    if !seen_texts.contains(&clean_text) {
                        seen_texts.insert(clean_text.clone());
                        alert_objects.push(Alert {
                            text: clean_text,
                            affected_routes: relevant.clone(),
                            priority,
                            alert_id: entity.id.clone(),
                        });
                    }
                }
            }
        }

        self.alerts_cache = alert_objects.clone();
        self.record_success(feed_id);
        alert_objects
    }

    fn should_fetch(&self, feed_id: &str) -> bool {
        match self.backoff.get(feed_id) {
            Some(state) => Instant::now() >= state.retry_after,
            None => true,
        }
    }

    fn record_success(&mut self, feed_id: &str) {
        self.backoff.remove(feed_id);
    }

    fn record_failure(&mut self, feed_id: &str) {
        let failures = self
            .backoff
            .get(feed_id)
            .map(|s| s.failures + 1)
            .unwrap_or(1);
        // Exponential backoff: 15s, 30s, 60s, 120s, 240s, max 300s
        let backoff_secs = (15 * (1u64 << (failures - 1).min(5))).min(300);
        self.backoff.insert(
            feed_id.to_string(),
            BackoffState {
                failures,
                retry_after: Instant::now() + std::time::Duration::from_secs(backoff_secs),
            },
        );
    }

    fn log_error(&mut self, source: &str, msg: &str) {
        let now = Instant::now();
        let should_log = match self.last_error_log.get(source) {
            Some(last) => last.elapsed().as_secs() >= ERROR_LOG_INTERVAL_SECS,
            None => true,
        };
        if should_log {
            warn!("[MTA] {}", msg);
            self.last_error_log.insert(source.to_string(), now);
        }
    }

    fn cleanup_feed_cache(&mut self) {
        self.feed_cache
            .retain(|_, entry| entry.fetched_at.elapsed().as_secs() < CACHE_TTL_SECONDS);
    }
}

/// Fetch and parse a single GTFS-RT feed.
async fn fetch_single_feed(
    http: &Client,
    url: &str,
    stop_ids: &[String],
    routes: &HashSet<String>,
) -> Result<(Vec<Train>, u64), String> {
    let response = http
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Read error: {}", e))?;

    let feed = transit_realtime::FeedMessage::decode(bytes.as_ref())
        .map_err(|e| format!("Protobuf decode error: {}", e))?;

    let feed_timestamp = feed.header.timestamp.unwrap_or(0);
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();

    let stop_id_set: HashSet<&str> = stop_ids.iter().map(|s| s.as_str()).collect();
    let mut trains = Vec::new();

    for entity in &feed.entity {
        let Some(ref trip_update) = entity.trip_update else {
            continue;
        };
        let trip = &trip_update.trip;
        let route_id = trip.route_id.as_deref().unwrap_or("");
        if !routes.contains(route_id) {
            continue;
        }

        let is_express = detect_express(trip, route_id);

        for stop_time in &trip_update.stop_time_update {
            let stop_id = stop_time.stop_id.as_deref().unwrap_or("");
            if !stop_id_set.contains(stop_id) {
                continue;
            }

            // Get arrival time
            let arrival_ts = stop_time
                .arrival
                .as_ref()
                .and_then(|a| a.time)
                .unwrap_or(0) as f64;

            if arrival_ts <= now_secs {
                continue; // Already passed
            }

            let mins = ((arrival_ts - now_secs) / 60.0).max(0.0) as i32;

            // Direction from stop_id suffix
            let direction = if stop_id.ends_with('S') {
                Direction::Downtown
            } else {
                Direction::Uptown
            };

            // Destination: find the terminal station (highest stop_sequence)
            let destination = trip_update
                .stop_time_update
                .iter()
                .max_by_key(|st| st.stop_sequence.unwrap_or(0))
                .and_then(|st| st.stop_id.as_deref())
                .and_then(crate::mta::stations::station_name_for_stop_id)
                .unwrap_or("Unknown")
                .to_string();

            trains.push(Train {
                route: route_id.to_string(),
                destination,
                minutes: mins,
                is_express,
                arrival_timestamp: arrival_ts,
                direction,
                stop_id: stop_id.to_string(),
            });

            break; // Only first matching stop per trip
        }
    }

    debug!("Feed {} returned {} trains", url, trains.len());
    Ok((trains, feed_timestamp))
}

/// Detect if a train is running express service.
fn detect_express(
    trip: &transit_realtime::TripDescriptor,
    route_id: &str,
) -> bool {
    if !crate::display::colors::is_express_capable(route_id) {
        return false;
    }
    trip.trip_id
        .as_deref()
        .map(|id| id.ends_with('X'))
        .unwrap_or(false)
}

/// Remove duplicate trains (same route/destination within same minute).
fn deduplicate_trains(trains: Vec<Train>) -> Vec<Train> {
    let mut unique = Vec::new();
    let mut seen: HashSet<(String, String, i32)> = HashSet::new();

    for train in trains {
        let key = (
            train.route.clone(),
            train.destination.clone(),
            train.minutes,
        );
        if seen.insert(key) {
            unique.push(train);
        }
    }

    unique
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deduplicate_trains() {
        let trains = vec![
            Train {
                route: "1".into(),
                destination: "Uptown".into(),
                minutes: 3,
                is_express: false,
                arrival_timestamp: 1000.0,
                direction: Direction::Uptown,
                stop_id: "127N".into(),
            },
            Train {
                route: "1".into(),
                destination: "Uptown".into(),
                minutes: 3,
                is_express: false,
                arrival_timestamp: 1000.0,
                direction: Direction::Uptown,
                stop_id: "127N".into(),
            },
            Train {
                route: "2".into(),
                destination: "Downtown".into(),
                minutes: 5,
                is_express: false,
                arrival_timestamp: 1120.0,
                direction: Direction::Downtown,
                stop_id: "127S".into(),
            },
        ];
        let unique = deduplicate_trains(trains);
        assert_eq!(unique.len(), 2);
    }

    #[test]
    fn test_detect_express() {
        let express_trip = transit_realtime::TripDescriptor {
            trip_id: Some("123_X".into()),
            route_id: Some("2".into()),
            ..Default::default()
        };
        assert!(detect_express(&express_trip, "2"));

        let local_trip = transit_realtime::TripDescriptor {
            trip_id: Some("123".into()),
            route_id: Some("2".into()),
            ..Default::default()
        };
        assert!(!detect_express(&local_trip, "2"));

        // Non-express-capable route
        let route_1 = transit_realtime::TripDescriptor {
            trip_id: Some("123_X".into()),
            route_id: Some("1".into()),
            ..Default::default()
        };
        assert!(!detect_express(&route_1, "1"));
    }

    #[test]
    fn test_client_creation() {
        let client = MtaClient::new();
        assert!(client.feed_cache.is_empty());
        assert!(client.alerts_cache.is_empty());
        assert!(client.backoff.is_empty());
    }

    #[test]
    fn test_backoff_logic() {
        let mut client = MtaClient::new();
        assert!(client.should_fetch("test"));

        client.record_failure("test");
        // After failure, should be in backoff
        assert!(!client.should_fetch("test"));

        client.record_success("test");
        assert!(client.should_fetch("test"));
    }
}
