use serde::Deserialize;
use std::path::Path;

use crate::models::{stop_ids_to_station_stops, StationStop};
use crate::mta::stations;

/// Top-level configuration file structure.
#[derive(Debug, Deserialize)]
struct RawConfig {
    station: RawStationConfig,
    display: DisplayConfig,
    #[serde(default)]
    refresh: RefreshConfig,
}

/// Raw station section — supports all 3 formats via Option fields.
#[derive(Debug, Deserialize)]
struct RawStationConfig {
    station_name: Option<String>,
    routes: Option<Vec<String>>,
    stations: Option<Vec<RawStationPair>>,
    uptown_stop_id: Option<String>,
    downtown_stop_id: Option<String>,
}

/// Explicit platform pair (format 2).
#[derive(Debug, Deserialize)]
struct RawStationPair {
    uptown: String,
    downtown: String,
}

/// Display settings.
#[derive(Debug, Clone, Deserialize)]
pub struct DisplayConfig {
    pub brightness: f64,
    pub max_trains: u32,
    pub show_alerts: bool,
}

/// Refresh intervals (optional in config file).
#[derive(Debug, Clone, Deserialize)]
pub struct RefreshConfig {
    #[serde(default = "default_trains_interval")]
    pub trains_interval: u64,
    #[serde(default = "default_alerts_interval")]
    pub alerts_interval: u64,
}

fn default_trains_interval() -> u64 {
    20
}
fn default_alerts_interval() -> u64 {
    60
}

impl Default for RefreshConfig {
    fn default() -> Self {
        RefreshConfig {
            trains_interval: default_trains_interval(),
            alerts_interval: default_alerts_interval(),
        }
    }
}

/// Resolved application configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub station_stops: Vec<StationStop>,
    pub routes: Vec<String>,
    pub display: DisplayConfig,
    pub refresh: RefreshConfig,
}

impl Config {
    /// Load configuration from a JSON file.
    ///
    /// Supports three station formats:
    /// 1. `station_name`: Auto-detect platforms via fuzzy matching
    /// 2. `stations`: Explicit list of `{uptown, downtown}` pairs
    /// 3. `uptown_stop_id`/`downtown_stop_id`: Legacy single platform
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| ConfigError::Io(e.to_string()))?;
        Self::from_json(&contents)
    }

    /// Parse config from a JSON string (useful for testing).
    pub fn from_json(json: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig =
            serde_json::from_str(json).map_err(|e| ConfigError::Parse(e.to_string()))?;

        let station = raw.station;

        // Resolve station stops and routes based on format
        let (stops, routes) = if let Some(ref station_name) = station.station_name {
            if !station_name.is_empty() {
                Self::resolve_station_name(station_name, &station.routes)?
            } else {
                return Err(ConfigError::Validation(
                    "station_name is empty".to_string(),
                ));
            }
        } else if let Some(ref station_pairs) = station.stations {
            let stops: Vec<StationStop> = station_pairs
                .iter()
                .map(|p| (p.uptown.clone(), p.downtown.clone()))
                .collect();
            let routes = station.routes.unwrap_or_default();
            (stops, routes)
        } else if let (Some(ref up), Some(ref down)) =
            (&station.uptown_stop_id, &station.downtown_stop_id)
        {
            let stops = vec![(up.clone(), down.clone())];
            let routes = station.routes.unwrap_or_default();
            (stops, routes)
        } else {
            return Err(ConfigError::Validation(
                "Config missing station configuration \
                 (station_name, stations, or uptown_stop_id/downtown_stop_id)"
                    .to_string(),
            ));
        };

        let config = Config {
            station_stops: stops,
            routes,
            display: raw.display,
            refresh: raw.refresh,
        };

        config.validate()?;
        Ok(config)
    }

    /// Resolve a station name to stop IDs and routes via the station database.
    fn resolve_station_name(
        station_name: &str,
        explicit_routes: &Option<Vec<String>>,
    ) -> Result<(Vec<StationStop>, Vec<String>), ConfigError> {
        let stop_ids = stations::get_stop_ids_for_station(station_name);
        if stop_ids.is_empty() {
            return Err(ConfigError::StationNotFound(station_name.to_string()));
        }

        let stops = stop_ids_to_station_stops(&stop_ids);

        // Use explicit routes if provided, otherwise auto-detect from station DB
        let routes = if let Some(r) = explicit_routes {
            if !r.is_empty() {
                r.clone()
            } else {
                stations::get_routes_for_station(station_name)
            }
        } else {
            stations::get_routes_for_station(station_name)
        };

        Ok((stops, routes))
    }

    /// Validate config values are within acceptable ranges.
    fn validate(&self) -> Result<(), ConfigError> {
        if !(0.0..=1.0).contains(&self.display.brightness) {
            return Err(ConfigError::Validation(format!(
                "brightness must be 0.0-1.0, got {}",
                self.display.brightness
            )));
        }
        if self.display.max_trains < 1 || self.display.max_trains > 20 {
            return Err(ConfigError::Validation(format!(
                "max_trains must be 1-20, got {}",
                self.display.max_trains
            )));
        }
        if self.routes.is_empty() {
            return Err(ConfigError::Validation(
                "routes cannot be empty".to_string(),
            ));
        }
        if self.station_stops.is_empty() {
            return Err(ConfigError::Validation(
                "station_stops cannot be empty".to_string(),
            ));
        }
        Ok(())
    }
}

