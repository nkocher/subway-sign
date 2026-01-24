"""
Immutable data models for MTA subway data.

All models use frozen dataclasses with __slots__ for memory efficiency
and to prevent accidental mutation.
"""
from dataclasses import dataclass
from typing import Optional
import json

from .station_lookup import (
    load_station_database,
    get_stop_ids_for_station,
    get_station_database,
)


def stop_ids_to_station_stops(stop_ids: list[str]) -> tuple[tuple[str, str], ...]:
    """Convert list of stop IDs to (uptown, downtown) tuples."""
    # Group by base ID (everything except last char which is N/S)
    platforms = {}
    for stop_id in stop_ids:
        if len(stop_id) < 2:
            continue
        base_id = stop_id[:-1]
        direction = stop_id[-1]
        if base_id not in platforms:
            platforms[base_id] = {'N': None, 'S': None}
        platforms[base_id][direction] = stop_id

    # Build tuples
    result = []
    for base_id, dirs in platforms.items():
        if dirs['N'] and dirs['S']:
            result.append((dirs['N'], dirs['S']))

    return tuple(result)


@dataclass(frozen=True, slots=True)
class Train:
    """
    A single train arrival.

    Attributes:
        route: Route ID (e.g., "1", "2", "A", "Q")
        destination: Headsign text (e.g., "Times Sq-42 St")
        minutes: Minutes until arrival (0 = arriving now, 999 = no data)
        is_express: True if running express service
        arrival_timestamp: Unix timestamp of scheduled arrival
        direction: Direction code ("N" for uptown/north, "S" for downtown/south)
        stop_id: Stop ID where this arrival occurs (for direction separation)
    """
    route: str
    destination: str
    minutes: int
    is_express: bool
    arrival_timestamp: float
    direction: str = "N"  # Default to uptown
    stop_id: str = ""

    def is_valid(self) -> bool:
        """Check if this represents real train data (not placeholder)."""
        return self.minutes < 999 and self.route != ""


@dataclass(frozen=True, slots=True)
class Alert:
    """
    Service alert message.

    Attributes:
        text: Alert message text
        affected_routes: Set of routes affected by this alert
        priority: Alert priority (1=high, 2=medium, 3=low)
        alert_id: Unique ID for deduplication and cooldown tracking
    """
    text: str
    affected_routes: frozenset[str]
    priority: int = 2  # Default to medium priority
    alert_id: str = ""


@dataclass(frozen=True, slots=True)
class DisplaySnapshot:
    """
    Complete immutable snapshot of all data needed to render a frame.

    This is passed from the background fetcher thread to the render thread
    via a queue. Being fully immutable eliminates any possibility of data races.

    Attributes:
        trains: All trains, sorted by arrival time (uptown + downtown mixed)
        alerts: Service alert messages for configured routes
        fetched_at: Unix timestamp when this data was fetched
    """
    trains: tuple[Train, ...]
    alerts: tuple[Alert, ...]
    fetched_at: float

    def get_first_train(self) -> Train:
        """Get the next arriving train (any direction)."""
        if self.trains:
            return self.trains[0]
        return self._empty_train()

    def get_cycling_trains(self, count: int = 6) -> tuple[Train, ...]:
        """Get trains #2 through #(count+1) for bottom row cycling."""
        # Skip first train (shown on top row), take next 'count' trains
        remaining = self.trains[1:count+1] if len(self.trains) > 1 else tuple()
        result = tuple(remaining)
        # Pad with empty trains if needed
        while len(result) < count:
            result = result + (self._empty_train(),)
        return result

    @staticmethod
    def _empty_train() -> Train:
        """Create an empty placeholder train."""
        return Train(
            route="",
            destination="---",
            minutes=999,
            is_express=False,
            arrival_timestamp=0.0,
            direction="N",
            stop_id=""
        )

    @staticmethod
    def empty() -> 'DisplaySnapshot':
        """Create an empty snapshot for initialization."""
        return DisplaySnapshot(
            trains=tuple(),
            alerts=tuple(),
            fetched_at=0.0
        )


@dataclass(frozen=True, slots=True)
class Config:
    """
    Application configuration.

    Attributes:
        station_stops: List of (uptown_id, downtown_id) tuples for each platform
        routes: Routes to display (e.g., ["1", "2", "3"])
        brightness: LED brightness (0.0-1.0)
        max_trains: Maximum trains to track
        show_alerts: Whether to display service alerts
    """
    station_stops: tuple[tuple[str, str], ...]  # ((uptown, downtown), ...)
    routes: tuple[str, ...]
    brightness: float
    max_trains: int
    show_alerts: bool

    @classmethod
    def load(cls, path: str) -> 'Config':
        """
        Load configuration from JSON file.

        Supports three station formats:
        1. station_name: "Times Sq-42 St" - auto-detect platforms via fuzzy matching
        2. stations: [{uptown, downtown}, ...] - explicit multi-platform
        3. uptown_stop_id/downtown_stop_id - legacy single platform
        """
        with open(path, 'r') as f:
            data = json.load(f)

        # Parse station configuration
        station = data['station']
        routes = tuple(station.get('routes', []))

        if 'station_name' in station and station['station_name']:
            # New format: station name with auto-lookup
            load_station_database(path)
            station_name = station['station_name']
            stop_ids = get_stop_ids_for_station(station_name)

            if stop_ids:
                stops = stop_ids_to_station_stops(stop_ids)
                print(f"✓ Auto-detected {len(stops)} platforms for '{station_name}'")

                # Auto-detect routes if not specified
                if not routes:
                    for s in get_station_database():
                        if s['name'].lower() == station_name.lower():
                            routes = tuple(s.get('routes', []))
                            break
            else:
                raise ValueError(f"Station '{station_name}' not found in database")

        elif 'stations' in station:
            # Multi-platform format: list of {uptown, downtown} dicts
            stops = tuple(
                (s['uptown'], s['downtown'])
                for s in station['stations']
            )
        elif 'uptown_stop_id' in station and 'downtown_stop_id' in station:
            # Legacy format: single uptown/downtown pair
            stops = ((station['uptown_stop_id'], station['downtown_stop_id']),)
        else:
            raise ValueError("Config missing station configuration (station_name, stations, or uptown_stop_id/downtown_stop_id)")

        return cls(
            station_stops=stops,
            routes=routes,
            brightness=data['display']['brightness'],
            max_trains=data['display']['max_trains'],
            show_alerts=data['display']['show_alerts']
        )
