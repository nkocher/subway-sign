# Architecture Overview

## System Purpose

Display real-time NYC subway arrivals on a 192x32 LED matrix panel (3x 64x32 chained) connected to a Raspberry Pi 4.

## Design Principles

1. **Zero lock contention in render path** - Main loop never blocks
2. **Immutable data structures** - No shared mutable state
3. **Simple > Clever** - Avoid premature optimization
4. **Type safety** - Full type hints with frozen dataclasses
5. **Fail gracefully** - Never crash, degrade smoothly

## Threading Model

```
┌─────────────────────────────────────────────────────────────┐
│                    MAIN THREAD (30 FPS)                      │
│  - Reads from snapshot_queue (non-blocking)                  │
│  - Renders train/alert display                               │
│  - Updates LED matrix                                        │
└─────────────────────────────────────────────────────────────┘
                              ↑
                    snapshot_queue (Queue)
                              ↑
┌─────────────────────────────────────────────────────────────┐
│                   BACKGROUND THREAD                          │
│  - Fetches MTA API data (every 20s trains, 60s alerts)      │
│  - Creates immutable DisplaySnapshot                         │
│  - Puts snapshot in queue                                    │
└─────────────────────────────────────────────────────────────┘
                              ↑
┌─────────────────────────────────────────────────────────────┐
│                     WEB THREAD (Flask)                       │
│  - Serves web interface for configuration                    │
│  - Sends commands via command_queue                          │
└─────────────────────────────────────────────────────────────┘
```

## Queue-Based Producer-Consumer Pattern

Instead of locks or atomic swaps, we use Python's stdlib Queue with immutable snapshots:

```python
snapshot_queue = queue.Queue(maxsize=1)

# Background thread (producer)
snapshot = DisplaySnapshot(
    trains=tuple(all_trains),  # Immutable tuple
    alerts=tuple(alert_messages),
    fetched_at=time.time()
)
try:
    snapshot_queue.put_nowait(snapshot)  # Drop old if full
except queue.Full:
    snapshot_queue.get_nowait()  # Discard old
    snapshot_queue.put_nowait(snapshot)  # Put new

# Render thread (consumer)
try:
    current = snapshot_queue.get_nowait()
except queue.Empty:
    pass  # Keep using previous snapshot
```

**Benefits:**
- `Queue` is thread-safe by default (stdlib)
- `get_nowait()` never blocks (<1μs)
- Immutable snapshots = no data races
- `maxsize=1` = automatic old-data eviction
- Clear ownership: producer creates, consumer reads

## Data Flow Pipeline

```
┌─────────────────── BACKGROUND THREAD ─────────────────┐
│                                                         │
│  MTA API → Parse → Filter → Build Snapshot → Queue    │
│     ↓                                          ↓        │
│  Backoff/Retry                           tuple(...) ←──┤── Immutable
│  ETag caching                                           │
│                                                         │
└─────────────────────────────────────────────────────────┘
                                │
                                │ Queue.put_nowait()
                                ↓
┌─────────────────── MAIN THREAD (30fps) ───────────────┐
│                                                         │
│  Queue.get_nowait() → Render → Display                │
│         ↓              ↓          ↓                     │
│    (no block)    Cache glyphs  SwapOnVSync()          │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

## Key Design Decisions

### Why Queue-Based Threading?
- Eliminates lock contention (V1 had ReadWriteLock issues)
- Immutable snapshots prevent data races
- Non-blocking reads ensure smooth rendering

### Why Simple Frame Timing?
- Complex debt-tracking frame limiter caused strobing
- Simple `time.sleep(1/30)` works reliably
- Hardware VSync handles actual display timing

```python
FRAME_TIME = 1.0 / 30.0

while True:
    frame_start = time.monotonic()
    render_frame(...)
    elapsed = time.monotonic() - frame_start
    sleep_time = FRAME_TIME - elapsed
    if sleep_time > 0:
        time.sleep(sleep_time)
