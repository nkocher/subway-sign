use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tracing::{info, warn};

use crate::config::Config;
use crate::mta::stations;
use crate::AppState;

#[derive(Deserialize)]
pub struct StationSearchParams {
    search: Option<String>,
    route: Option<String>,
    #[allow(dead_code)]
    borough: Option<String>,
    multi_platform_only: Option<String>,
}

/// GET /api/config — return current config as JSON.
pub async fn get_config(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.load();
    let config_json = config_to_json(&config);
    let last_modified = config_file_mtime(&state);

    Json(json!({
        "success": true,
        "config": config_json,
        "last_modified": last_modified,
    }))
}

/// POST /api/config — validate and save new config.
pub async fn update_config(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let validated_json = serde_json::to_string_pretty(&body).unwrap_or_default();

    let new_config = match Config::from_json(&validated_json) {
        Ok(cfg) => cfg,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "success": false, "message": format!("Invalid config: {}", e) })),
            );
        }
    };

    // Atomic write via spawn_blocking (sync fs ops: rename, sync_all)
    let write_result = tokio::task::spawn_blocking({
        let path = state.config_path.clone();
        move || crate::config::atomic_write_config(&path, &validated_json)
    })
    .await;

    match write_result {
        Ok(Ok(_)) => {
            info!("[WEB] Config saved (atomic)");
            state.config.store(Arc::new(new_config));
            state.config_changed.notify_one();
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "Configuration saved and applied."
                })),
            )
        }
        Ok(Err(e)) => {
            warn!("[WEB] Failed to write config: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "message": format!("Failed to save config: {}", e) })),
            )
        }
        Err(e) => {
            warn!("[WEB] Config write task failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "success": false, "message": format!("Config write failed: {}", e) })),
            )
        }
    }
}

/// GET /api/status — service status, current station, routes.
pub async fn get_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.load();
    let snapshot = state.snapshot.load();
    let station = determine_station_name(&config);
    let last_update = config_file_mtime(&state);

    Json(json!({
        "success": true,
        "status": {
            "service": "Running",
            "status_class": "running",
            "station": station,
            "routes": config.routes,
            "brightness": config.display.brightness,
            "max_trains": config.display.max_trains,
            "last_update": last_update,
            "uptime": format!("trains: {}, alerts: {}", snapshot.trains.len(), snapshot.alerts.len()),
        }
    }))
}

/// GET /api/debug/snapshot — dump current train + alert data for verification.
pub async fn get_debug_snapshot(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let snapshot = state.snapshot.load();
    let trains: Vec<serde_json::Value> = snapshot
        .trains
        .iter()
        .map(|t| {
            json!({
                "route": t.route,
                "destination": t.destination,
                "minutes": t.minutes,
                "direction": format!("{:?}", t.direction),
                "is_express": t.is_express,
                "stop_id": t.stop_id,
            })
        })
        .collect();
    let alerts: Vec<serde_json::Value> = snapshot
        .alerts
        .iter()
        .map(|a| {
            json!({
                "text": a.text,
                "affected_routes": a.affected_routes,
                "priority": a.priority,
            })
        })
        .collect();
    Json(json!({
        "trains": trains,
        "alerts": alerts,
        "fetched_at": snapshot.fetched_at,
        "train_count": trains.len(),
        "alert_count": alerts.len(),
    }))
}

/// GET /api/stations/complete — search/filter complete station database.
pub async fn get_complete_stations(
    Query(params): Query<StationSearchParams>,
) -> impl IntoResponse {
    let all_stations = stations::get_station_database();

    let search = params.search.unwrap_or_default().to_lowercase();
    let route_filter = params.route.unwrap_or_default();
    let multi_only = params
        .multi_platform_only
        .unwrap_or_default()
        .to_lowercase()
        == "true";

    let database_total = all_stations.len();

    let filtered: Vec<serde_json::Value> = all_stations
        .iter()
        .filter(|s| {
            (search.is_empty() || s.name.to_lowercase().contains(&search))
                && (route_filter.is_empty() || s.routes.contains(&route_filter))
                && (!multi_only || s.platform_count > 1)
        })
        .map(|s| {
            json!({
                "name": s.name,
                "routes": s.routes,
                "stop_ids": s.stop_ids,
                "platform_count": s.platform_count,
                "borough": s.borough,
            })
        })
        .collect();

    let total = filtered.len();

    Json(json!({
        "success": true,
        "stations": filtered,
        "total": total,
        "database_total": database_total,
    }))
}