/// Configuration errors.
#[derive(Debug)]
pub enum ConfigError {
    Io(String),
    Parse(String),
    Validation(String),
    StationNotFound(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(msg) => write!(f, "Config I/O error: {}", msg),
            ConfigError::Parse(msg) => write!(f, "Config parse error: {}", msg),
            ConfigError::Validation(msg) => write!(f, "Config validation error: {}", msg),
            ConfigError::StationNotFound(name) => {
                write!(f, "Station '{}' not found in database", name)
            }
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_station_name_format() {
        let json = r#"{
            "station": {
                "station_name": "Times Sq-42 St",
                "routes": ["1", "2", "3", "7", "N", "Q", "R", "W", "S"]
            },
            "display": {
                "brightness": 0.61,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let config = Config::from_json(json).expect("should parse station_name format");
        assert!(!config.station_stops.is_empty());
        assert!(config.routes.contains(&"1".to_string()));
        assert_eq!(config.display.max_trains, 7);
    }

    #[test]
    fn test_load_stations_array_format() {
        let json = r#"{
            "station": {
                "stations": [
                    {"uptown": "127N", "downtown": "127S"},
                    {"uptown": "725N", "downtown": "725S"}
                ],
                "routes": ["1", "2", "3"]
            },
            "display": {
                "brightness": 0.5,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let config = Config::from_json(json).expect("should parse stations array format");
        assert_eq!(config.station_stops.len(), 2);
        assert_eq!(config.station_stops[0].0, "127N");
    }

    #[test]
    fn test_load_legacy_format() {
        let json = r#"{
            "station": {
                "uptown_stop_id": "127N",
                "downtown_stop_id": "127S",
                "routes": ["1", "2", "3"]
            },
            "display": {
                "brightness": 0.3,
                "max_trains": 5,
                "show_alerts": false
            }
        }"#;
        let config = Config::from_json(json).expect("should parse legacy format");
        assert_eq!(config.station_stops.len(), 1);
        assert_eq!(config.station_stops[0], ("127N".to_string(), "127S".to_string()));
    }

    #[test]
    fn test_refresh_defaults() {
        let json = r#"{
            "station": {
                "station_name": "Times Sq-42 St",
                "routes": ["1"]
            },
            "display": {
                "brightness": 0.5,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let config = Config::from_json(json).unwrap();
        assert_eq!(config.refresh.trains_interval, 20);
        assert_eq!(config.refresh.alerts_interval, 60);
    }

    #[test]
    fn test_refresh_custom() {
        let json = r#"{
            "station": {
                "station_name": "Times Sq-42 St",
                "routes": ["1"]
            },
            "display": {
                "brightness": 0.5,
                "max_trains": 7,
                "show_alerts": true
            },
            "refresh": {
                "trains_interval": 30,
                "alerts_interval": 120
            }
        }"#;
        let config = Config::from_json(json).unwrap();
        assert_eq!(config.refresh.trains_interval, 30);
        assert_eq!(config.refresh.alerts_interval, 120);
    }

    #[test]
    fn test_validation_brightness_too_high() {
        let json = r#"{
            "station": {
                "station_name": "Times Sq-42 St",
                "routes": ["1"]
            },
            "display": {
                "brightness": 1.5,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let err = Config::from_json(json).unwrap_err();
        assert!(err.to_string().contains("brightness"));
    }

    #[test]
    fn test_validation_max_trains_zero() {
        let json = r#"{
            "station": {
                "station_name": "Times Sq-42 St",
                "routes": ["1"]
            },
            "display": {
                "brightness": 0.5,
                "max_trains": 0,
                "show_alerts": true
            }
        }"#;
        let err = Config::from_json(json).unwrap_err();
        assert!(err.to_string().contains("max_trains"));
    }

    #[test]
    fn test_validation_empty_routes() {
        let json = r#"{
            "station": {
                "stations": [{"uptown": "127N", "downtown": "127S"}],
                "routes": []
            },
            "display": {
                "brightness": 0.5,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let err = Config::from_json(json).unwrap_err();
        assert!(err.to_string().contains("routes"));
    }

    #[test]
    fn test_missing_station_config() {
        let json = r#"{
            "station": {},
            "display": {
                "brightness": 0.5,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let err = Config::from_json(json).unwrap_err();
        assert!(err.to_string().contains("missing station"));
    }

    #[test]
    fn test_unknown_station_name() {
        let json = r#"{
            "station": {
                "station_name": "Totally Fake Station XYZ 999",
                "routes": ["1"]
            },
            "display": {
                "brightness": 0.5,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let err = Config::from_json(json).unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn test_auto_detect_routes() {
        // No explicit routes — should auto-detect from station DB
        let json = r#"{
            "station": {
                "station_name": "Times Sq-42 St"
            },
            "display": {
                "brightness": 0.5,
                "max_trains": 7,
                "show_alerts": true
            }
        }"#;
        let config = Config::from_json(json).unwrap();
        assert!(!config.routes.is_empty(), "should auto-detect routes");
    }
}