```

### Alert System
- Alerts triggered on train arrival (0 min) or preview (even minutes)
- Pre-rendered to cached image, then scrolled
- Queue protected from mid-display replacement

## Memory Management

**Strategy: Allocation Hygiene > Manual GC**

1. **Pre-allocate at startup:**
   ```python
   for char in "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz .-:min":
       glyph_cache[char] = font.render(char)
   ```

2. **Use `__slots__` for data classes:**
   ```python
   @dataclass(frozen=True, slots=True)
   class Train:
       route: str
       destination: str
       minutes: int
   ```

3. **Avoid string ops in render loop:**
   ```python
   # Bad: Creates new string every frame
   time_text = f"{mins}min"

   # Good: Cache formatted strings
   if mins not in time_text_cache:
       time_text_cache[mins] = f"{mins}min"
   ```

## Caching Strategy

### Text Rendering Cache
```python
class TextCache:
    def __init__(self, maxsize=200):
        self._cache = {}  # (text, color, italic) -> Image

    def get(self, text: str, color: tuple, italic: bool) -> Image | None:
        return self._cache.get((text, color, italic))

    def put(self, text: str, color: tuple, italic: bool, img: Image):
        if len(self._cache) >= self._maxsize:
            self._cache.pop(next(iter(self._cache)))  # FIFO eviction
        self._cache[(text, color, italic)] = img
```

### Icon Cache
Icons never change - load once at startup:
```python
class IconManager:
    def __init__(self, font_wrapper):
        self.icons = {}  # (route, is_express) -> dict with 'image', 'width', etc.
        for route in "1234567ABCDEFGJLMNQRWZ":
            self.icons[(route, False)] = font_wrapper.get_route_icon(route, False)
            if route in "234567ADE":
                self.icons[(route, True)] = font_wrapper.get_route_icon(route, True)
```

**Icon Sizes:**
- Circle icons: 14px wide × 13 rows (standard routes)
- Diamond icons: 15px wide × 15 rows (express indicators in alerts)
- Icon metadata: `assets/route_icon_metadata.json`
- Icon bitmaps: `assets/fonts/mta-sign.json`

## Error Handling Philosophy

**Never Crash, Always Degrade Gracefully**

```python
def fetch_trains(station_id: str, routes: set[str]) -> list[Train]:
    try:
        # ... fetch logic ...
    except requests.Timeout:
        logger.warning("API timeout, using cached data")
        return get_cached_trains()  # Return stale data
    except Exception as e:
        logger.error(f"Unexpected error: {e}")
        return []  # Return empty rather than crashing
```

**Fallback Chain:**
1. Fresh API data (preferred)
2. Cached data from last successful fetch
3. Empty list (display "---")
4. Never crash the display loop

## Key Files

| File | Responsibility |
|------|---------------|
| `src/main.py` | Main loop, threading orchestration |
| `src/mta/client.py` | API fetching with parallel execution |
| `src/mta/models.py` | Immutable data models (Train, Alert, DisplaySnapshot) |
| `src/display/renderer.py` | All rendering logic |
| `src/display/fonts.py` | MTA font handling |
| `src/display/colors.py` | MTA route colors |
| `web/app.py` | Flask web interface |

## What We Kept (Essential)

- HTTP session pooling (works great)
- ETag caching for API (smart optimization)
- Exponential backoff (essential for reliability)
- MTA font renderer (authentic, necessary)
- Route icon system (MTA accurate)
- Express train detection (works well)
- Alert scrolling animation (core feature)
- 30 FPS target (smooth display)

## What We Removed (Cruft)

- Complex frame limiter (528 lines in V1 → simple sleep)
- Manual GC control (90 lines → trust default)
- Multiple config formats (120 lines → one JSON schema)
- Render skip optimization (was disabled anyway)
- Custom ReadWriteLock → stdlib Queue

## Success Metrics

- **Lines of Code:** <2000 LOC
- **Startup Time:** <3 seconds
- **Memory Usage:** <50MB RSS
- **CPU at Idle:** <5%
- **Frame Consistency:** >99% on-time frames
- **Maintainability:** New dev/LLM understands in <30 minutes
