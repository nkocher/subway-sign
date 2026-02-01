"""
Main orchestration for Subway Sign V3.

This is the heart of the application - a simple, clean main loop that:
1. Fetches data in the background (non-blocking)
2. Renders frames at 30fps
3. Hot-reloads config when file changes
4. Never crashes (graceful error handling)

Architecture: Queue-based producer-consumer pattern
- Background thread: Fetches data, builds immutable snapshots, pushes to queue
- Main thread: Pulls snapshots (non-blocking), renders, displays
- Config reload: Watches config.json for changes, updates both threads
"""
import time
import queue
import threading
import os
import sys
import signal
from pathlib import Path
from dataclasses import dataclass
from typing import Optional

# Add src to path for imports
sys.path.insert(0, str(Path(__file__).parent))

from mta import Config, DisplaySnapshot, MTAClient
from mta.alert_manager import AlertManager
from display import Renderer


class ConfigHolder:
    """
    Thread-safe config holder with hot-reload support.

    Watches config.json for changes and updates the config atomically.
    Both main thread and fetch thread read from this holder.
    """

    def __init__(self, config_path: str):
        self._config_path = config_path
        self._config: Optional[Config] = None
        self._mtime: float = 0
        self._lock = threading.Lock()
        # Initial load - must succeed
        self._config = Config.load(config_path)
        self._mtime = os.path.getmtime(config_path)

    def _reload(self) -> bool:
        """Reload config from file. Returns True if config changed."""
        try:
            current_mtime = os.path.getmtime(self._config_path)
            if current_mtime > self._mtime:
                new_config = Config.load(self._config_path)
                with self._lock:
                    self._config = new_config
                    self._mtime = current_mtime
                return True
            return False
        except Exception as e:
            print(f"[CONFIG] Error reloading config: {e}")
            return False

    def check_and_reload(self) -> bool:
        """Check if config file changed and reload if so. Returns True if reloaded."""
        try:
            current_mtime = os.path.getmtime(self._config_path)
            if current_mtime > self._mtime:
                print("[CONFIG] Configuration file changed, reloading...")
                if self._reload():
                    print("[CONFIG] ✓ Configuration reloaded successfully")
                    return True
            return False
        except Exception as e:
            print(f"[CONFIG] Error checking config: {e}")
            return False

    @property
    def config(self) -> Config:
        """Get current config (thread-safe read)."""
        with self._lock:
            return self._config

# Try to import matrix library, fall back to mock for dev
try:
    from rgbmatrix import RGBMatrix, RGBMatrixOptions
    MOCK_DISPLAY = False
except ImportError:
    print("⚠️  rpi-rgb-led-matrix not available, using mock display")
    MOCK_DISPLAY = True


class MockMatrix:
    """Mock matrix for development without hardware."""

    def __init__(self, options):
        self.width = 192
        self.height = 32
        self.brightness = options.brightness
        print(f"[MOCK] Matrix initialized: {self.width}x{self.height}, brightness={self.brightness}")

    def CreateFrameCanvas(self):
        return self

    def SetImage(self, img):
        # In real implementation, could save to file for debugging
        pass

    def SwapOnVSync(self, canvas=None):
        return self


def init_matrix(config: Config):
    """Initialize LED matrix with configuration."""
    if MOCK_DISPLAY:
        class MockOptions:
            brightness = int(config.brightness * 100)
        return MockMatrix(MockOptions())

    options = RGBMatrixOptions()
    options.rows = 32
    options.cols = 64
    options.chain_length = 3
    options.parallel = 1
    options.hardware_mapping = 'regular'
    options.gpio_slowdown = 3
    options.brightness = int(config.brightness * 100)
    options.pwm_lsb_nanoseconds = 130
    options.pwm_bits = 11
    options.pwm_dither_bits = 0
    options.drop_privileges = False
    options.limit_refresh_rate_hz = 120
    options.disable_hardware_pulsing = False

    matrix = RGBMatrix(options=options)
    print(f"✓ Matrix initialized: 192x32, brightness={config.brightness}")
    return matrix


