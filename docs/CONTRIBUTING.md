# Contributing / Code Style Guide

## For Humans and LLM Agents

This document helps you modify the codebase correctly. Read this before making changes.

## Code Principles

1. **Immutability First** - Use frozen dataclasses for data models
2. **No Shared Mutable State** - Use queues for inter-thread communication
3. **Simple Over Clever** - Prefer obvious code over optimized code
4. **Type Hints Everywhere** - All functions should have type hints
5. **Never Crash** - Degrade gracefully, return empty rather than raise

## File Responsibilities

| File | Responsibility | Should NOT contain |
|------|----------------|-------------------|
| `src/main.py` | Loop orchestration, threading | Rendering logic, API calls |
| `src/mta/client.py` | HTTP requests, API interaction | Display code, business logic |
| `src/mta/models.py` | Data structures only | Any logic beyond validation |
| `src/mta/alerts.py` | Alert triggering/queue logic | Rendering, API calls |
| `src/display/renderer.py` | All drawing/rendering | Network calls, state management |
| `src/display/fonts.py` | Font loading/rendering | Business logic |
| `src/display/colors.py` | Color constants | Logic |
| `web/app.py` | Flask routes, web interface | Direct hardware access |

## Adding a New Feature

1. Identify which module owns the feature
2. Add data models to `models.py` if needed
3. Add logic to appropriate module
4. Update tests if they exist
5. Update relevant docs

## Common Patterns

### Creating Immutable Data

```python
from dataclasses import dataclass

@dataclass(frozen=True, slots=True)
class Train:
    route: str
    destination: str
    minutes: int
    arrival_timestamp: float
    direction: str  # "N" or "S"
    stop_id: str
    is_express: bool = False
```

### Inter-Thread Communication

```python
# Producer (background thread)
snapshot = DisplaySnapshot(trains=tuple(trains))
try:
    snapshot_queue.put_nowait(snapshot)
except queue.Full:
    snapshot_queue.get_nowait()  # Discard old
    snapshot_queue.put_nowait(snapshot)

# Consumer (main thread)
try:
    current = snapshot_queue.get_nowait()
except queue.Empty:
    pass  # Use previous snapshot
```

### Safe Frame Timing

```python
FRAME_TIME = 1.0 / 30.0

while True:
    frame_start = time.monotonic()
    # ... render ...
    elapsed = time.monotonic() - frame_start
    sleep_time = FRAME_TIME - elapsed
    if sleep_time > 0:
        time.sleep(sleep_time)
```

### Graceful Error Handling

```python
def fetch_data() -> list[Train]:
    try:
        # ... fetch logic ...
        return trains
    except requests.Timeout:
        logger.warning("API timeout, using cached data")
        return self._cached_trains  # Stale data better than nothing
    except Exception as e:
        logger.error(f"Fetch error: {e}")
        return []  # Empty list, don't crash
```

### Caching Pattern

```python
class TextCache:
    def __init__(self, maxsize: int = 200):
        self._cache: dict[tuple, Image] = {}
        self._maxsize = maxsize

    def get_or_create(self, key: tuple, factory: Callable) -> Image:
        if key not in self._cache:
            if len(self._cache) >= self._maxsize:
                self._cache.pop(next(iter(self._cache)))  # FIFO eviction
            self._cache[key] = factory()
        return self._cache[key]
```

## What NOT to Do

- **Don't add locks** unless absolutely necessary (use queues instead)
- **Don't create mutable shared state** between threads
- **Don't put rendering logic** in data fetching code
- **Don't use bare `except:`** clauses (always specify exception type)
- **Don't ignore type hints** - add them to all new code
- **Don't add manual GC** - trust Python's default garbage collector
- **Don't over-engineer** - solve the problem at hand, not hypothetical future problems

## Type Annotations

All functions must have type hints:

```python
# Good
def fetch_trains(station_id: str, routes: set[str]) -> list[Train]:
    ...

# Bad
def fetch_trains(station_id, routes):
    ...
```

For complex types, use the `typing` module:

```python
from typing import Optional, Callable

def get_icon(route: str, express: bool = False) -> Optional[Image]:
    ...
```

## Logging

Use the logging module, not print statements:

```python
import logging
logger = logging.getLogger(__name__)

# Levels
logger.debug("Detailed info for debugging")
logger.info("Normal operational messages")
logger.warning("Something unexpected but recoverable")
logger.error("Something failed, but app continues")
```

## Testing Changes

Before deploying:

1. Run locally in mock mode: `python3 run.py`
2. Check for import errors and type errors
3. Verify API calls work (check logs for errors)
4. Deploy to Pi and monitor logs for 10+ minutes

## Deployment

```bash
cd ~/projects/subway-sign
./deploy.sh
```

Then monitor:
```bash
ssh YOUR_USERNAME@YOUR_PI "sudo journalctl -u subway-sign.service -f"
```

## For LLM Agents

When modifying this codebase:

1. **Read ARCHITECTURE.md first** - Understand the threading model
2. **Follow the file responsibility matrix** - Don't mix concerns
3. **Use existing patterns** - Look at similar code in the file
4. **Don't add complexity** - Solve the specific problem

### Quick Reference

| To do this... | Modify this file... |
|---------------|---------------------|
| Change API fetching | `src/mta/client.py` |
| Add new data fields | `src/mta/models.py` |
| Change how trains display | `src/display/renderer.py` |
| Add web endpoint | `web/app.py` |
| Change timing/threading | `src/main.py` |
| Add new alert logic | `src/mta/alerts.py` |
| Edit route icons | Use `tools/icon_designer.py` |

### Route Icon Editing

Route icons are stored as bitmap data in `assets/fonts/mta-sign.json`. Use the icon designer tool to visualize and edit:

```bash
# Show an icon
python3 tools/icon_designer.py --show 1

# Compare icons
python3 tools/icon_designer.py --compare 1 2 3

# Show all circle icons
python3 tools/icon_designer.py --all-circles

# Show all diamond icons
python3 tools/icon_designer.py --all-diamonds
```

Icon metadata (dimensions, colors) is in `assets/route_icon_metadata.json`.
