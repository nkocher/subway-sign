use std::collections::HashSet;
use std::sync::OnceLock;

/// Direction a train is traveling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Uptown,   // N
    Downtown, // S
}

/// A single train arrival.
#[derive(Debug, Clone)]
pub struct Train {
    pub route: String,
    pub destination: String,
    pub minutes: i32,
    pub is_express: bool,
    pub arrival_timestamp: f64,
    pub direction: Direction,
    pub stop_id: String,
}

impl Train {
    /// Create an empty placeholder train.
    pub fn empty() -> Self {
        Train {
            route: String::new(),
            destination: "---".to_string(),
            minutes: 999,
            is_express: false,
            arrival_timestamp: 0.0,
            direction: Direction::Uptown,
            stop_id: String::new(),
        }
    }
}

/// A service alert message.
#[derive(Debug, Clone)]
pub struct Alert {
    pub text: String,
    pub affected_routes: HashSet<String>,
    pub priority: i32,
    pub alert_id: String,
}

/// Complete immutable snapshot of all data needed to render a frame.
///
/// Passed from the fetch task to the render thread via ArcSwap.
/// Being fully immutable eliminates data races.
#[derive(Debug, Clone)]
pub struct DisplaySnapshot {
    pub trains: Vec<Train>,
    pub alerts: Vec<Alert>,
    pub fetched_at: f64,
}

impl DisplaySnapshot {
    /// Create an empty snapshot for initialization.
    pub fn empty() -> Self {
        DisplaySnapshot {
            trains: Vec::new(),
            alerts: Vec::new(),
            fetched_at: 0.0,
        }
    }

    /// Get the next arriving train (any direction).
    pub fn get_first_train(&self) -> &Train {
        static EMPTY_TRAIN: OnceLock<Train> = OnceLock::new();
        self.trains.first().unwrap_or_else(||
            EMPTY_TRAIN.get_or_init(Train::empty)
        )
    }

    /// Get trains #2 through #(count+1) for bottom row cycling.
    /// Skips first train (shown on top row), takes next `count` trains.
    pub fn get_cycling_trains(&self, count: usize) -> Vec<Train> {
        let mut result: Vec<Train> = self
            .trains
            .iter()
            .skip(1)
            .take(count)
            .cloned()
            .collect();

        // Pad with empty trains if needed
        while result.len() < count {
            result.push(Train::empty());
        }
        result
    }
}

/// A (uptown_stop_id, downtown_stop_id) platform pair.
pub type StationStop = (String, String);

/// Convert a list of stop IDs to (uptown, downtown) tuples.
///
/// Stop IDs end with N (northbound/uptown) or S (southbound/downtown).
/// Groups by base ID and pairs them up.
pub fn stop_ids_to_station_stops(stop_ids: &[String]) -> Vec<StationStop> {
    use std::collections::HashMap;

    let mut platforms: HashMap<&str, (Option<&str>, Option<&str>)> = HashMap::new();

    for stop_id in stop_ids {
        if stop_id.len() < 2 {
            continue;
        }
        let (base_id, dir) = stop_id.split_at(stop_id.len() - 1);
        let entry = platforms.entry(base_id).or_insert((None, None));
        match dir {
            "N" => entry.0 = Some(stop_id),
            "S" => entry.1 = Some(stop_id),
            _ => {}
        }
    }

    platforms
        .into_values()
        .filter_map(|(n, s)| match (n, s) {
            (Some(n), Some(s)) => Some((n.to_string(), s.to_string())),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_snapshot_empty() {
        let snap = DisplaySnapshot::empty();
        assert!(snap.trains.is_empty());
        assert!(snap.alerts.is_empty());
        assert_eq!(snap.fetched_at, 0.0);
    }

    #[test]
    fn test_get_first_train_empty() {
        let snap = DisplaySnapshot::empty();
        let first = snap.get_first_train();
        assert_eq!(first.minutes, 999);
        assert_eq!(first.destination, "---");
    }

    #[test]
    fn test_get_first_train_with_data() {
        let snap = DisplaySnapshot {
            trains: vec![
                Train {
                    route: "1".into(),
                    destination: "Van Cortlandt Park".into(),
                    minutes: 2,
                    is_express: false,
                    arrival_timestamp: 1000.0,
                    direction: Direction::Uptown,
                    stop_id: "127N".into(),
                },
                Train {
                    route: "2".into(),
                    destination: "Wakefield".into(),
                    minutes: 5,
                    is_express: true,
                    arrival_timestamp: 1180.0,
                    direction: Direction::Uptown,
                    stop_id: "127N".into(),
                },
            ],
            alerts: Vec::new(),
            fetched_at: 999.0,
        };
        let first = snap.get_first_train();
        assert_eq!(first.route, "1");
        assert_eq!(first.minutes, 2);
    }

    #[test]
    fn test_get_cycling_trains_padding() {
        let snap = DisplaySnapshot {
            trains: vec![Train {
                route: "1".into(),
                destination: "Test".into(),
                minutes: 1,
                is_express: false,
                arrival_timestamp: 0.0,
                direction: Direction::Uptown,
                stop_id: "".into(),
            }],
            alerts: Vec::new(),
            fetched_at: 0.0,
        };
        // Only 1 train total, so cycling skips it → all padding
        let cycling = snap.get_cycling_trains(6);
        assert_eq!(cycling.len(), 6);
        assert_eq!(cycling[0].minutes, 999); // all empty
    }

    #[test]
    fn test_get_cycling_trains_with_data() {
        let mut trains = Vec::new();
        for i in 0..8 {
            trains.push(Train {
                route: format!("{}", i + 1),
                destination: format!("Dest {}", i),
                minutes: i,
                is_express: false,
                arrival_timestamp: 0.0,
                direction: Direction::Uptown,
                stop_id: "".into(),
            });
        }
        let snap = DisplaySnapshot {
            trains,
            alerts: Vec::new(),
            fetched_at: 0.0,
        };
        let cycling = snap.get_cycling_trains(6);
        assert_eq!(cycling.len(), 6);
        assert_eq!(cycling[0].route, "2"); // skipped first train
        assert_eq!(cycling[5].route, "7");
    }

    #[test]
    fn test_stop_ids_to_station_stops() {
        let ids: Vec<String> = vec![
            "127N".into(),
            "127S".into(),
            "725N".into(),
            "725S".into(),
        ];
        let stops = stop_ids_to_station_stops(&ids);
        assert_eq!(stops.len(), 2);
        // Check both platforms exist (order may vary from HashMap)
        let bases: Vec<String> = stops
            .iter()
            .map(|(n, _)| n[..n.len() - 1].to_string())
            .collect();
        assert!(bases.contains(&"127".to_string()));
        assert!(bases.contains(&"725".to_string()));
    }

    #[test]
    fn test_stop_ids_unpaired_ignored() {
        let ids: Vec<String> = vec!["127N".into()]; // no matching S
        let stops = stop_ids_to_station_stops(&ids);
        assert!(stops.is_empty());
    }
}