/// GET /api/stations/lookup/:station_name — look up stop IDs for a station.
pub async fn lookup_station(Path(station_name): Path<String>) -> impl IntoResponse {
    let stop_ids = stations::get_stop_ids_for_station(&station_name);

    if stop_ids.is_empty() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "success": false,
                "error": format!("Station '{}' not found in database", station_name),
                "suggestion": "Try searching with /api/stations/complete?search=<partial_name>"
            })),
        );
    }

    let routes = stations::get_routes_for_station(&station_name);
    let platform_count = stop_ids.len() / 2;

    (
        StatusCode::OK,
        Json(json!({
            "success": true,
            "station_name": station_name,
            "stop_ids": stop_ids,
            "platform_count": platform_count,
            "routes": routes,
        })),
    )
}

/// POST /api/restart — trigger config reload (not process restart).
pub async fn restart(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    info!("[WEB] Restart requested — reloading config");

    match Config::load(&state.config_path) {
        Ok(new_config) => {
            state.config.store(Arc::new(new_config));
            state.config_changed.notify_one();
            Json(json!({
                "success": true,
                "message": "Configuration reloaded successfully"
            }))
        }
        Err(e) => Json(json!({
            "success": false,
            "message": format!("Reload failed: {}", e)
        })),
    }
}

/// GET /api/healthz — liveness check with fetch and render heartbeats.
pub async fn healthz(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let config = state.config.load();
    let fetch_age = now - state.last_fetch_success.load(std::sync::atomic::Ordering::Relaxed);
    let render_age = now - state.last_render_tick.load(std::sync::atomic::Ordering::Relaxed);

    let fetch_stale_threshold = config.refresh.trains_interval * 3;
    let render_stale_threshold = 10;

    let fetch_stale = fetch_age > fetch_stale_threshold;
    let render_stale = render_age > render_stale_threshold;
    let ok = !fetch_stale && !render_stale;

    let reason = if fetch_stale && render_stale {
        Some(format!("fetch stale {}s, render stale {}s", fetch_age, render_age))
    } else if fetch_stale {
        Some(format!("fetch stale {}s", fetch_age))
    } else if render_stale {
        Some(format!("render stale {}s", render_age))
    } else {
        None
    };

    Json(json!({
        "ok": ok,
        "age_seconds": fetch_age,
        "render_age_seconds": render_age,
        "degraded": fetch_stale && !render_stale,
        "reason": reason,
    }))
}

// -- Helper functions --

/// Get config file mtime as RFC 3339 string (for last_modified / last_update).
fn config_file_mtime(state: &AppState) -> Option<String> {
    std::fs::metadata(&state.config_path)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| {
            let datetime: chrono::DateTime<chrono::Local> = t.into();
            datetime.to_rfc3339()
        })
}

fn config_to_json(config: &Config) -> serde_json::Value {
    let station = if config.station_stops.len() == 1 {
        json!({
            "uptown_stop_id": config.station_stops[0].0,
            "downtown_stop_id": config.station_stops[0].1,
            "routes": config.routes,
        })
    } else {
        let stations_arr: Vec<serde_json::Value> = config
            .station_stops
            .iter()
            .map(|(up, down)| json!({"uptown": up, "downtown": down}))
            .collect();
        json!({
            "stations": stations_arr,
            "routes": config.routes,
        })
    };

    json!({
        "station": station,
        "display": {
            "brightness": config.display.brightness,
            "max_trains": config.display.max_trains,
            "show_alerts": config.display.show_alerts,
        },
        "refresh": {
            "trains_interval": config.refresh.trains_interval,
            "alerts_interval": config.refresh.alerts_interval,
        },
    })
}

fn determine_station_name(config: &Config) -> String {
    config
        .station_stops
        .first()
        .and_then(|(up, _)| {
            let base_id = up.trim_end_matches(['N', 'S']);
            stations::get_station_database()
                .iter()
                .find(|s| {
                    s.stop_ids
                        .iter()
                        .any(|sid| sid.trim_end_matches(['N', 'S']) == base_id)
                })
                .map(|s| s.name.clone())
        })
        .unwrap_or_else(|| {
            if config.station_stops.is_empty() {
                "Not configured".into()
            } else {
                format!("{} platforms configured", config.station_stops.len())
            }
        })
}
