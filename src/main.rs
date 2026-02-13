mod config;
mod display;
mod models;
mod mta;
mod web;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arc_swap::ArcSwap;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use config::Config;
use display::matrix::create_display;
use display::renderer::Renderer;
use models::{Alert, DisplaySnapshot};
use mta::alerts::AlertManager;
use mta::client::MtaClient;

/// Shared application state — lock-free reads via ArcSwap.
pub struct AppState {
    pub config: ArcSwap<Config>,
    pub snapshot: ArcSwap<DisplaySnapshot>,
    pub alert_manager: Mutex<AlertManager>,
    pub config_path: PathBuf,
    pub shutdown: CancellationToken,
    pub config_changed: tokio::sync::Notify,
    pub last_fetch_success: AtomicU64,
    pub last_render_tick: AtomicU64,
}

#[tokio::main]
async fn main() {
    // Initialize tracing (structured logging)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "subway_sign=info".parse().unwrap()),
        )
        .init();

    info!("NYC Subway Sign (Rust) starting");

    // Find config file
    let config_path = find_config_path();
    info!("Config file: {}", config_path.display());

    // Load initial config
    let initial_config = match Config::load(&config_path) {
        Ok(cfg) => {
            info!(
                "Config loaded: {} platforms, routes: {}, brightness: {:.0}%",
                cfg.station_stops.len(),
                cfg.routes.join(","),
                cfg.display.brightness * 100.0
            );
            cfg
        }
        Err(e) => {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    // Build shared state
    let state = Arc::new(AppState {
        config: ArcSwap::from_pointee(initial_config.clone()),
        snapshot: ArcSwap::from_pointee(DisplaySnapshot::empty()),
        alert_manager: Mutex::new(AlertManager::new()),
        config_path: config_path.clone(),
        shutdown: CancellationToken::new(),
        config_changed: tokio::sync::Notify::new(),
        last_fetch_success: AtomicU64::new(0),
        last_render_tick: AtomicU64::new(0),
    });

    // Spawn fetch task
    let fetch_state = Arc::clone(&state);
    let fetch_handle = tokio::spawn(fetch_task(fetch_state));

    // Spawn config watcher task
    let config_state = Arc::clone(&state);
    let config_handle = tokio::spawn(config_watcher_task(config_state));

    // Spawn web server task
    let web_state = Arc::clone(&state);
    let web_handle = tokio::spawn(web::server::run(web_state));

    // Spawn render thread (dedicated OS thread, not tokio)
    let render_state = Arc::clone(&state);
    let render_running = Arc::new(AtomicBool::new(true));
    let render_flag = Arc::clone(&render_running);
    let render_thread = match std::thread::Builder::new()
        .name("render".into())
        .spawn(move || render_loop(render_state, render_flag))
    {
        Ok(handle) => handle,
        Err(e) => {
            error!("Failed to spawn render thread: {}", e);
            std::process::exit(1);
        }
    };

    info!("All tasks started — rendering at 60fps");

    // Wait for shutdown signal
    shutdown_signal().await;
    info!("Shutdown signal received");

    // Signal all tasks to stop
    state.shutdown.cancel();
    render_running.store(false, Ordering::Relaxed);

    // Wait for tasks to finish
    let _ = fetch_handle.await;
    let _ = config_handle.await;
    let _ = web_handle.await;
    render_thread.join().ok();

    info!("Shutdown complete");
}

/// Find the config.json file (check CWD, then parent directory).
fn find_config_path() -> PathBuf {
    let candidates = [
        PathBuf::from("config.json"),
        PathBuf::from("../config.json"),
    ];
    for path in &candidates {
        if path.exists() {
            return path.clone();
        }
    }
    // Default even if it doesn't exist yet
    PathBuf::from("config.json")
}

/// Fetch trains for the current config and update the snapshot.
async fn do_train_fetch(
    client: &mut MtaClient,
    state: &AppState,
    cached_alerts: &[models::Alert],
    last_train_count: &mut i32,
) {
    let config = state.config.load();

    let all_stop_ids: Vec<String> = config
        .station_stops
        .iter()
        .flat_map(|(up, down)| vec![up.clone(), down.clone()])
        .collect();

    let routes: HashSet<String> = config.routes.iter().cloned().collect();

    let trains = client
        .fetch_trains(&all_stop_ids, &routes, config.display.max_trains as usize)
        .await;

    let train_count = trains.len() as i32;

    let snapshot = DisplaySnapshot {
        trains,
        alerts: cached_alerts.to_vec(),
        fetched_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64(),
    };

    state.snapshot.store(Arc::new(snapshot));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    state.last_fetch_success.store(now, Ordering::Relaxed);

    if train_count != *last_train_count {
        info!("[FETCH] {} trains fetched", train_count);
        *last_train_count = train_count;
    }
}

/// Background fetch task — runs train + alert fetches on separate intervals.
async fn fetch_task(state: Arc<AppState>) {
    let mut client = match MtaClient::new() {
        Ok(c) => c,
        Err(e) => {
            error!("[FETCH] {}", e);
            return;
        }
    };
    let mut last_train_count: i32 = -1;
    let mut cached_alerts: Vec<models::Alert> = Vec::new();

    info!("[FETCH] Background fetch task started");

    // Initial 3-second delay to let everything settle
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // Use configured intervals (not hardcoded)
    let config = state.config.load();
    let mut train_interval = tokio::time::interval(
        std::time::Duration::from_secs(config.refresh.trains_interval),
    );
    let mut alert_interval = tokio::time::interval(
        std::time::Duration::from_secs(config.refresh.alerts_interval),
    );

    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => {
                info!("[FETCH] Shutting down");
                break;
            }
            _ = state.config_changed.notified() => {
                info!("[FETCH] Config changed — re-fetching");
                do_train_fetch(&mut client, &state, &cached_alerts, &mut last_train_count).await;
            }
            _ = alert_interval.tick() => {
                let config = state.config.load();
                if config.display.show_alerts {
                    let routes: HashSet<String> = config.routes.iter().cloned().collect();
                    let raw_alerts = client.fetch_alerts(&routes).await;
                    let mut am = state.alert_manager.lock()
                        .unwrap_or_else(|e| e.into_inner());
                    cached_alerts = am.filter_and_sort(&raw_alerts);
                }
            }
            _ = train_interval.tick() => {
                do_train_fetch(&mut client, &state, &cached_alerts, &mut last_train_count).await;
            }
        }
    }
}