def fetch_loop(client: MTAClient, config_holder: ConfigHolder, snapshot_queue: queue.Queue, alert_manager: AlertManager):
    """
    Background thread: Continuously fetch data and push snapshots.

    Args:
        client: MTA API client
        config_holder: Thread-safe config holder (hot-reload aware)
        snapshot_queue: Queue to push snapshots to (maxsize=1)
        alert_manager: AlertManager for filtering and prioritizing alerts
    """
    print("[FETCH] Background fetch thread started")

    last_train_fetch = 0
    last_alert_fetch = 0
    cached_alerts = []  # Keep alerts between fetches
    last_train_count = -1  # Track for change-only logging

    TRAIN_INTERVAL = 20  # seconds
    ALERT_INTERVAL = 60  # seconds

    while True:
        current_time = time.time()

        # Get current config (may have been hot-reloaded)
        config = config_holder.config

        # Fetch trains
        if current_time - last_train_fetch >= TRAIN_INTERVAL:
            # Collect all stop IDs
            all_stop_ids = []
            for uptown_id, downtown_id in config.station_stops:
                all_stop_ids.extend([uptown_id, downtown_id])

            trains = client.fetch_trains(
                stop_ids=all_stop_ids,
                routes=set(config.routes),
                max_count=config.max_trains
            )
            last_train_fetch = current_time

            # Fetch alerts (every ALERT_INTERVAL)
            if config.show_alerts and current_time - last_alert_fetch >= ALERT_INTERVAL:
                raw_alerts = client.fetch_alerts(set(config.routes))
                # Filter and sort using AlertManager (handles cooldowns, priority)
                cached_alerts = alert_manager.filter_and_sort(raw_alerts)
                last_alert_fetch = current_time

            snapshot = DisplaySnapshot(
                trains=tuple(trains),
                alerts=tuple(cached_alerts) if config.show_alerts else tuple(),
                fetched_at=current_time
            )

            # Push to queue (replace old if full)
            try:
                snapshot_queue.put_nowait(snapshot)
            except queue.Full:
                # Queue is full - discard old snapshot and put new one
                try:
                    snapshot_queue.get_nowait()
                    snapshot_queue.put_nowait(snapshot)
                except (queue.Empty, queue.Full):
                    pass

            if len(trains) != last_train_count:
                uptown = len([t for t in trains if t.direction == "N"])
                downtown = len([t for t in trains if t.direction == "S"])
                print(f"[FETCH] {len(trains)} trains ({uptown}N, {downtown}S), {len(cached_alerts)} alerts")
                last_train_count = len(trains)

        # Sleep for a bit
        time.sleep(1.0)


# Module-level client reference for cleanup
_client = None