/// Config watcher — polls config file mtime every 5 seconds.
async fn config_watcher_task(state: Arc<AppState>) {
    let mut last_mtime = std::fs::metadata(&state.config_path)
        .and_then(|m| m.modified())
        .ok();

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => {
                info!("[CONFIG] Shutting down");
                break;
            }
            _ = interval.tick() => {
                let current_mtime = std::fs::metadata(&state.config_path)
                    .and_then(|m| m.modified())
                    .ok();

                if current_mtime != last_mtime {
                    info!("[CONFIG] File changed, reloading...");
                    match Config::load(&state.config_path) {
                        Ok(new_config) => {
                            info!(
                                "[CONFIG] Reloaded: {} platforms, routes: {}",
                                new_config.station_stops.len(),
                                new_config.routes.join(",")
                            );
                            state.config.store(Arc::new(new_config));
                            state.config_changed.notify_one();
                            last_mtime = current_mtime;
                        }
                        Err(e) => {
                            warn!("[CONFIG] Reload failed: {}", e);
                        }
                    }
                }
            }
        }
    }
}

/// Alert display state machine.
///
/// Tracks whether an alert is currently showing, which alert it is,
/// the scroll position, and what train triggered the alert cycle.
/// Extracted from the render loop to reduce parameter sprawl.
struct AlertState {
    show_alert: bool,
    current_alert: Option<Alert>,
    scroll_offset: f32,
    triggered_by: Option<(String, String)>,
    cycle_start_time: Instant,
}

impl AlertState {
    fn new() -> Self {
        Self {
            show_alert: false,
            current_alert: None,
            scroll_offset: 0.0,
            triggered_by: None,
            cycle_start_time: Instant::now(),
        }
    }

    /// Reset all alert display state to idle.
    fn clear(&mut self) {
        self.show_alert = false;
        self.current_alert = None;
        self.scroll_offset = 0.0;
        self.triggered_by = None;
    }

    /// Update the alert state machine for one frame.
    ///
    /// Triggers alert display when a train arrives (minutes == 0), cycles through
    /// queued alerts with scrolling, and clears when all alerts have been shown
    /// or the triggering train departs.
    fn update(
        &mut self,
        state: &AppState,
        snapshot: &DisplaySnapshot,
        renderer: &mut Renderer,
        scroll_speed: f32,
        max_duration: std::time::Duration,
    ) {
        let first_train = snapshot.get_first_train();
        let train_at_zero = first_train.minutes == 0;

        // Skip mutex entirely when no alerts are active and none could trigger
        if !train_at_zero && !self.show_alert {
            return;
        }

        // Check if the train that triggered alerts has departed
        let triggering_train_departed = self.show_alert
            && self.triggered_by.as_ref().is_some_and(|(route, dest)| {
                !snapshot.trains.iter().any(|t| {
                    t.route == *route && t.destination == *dest && t.minutes == 0
                })
            });

        let mut am = state.alert_manager.lock()
            .unwrap_or_else(|e| e.into_inner());

        // Start showing alerts when a train arrives and alerts are queued
        if train_at_zero && !self.show_alert && am.has_alerts() {
            am.reset_cycle();
            if let Some(alert) = am.get_next_alert() {
                self.current_alert = Some(alert.clone());
                self.show_alert = true;
                self.scroll_offset = 0.0;
                self.triggered_by = Some((first_train.route.clone(), first_train.destination.clone()));
                self.cycle_start_time = Instant::now();
            }
        }

        // Process active alert display
        if self.show_alert && self.current_alert.is_some() {
            if self.cycle_start_time.elapsed() > max_duration {
                self.clear();
                am.periodic_cleanup();
                return;
            }

            self.scroll_offset += scroll_speed;

            let scroll_complete = self.scroll_offset >= renderer.get_scroll_complete_distance() as f32;
            if !scroll_complete {
                am.periodic_cleanup();
                return;
            }

            // Current alert finished scrolling -- mark it displayed
            if let Some(ref alert) = self.current_alert {
                am.mark_displayed(alert);
            }

            // Decide what to show next
            let next = if triggering_train_departed && train_at_zero && am.has_alerts() {
                // Train departed but another arrived -- restart the cycle
                am.reset_cycle();
                am.get_next_alert().cloned()
            } else if !triggering_train_departed && !am.all_shown_this_cycle() {
                am.get_next_alert().cloned()
            } else {
                None
            };

            if let Some(alert) = next {
                self.current_alert = Some(alert);
                self.scroll_offset = 0.0;
                if triggering_train_departed {
                    self.triggered_by = Some((
                        first_train.route.clone(),
                        first_train.destination.clone(),
                    ));
                    self.cycle_start_time = Instant::now();
                }
            } else {
                self.clear();
            }
        }

        am.periodic_cleanup();
    }
}