def main():
    """Main entry point."""
    global _client

    print("=" * 60)
    print("NYC Subway Sign V3 - Starting")
    print("Best of V1 (features) + V2 (architecture)")
    print("=" * 60)

    # Load configuration with hot-reload support
    project_root = Path(__file__).parent.parent
    config_path = project_root / 'config.json'

    if not config_path.exists():
        print(f"ERROR: Config file not found: {config_path}")
        print("Please create config.json in the project root")
        sys.exit(1)

    config_holder = ConfigHolder(str(config_path))
    config = config_holder.config
    print(f"✓ Configuration loaded (hot-reload enabled)")
    print(f"  Station platforms: {len(config.station_stops)}")
    print(f"  Routes: {', '.join(config.routes)}")
    print(f"  Max trains: {config.max_trains}")
    print(f"  Show alerts: {config.show_alerts}")

    # Initialize display
    matrix = init_matrix(config)
    canvas = matrix.CreateFrameCanvas()

    # Initialize renderer
    font_path = project_root / 'assets' / 'fonts' / 'mta-sign.json'
    icon_path = project_root / 'assets' / 'icons' / 'route_icon_metadata.json'

    if not font_path.exists():
        print(f"ERROR: Font file not found: {font_path}")
        sys.exit(1)

    renderer = Renderer(str(font_path), str(icon_path))
    print("✓ Renderer initialized")

    # Initialize MTA client
    _client = MTAClient()
    print("✓ MTA client initialized")

    # Initialize alert manager
    alert_manager = AlertManager()
    print("✓ Alert manager initialized (5 min cooldown, priority sorting)")

    # Start background fetcher (uses config_holder for hot-reload)
    snapshot_queue = queue.Queue(maxsize=1)
    fetcher = threading.Thread(
        target=fetch_loop,
        args=(_client, config_holder, snapshot_queue, alert_manager),
        daemon=True,
        name="DataFetcher"
    )
    fetcher.start()
    print("✓ Background fetcher started")

    # Wait for initial data
    print("\nWaiting for initial data fetch...")
    time.sleep(3)

    # Main render loop
    print("\n" + "=" * 60)
    print("Starting render loop (30 FPS)")
    print("Hot-reload: config changes apply within 5 seconds")
    print("=" * 60 + "\n")

    current_snapshot = DisplaySnapshot.empty()
    cycle_index = 0
    flash_state = False
    alert_scroll_offset = 0.0
    show_alert = False
    current_alert = None
    alert_triggered_by = None
    alert_cycle_start_time = 0
    MAX_ALERT_CYCLE_DURATION = 90.0

    last_cycle_time = time.time()
    last_flash_time = time.time()
    last_config_check = time.time()

    CYCLE_INTERVAL = 3.0
    FLASH_INTERVAL = 0.5
    SCROLL_SPEED = 2.0
    CONFIG_CHECK_INTERVAL = 5.0

    frame_count = 0
    last_stats_time = time.time()

    TARGET_FPS = 30
    FRAME_TIME = 1.0 / TARGET_FPS

    while True:
        frame_start = time.monotonic()

        try:
            current_snapshot = snapshot_queue.get_nowait()
        except queue.Empty:
            pass

        current_time = time.time()

        if current_time - last_config_check >= CONFIG_CHECK_INTERVAL:
            last_config_check = current_time
            if config_holder.check_and_reload():
                config = config_holder.config
                print(f"  New config: {len(config.station_stops)} platforms, routes: {', '.join(config.routes)}")

        if current_time - last_cycle_time >= CYCLE_INTERVAL:
            last_cycle_time = current_time
            cycle_index = (cycle_index + 1) % 6

        if current_time - last_flash_time >= FLASH_INTERVAL:
            last_flash_time = current_time
            flash_state = not flash_state

        first_train = current_snapshot.get_first_train()
        train_at_zero = first_train.minutes == 0

        triggering_train_departed = False
        if show_alert and alert_triggered_by is not None:
            trigger_route, trigger_dest = alert_triggered_by
            trigger_still_at_zero = any(
                t.route == trigger_route and
                t.destination == trigger_dest and
                t.minutes == 0
                for t in current_snapshot.trains
            )
            triggering_train_departed = not trigger_still_at_zero

        if train_at_zero and alert_manager.has_alerts:
            arriving_train_id = (first_train.route, first_train.destination)

            if not show_alert or alert_triggered_by != arriving_train_id:
                if not show_alert:
                    alert_manager.reset_cycle()
                    current_alert = alert_manager.get_next_alert()
                    if current_alert:
                        show_alert = True
                        alert_scroll_offset = 0.0
                        alert_triggered_by = arriving_train_id
                        alert_cycle_start_time = current_time

        if show_alert and current_alert:
            cycle_elapsed = current_time - alert_cycle_start_time

            if cycle_elapsed > MAX_ALERT_CYCLE_DURATION:
                show_alert = False
                current_alert = None
                alert_scroll_offset = 0.0
                alert_triggered_by = None
            else:
                alert_scroll_offset += SCROLL_SPEED

                scroll_complete_distance = renderer.get_scroll_complete_distance()
                if alert_scroll_offset >= scroll_complete_distance:
                    alert_manager.mark_displayed(current_alert)

                    if triggering_train_departed:
                        if train_at_zero and alert_manager.has_alerts:
                            arriving_train_id = (first_train.route, first_train.destination)
                            alert_manager.reset_cycle()
                            current_alert = alert_manager.get_next_alert()
                            if current_alert:
                                alert_scroll_offset = 0.0
                                alert_triggered_by = arriving_train_id
                                alert_cycle_start_time = current_time
                            else:
                                show_alert = False
                                current_alert = None
                                alert_scroll_offset = 0.0
                                alert_triggered_by = None
                        else:
                            show_alert = False
                            current_alert = None
                            alert_scroll_offset = 0.0
                            alert_triggered_by = None

                    elif alert_manager.all_shown_this_cycle():
                        show_alert = False
                        current_alert = None
                        alert_scroll_offset = 0.0
                        alert_triggered_by = None

                    else:
                        next_alert = alert_manager.get_next_alert()
                        if next_alert:
                            current_alert = next_alert
                            alert_scroll_offset = 0.0
                        else:
                            show_alert = False
                            current_alert = None
                            alert_scroll_offset = 0.0
                            alert_triggered_by = None

        img = renderer.render_frame(
            snapshot=current_snapshot,
            cycle_index=cycle_index,
            flash_state=flash_state,
            alert_scroll_offset=alert_scroll_offset,
            show_alert=show_alert,
            current_alert=current_alert
        )

        canvas.SetImage(img)
        canvas = matrix.SwapOnVSync(canvas)

        frame_count += 1

        alert_manager.periodic_cleanup()

        if current_time - last_stats_time >= 300.0:
            fps = frame_count / (current_time - last_stats_time)
            uptown_count = len([t for t in current_snapshot.trains if t.direction == "N"])
            downtown_count = len([t for t in current_snapshot.trains if t.direction == "S"])

            print(f"[STATS] FPS: {fps:.1f} | Trains: {len(current_snapshot.trains)} ({uptown_count}N {downtown_count}S) | Alerts: {alert_manager.queue_size} | Age: {current_time - current_snapshot.fetched_at:.1f}s")

            frame_count = 0
            last_stats_time = current_time

        elapsed = time.monotonic() - frame_start
        sleep_time = FRAME_TIME - elapsed
        if sleep_time > 0:
            time.sleep(sleep_time)
        elif elapsed > FRAME_TIME * 1.5:
            print(f"[WARN] Slow frame: {elapsed*1000:.1f}ms")


def _shutdown(signum=None, frame=None):
    """Graceful shutdown handler for SIGTERM and SIGINT."""
    sig_name = signal.Signals(signum).name if signum else "unknown"
    print(f"\n\nShutting down gracefully ({sig_name})...")
    if _client:
        _client.close()
    sys.exit(0)


if __name__ == '__main__':
    signal.signal(signal.SIGTERM, _shutdown)

    try:
        main()
    except KeyboardInterrupt:
        _shutdown(signal.SIGINT)
    except Exception as e:
        print(f"\n\nFATAL ERROR: {e}")
        import traceback
        traceback.print_exc()
        if _client:
            _client.close()
        sys.exit(1)