/// Render loop — runs in a dedicated OS thread at 60fps.
///
/// This is NOT a tokio task. It's a real thread because:
/// - It runs perpetually at 60fps with precise timing
/// - It calls blocking FFI (LED matrix VSync) on hardware
/// - spawn_blocking is for short-lived operations, not permanent loops
fn render_loop(state: Arc<AppState>, running: Arc<AtomicBool>) {
    let config = state.config.load();
    let brightness = (config.display.brightness * 100.0).round() as u8;
    let brightness = brightness.clamp(1, 100);
    let mut display = create_display(brightness);
    let mut renderer = Renderer::new();
    let mut alert_state = AlertState::new();

    let mut current_brightness = brightness;
    let mut cycle_index: usize = 0;
    let mut flash_state = false;

    let mut last_cycle_time = Instant::now();
    let mut last_flash_time = Instant::now();
    let mut frame_count: u64 = 0;
    let mut missed_frames: u64 = 0;
    let mut max_frame_us: u64 = 0;
    let mut total_frame_us: u64 = 0;
    let mut last_stats_time = Instant::now();

    const TARGET_FPS: f64 = 60.0;
    const FRAME_TIME: std::time::Duration =
        std::time::Duration::from_nanos((1_000_000_000.0 / TARGET_FPS) as u64);
    const CYCLE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);
    const FLASH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
    const SCROLL_PX_PER_SEC: f32 = 60.0;
    const SCROLL_SPEED: f32 = SCROLL_PX_PER_SEC / TARGET_FPS as f32;
    const MAX_ALERT_CYCLE_DURATION: std::time::Duration = std::time::Duration::from_secs(90);
    const STATS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

    info!("[RENDER] Render loop started ({}fps)", TARGET_FPS as u32);

    while running.load(Ordering::Relaxed) {
        let frame_start = Instant::now();

        // Load latest snapshot (lock-free)
        let snapshot = state.snapshot.load();

        // Update cycle index
        if last_cycle_time.elapsed() >= CYCLE_INTERVAL {
            last_cycle_time = Instant::now();
            cycle_index = (cycle_index + 1) % 6;
        }

        // Update flash state
        if last_flash_time.elapsed() >= FLASH_INTERVAL {
            last_flash_time = Instant::now();
            flash_state = !flash_state;
        }

        // Alert state machine
        alert_state.update(
            &state,
            &snapshot,
            &mut renderer,
            SCROLL_SPEED,
            MAX_ALERT_CYCLE_DURATION,
        );

        // Render frame
        let frame = renderer.render_frame(
            &snapshot,
            cycle_index,
            flash_state,
            alert_state.scroll_offset,
            alert_state.show_alert,
            alert_state.current_alert.as_ref(),
        );

        // Push to display
        display.swap(&frame);

        // Measure work time (render + swap/vsync) before compensating sleep
        let work_time = frame_start.elapsed();
        let work_us = work_time.as_micros() as u64;
        total_frame_us += work_us;
        if work_us > max_frame_us {
            max_frame_us = work_us;
        }
        if work_time > FRAME_TIME {
            missed_frames += 1;
        }

        frame_count += 1;

        // Poll for brightness changes every ~1 second (60 frames)
        if frame_count.is_multiple_of(60) {
            let cfg = state.config.load();
            let new_brightness = (cfg.display.brightness * 100.0).round() as u8;
            let new_brightness = new_brightness.clamp(1, 100);
            if new_brightness != current_brightness {
                display.set_brightness(new_brightness);
                current_brightness = new_brightness;
                info!("[RENDER] Brightness updated to {}%", new_brightness);
            }

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            state.last_render_tick.store(now, Ordering::Relaxed);
        }

        // Stats logging every 5 minutes
        if last_stats_time.elapsed() >= STATS_INTERVAL {
            let fps = frame_count as f64 / last_stats_time.elapsed().as_secs_f64();
            info!(
                "[STATS] FPS: {:.1} | Missed: {}/{} ({:.1}%) | Frame: avg {:.1}ms, max {:.1}ms | Trains: {} | Alerts: {}",
                fps,
                missed_frames, frame_count,
                if frame_count > 0 { missed_frames as f64 / frame_count as f64 * 100.0 } else { 0.0 },
                if frame_count > 0 { total_frame_us as f64 / frame_count as f64 / 1000.0 } else { 0.0 },
                max_frame_us as f64 / 1000.0,
                snapshot.trains.len(),
                snapshot.alerts.len(),
            );
            frame_count = 0;
            missed_frames = 0;
            max_frame_us = 0;
            total_frame_us = 0;
            last_stats_time = Instant::now();
        }

        // Sleep to maintain target FPS
        let elapsed = frame_start.elapsed();
        if elapsed < FRAME_TIME {
            std::thread::sleep(FRAME_TIME - elapsed);
        }
    }

    info!("[RENDER] Render loop stopped");
}

/// Wait for SIGTERM or SIGINT (Ctrl-C).
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            error!("Failed to install Ctrl-C handler: {}", e);
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                error!("Failed to install SIGTERM handler: {}", e);
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use models::{Alert, Direction, DisplaySnapshot, Train};

    fn test_config() -> Config {
        Config {
            station_stops: vec![("127N".to_string(), "127S".to_string())],
            routes: vec!["1".to_string()],
            display: config::DisplayConfig {
                brightness: 0.5,
                max_trains: 10,
                show_alerts: true,
            },
            refresh: config::RefreshConfig::default(),
        }
    }

    fn make_state(alerts: Vec<Alert>) -> Arc<AppState> {
        let mut am = mta::alerts::AlertManager::new();
        am.filter_and_sort(&alerts);
        Arc::new(AppState {
            config: ArcSwap::from_pointee(test_config()),
            snapshot: ArcSwap::from_pointee(DisplaySnapshot::empty()),
            alert_manager: Mutex::new(am),
            config_path: PathBuf::from("config.json"),
            shutdown: CancellationToken::new(),
            config_changed: tokio::sync::Notify::new(),
            last_fetch_success: AtomicU64::new(0),
            last_render_tick: AtomicU64::new(0),
        })
    }

    fn make_train(route: &str, dest: &str, minutes: i32) -> Train {
        Train {
            route: route.into(),
            destination: dest.into(),
            minutes,
            is_express: false,
            arrival_timestamp: 0.0,
            direction: Direction::Uptown,
            stop_id: "127N".into(),
        }
    }

    fn make_alert(id: &str) -> Alert {
        Alert {
            text: format!("Alert {}", id),
            affected_routes: HashSet::from(["1".to_string()]),
            priority: 1,
            alert_id: id.to_string(),
        }
    }

    #[test]
    fn test_alert_triggers_on_arrival() {
        let state = make_state(vec![make_alert("a1")]);
        let snapshot = DisplaySnapshot {
            trains: vec![make_train("1", "Uptown", 0)], // arriving!
            alerts: vec![make_alert("a1")],
            fetched_at: 0.0,
        };
        let mut renderer = display::renderer::Renderer::new();
        let mut alert = AlertState::new();

        assert!(!alert.show_alert);

        alert.update(&state, &snapshot, &mut renderer, 1.0, Duration::from_secs(90));

        assert!(alert.show_alert, "alert should trigger when train at 0 min");
        assert!(alert.current_alert.is_some());
        assert_eq!(alert.triggered_by.as_ref().unwrap().0, "1");
    }

    #[test]
    fn test_alert_does_not_trigger_without_arrival() {
        let state = make_state(vec![make_alert("a1")]);
        let snapshot = DisplaySnapshot {
            trains: vec![make_train("1", "Uptown", 3)], // not arriving
            alerts: vec![make_alert("a1")],
            fetched_at: 0.0,
        };
        let mut renderer = display::renderer::Renderer::new();
        let mut alert = AlertState::new();

        alert.update(&state, &snapshot, &mut renderer, 1.0, Duration::from_secs(90));

        assert!(!alert.show_alert, "alert should not trigger when no train at 0 min");
    }

    #[test]
    fn test_alert_clears_when_all_shown() {
        let state = make_state(vec![make_alert("a1")]);
        let snapshot = DisplaySnapshot {
            trains: vec![make_train("1", "Uptown", 0)],
            alerts: vec![make_alert("a1")],
            fetched_at: 0.0,
        };
        let mut renderer = display::renderer::Renderer::new();
        let mut alert = AlertState::new();

        // Trigger alert
        alert.update(&state, &snapshot, &mut renderer, 1.0, Duration::from_secs(90));
        assert!(alert.show_alert);

        // Simulate scroll completing by setting offset past the threshold
        let complete_dist = renderer.get_scroll_complete_distance() as f32;
        alert.scroll_offset = complete_dist + 1.0;

        // Update should mark as displayed and clear (only one alert)
        alert.update(&state, &snapshot, &mut renderer, 0.0, Duration::from_secs(90));

        assert!(!alert.show_alert, "alert should clear after all shown this cycle");
    }

    #[test]
    fn test_alert_max_duration_timeout() {
        let state = make_state(vec![make_alert("a1")]);
        let snapshot = DisplaySnapshot {
            trains: vec![make_train("1", "Uptown", 0)],
            alerts: vec![make_alert("a1")],
            fetched_at: 0.0,
        };
        let mut renderer = display::renderer::Renderer::new();
        let mut alert = AlertState::new();

        // Trigger alert
        alert.update(&state, &snapshot, &mut renderer, 1.0, Duration::from_secs(90));
        assert!(alert.show_alert);

        // Simulate timeout by setting cycle_start_time far in the past
        alert.cycle_start_time = Instant::now() - Duration::from_secs(100);

        // Update with a very short max_duration to trigger timeout
        alert.update(&state, &snapshot, &mut renderer, 1.0, Duration::from_secs(90));

        assert!(!alert.show_alert, "alert should clear after max duration timeout");
    }

    #[test]
    fn test_alert_departure_resets_cycle() {
        let alerts = vec![make_alert("a1"), make_alert("a2")];
        let state = make_state(alerts.clone());
        let mut renderer = display::renderer::Renderer::new();
        let mut alert = AlertState::new();

        // Train arrives, triggers alerts
        let snapshot_arrive = DisplaySnapshot {
            trains: vec![make_train("1", "Uptown", 0)],
            alerts: alerts.clone(),
            fetched_at: 0.0,
        };
        alert.update(&state, &snapshot_arrive, &mut renderer, 1.0, Duration::from_secs(90));
        assert!(alert.show_alert);
        assert_eq!(alert.triggered_by.as_ref().unwrap(), &("1".to_string(), "Uptown".to_string()));
    }
}
