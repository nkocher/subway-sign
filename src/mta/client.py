"""
MTA API client with connection pooling, caching, and exponential backoff.

This module handles all interactions with the MTA GTFS-RT API, including:
- HTTP session pooling for efficiency
- ETag-based conditional requests
- Exponential backoff on failures
- Graceful error handling (never crashes)
"""
import time
import requests
from requests.adapters import HTTPAdapter
from urllib3.util.retry import Retry
from datetime import datetime
from typing import Optional
from concurrent.futures import ThreadPoolExecutor, as_completed
from nyct_gtfs import NYCTFeed
from nyct_gtfs.compiled_gtfs import gtfs_realtime_pb2

from .models import Train, Alert
from .alert_manager import extract_priority_from_effect


# Cache TTL - entries older than this are eligible for cleanup
CACHE_TTL_SECONDS = 300  # 5 minutes


_ERROR_LOG_INTERVAL = 300  # Only log same error source every 5 minutes


class MTAClient:
    """
    MTA API client with robust error handling and caching.

    Never raises exceptions - all errors are handled internally and logged.
    Returns empty lists on error, or cached data if available.
    """

    def __init__(self):
        """Initialize the client with session pooling configured."""
        self._session = self._create_session()
        self._feed_cache = {}  # {feed_id: {data: [...], timestamp: int, fetched_at: float}}
        self._alerts_cache = {}  # {route: [alerts]}
        self._alerts_etag = None
        self._backoff = {}  # {feed_id: {failures: int, retry_after: float}}
        self._last_error_log = {}  # {source_key: timestamp} for rate-limited logging
        # Reuse a single ThreadPoolExecutor instead of creating one per fetch
        self._executor = ThreadPoolExecutor(max_workers=8)

    def _create_session(self) -> requests.Session:
        """Create HTTP session with connection pooling and retries."""
        session = requests.Session()
        session.headers.update({
            'User-Agent': 'NYC-SubwaySign-V2/2.0',
            'Accept-Encoding': 'gzip, deflate'
        })

        # Configure retry strategy
        retry_strategy = Retry(
            total=3,
            backoff_factor=0.5,
            status_forcelist=[429, 500, 502, 503, 504],
            allowed_methods=["GET"]
        )

        adapter = HTTPAdapter(
            pool_connections=4,
            pool_maxsize=8,
            max_retries=retry_strategy
        )

        session.mount('https://', adapter)
        session.mount('http://', adapter)

        return session

    def fetch_trains(self, stop_ids: list[str], routes: set[str], max_count: int = 10) -> list[Train]:
        """
        Fetch upcoming trains for given stops and routes IN PARALLEL.

        Args:
            stop_ids: List of stop IDs to query
            routes: Set of route IDs to filter for
            max_count: Maximum number of trains to return

        Returns:
            List of Train objects, sorted by arrival time.
            Returns empty list or cached data on error.
        """
        all_trains = []

        # Determine which feeds to fetch
        feeds_to_fetch = self._get_feeds_for_routes(routes)

        # Fetch feeds in parallel using reusable ThreadPoolExecutor
        # Submit all feed fetch tasks
        future_to_feed = {
            self._executor.submit(self._fetch_single_feed, feed_id, stop_ids, routes): feed_id
            for feed_id in feeds_to_fetch
        }

        # Collect results as they complete
        for future in as_completed(future_to_feed):
            feed_id = future_to_feed[future]
            try:
                feed_trains = future.result()
                all_trains.extend(feed_trains)
            except Exception as e:
                self._log_error(f"parallel_{feed_id}", f"Error in parallel fetch for feed {feed_id}: {e}")
                # Use cached data as fallback
                all_trains.extend(self._get_cached_trains(feed_id))

        # Periodic cache cleanup to prevent memory growth
        self._cleanup_feed_cache()

        # Sort by arrival time and deduplicate
        all_trains.sort(key=lambda t: t.arrival_timestamp)
        unique_trains = self._deduplicate_trains(all_trains)

        return unique_trains[:max_count]

    def _fetch_single_feed(self, feed_id: str, stop_ids: list[str], routes: set[str]) -> list[Train]:
        """
        Fetch and parse a single GTFS-RT feed (called in parallel).

        Args:
            feed_id: Feed ID to fetch
            stop_ids: Stop IDs to filter for
            routes: Routes to filter for

        Returns:
            List of Train objects from this feed
        """
        # Check backoff
        if not self._should_fetch(feed_id):
            # Use cached data
            return self._get_cached_trains(feed_id)

        try:
            # Fetch feed
            feed = NYCTFeed(feed_id, fetch_immediately=True)

            # Check if data changed (timestamp comparison)
            feed_timestamp = 0
            if hasattr(feed, '_feed') and feed._feed.header.HasField('timestamp'):
                feed_timestamp = feed._feed.header.timestamp

            cached_timestamp = self._feed_cache.get(feed_id, {}).get('timestamp', 0)
            if feed_timestamp > 0 and feed_timestamp == cached_timestamp:
                # Data unchanged, use cache
                self._record_success(feed_id)
                return self._get_cached_trains(feed_id)

            # Parse new data
            feed_trains = []
            now = datetime.now()

            for trip in feed.trips:
                trip_route = getattr(trip, 'route_id', None)
                if trip_route not in routes:
                    continue

                is_express = self._detect_express(trip, trip_route)

                for stop_time in trip.stop_time_updates:
                    if stop_time.stop_id not in stop_ids:
                        continue

                    if not stop_time.arrival or stop_time.arrival <= now:
                        continue

                    arrival_ts = stop_time.arrival.timestamp()
                    mins = max(0, int((stop_time.arrival - now).total_seconds() / 60))
                    headsign = trip.headsign_text or "Unknown"

                    # Determine direction from stop_id (last character: N=north/uptown, S=south/downtown)
                    direction = stop_time.stop_id[-1] if stop_time.stop_id else "N"
                    if direction not in ("N", "S"):
                        direction = "N"  # Default if unclear

                    feed_trains.append(Train(
                        route=trip_route,
                        destination=headsign,
                        minutes=mins,
                        is_express=is_express,
                        arrival_timestamp=arrival_ts,
                        direction=direction,
                        stop_id=stop_time.stop_id
                    ))
                    break  # Only first stop time per trip

            # Cache the results with fetch timestamp for TTL cleanup
            self._feed_cache[feed_id] = {
                'data': feed_trains,
                'timestamp': feed_timestamp,
                'fetched_at': time.time()
            }

            self._record_success(feed_id)
            return feed_trains

        except Exception as e:
            self._log_error(f"feed_{feed_id}", f"Error fetching feed {feed_id}: {e}")
            self._record_failure(feed_id)
            # Use cached data as fallback
            return self._get_cached_trains(feed_id)

    def fetch_alerts(self, routes: set[str]) -> list[Alert]:
        """
        Fetch service alerts for given routes.

        Args:
            routes: Set of route IDs to get alerts for

        Returns:
            List of Alert objects.
            Returns cached data on error.
        """
        feed_id = 'alerts'

        # Check backoff
        if not self._should_fetch(feed_id):
            return self._get_cached_alerts(routes)

        try:
            url = 'https://api-endpoint.mta.info/Dataservice/mtagtfsfeeds/camsys%2Fsubway-alerts'

            headers = {}
            if self._alerts_etag:
                headers['If-None-Match'] = self._alerts_etag

            response = self._session.get(url, timeout=12, headers=headers)

            # Handle 304 Not Modified
            if response.status_code == 304:
                self._record_success(feed_id)
                return self._get_cached_alerts(routes)

            # Store ETag
            if 'ETag' in response.headers:
                self._alerts_etag = response.headers['ETag']

            # Parse protobuf
            feed = gtfs_realtime_pb2.FeedMessage()
            feed.ParseFromString(response.content)

            # Extract alerts as Alert objects
            alert_objects = []
            seen_alerts = set()  # Deduplicate by text

            for entity in feed.entity:
                if entity.HasField('alert'):
                    alert_proto = entity.alert
                    affected_routes = set()

                    for informed_entity in alert_proto.informed_entity:
                        if informed_entity.HasField('route_id'):
                            affected_routes.add(informed_entity.route_id)

                    relevant_routes = affected_routes.intersection(routes)
                    if relevant_routes and alert_proto.HasField('header_text'):
                        # Extract priority from GTFS-RT effect field
                        if alert_proto.HasField('effect'):
                            priority = extract_priority_from_effect(alert_proto.effect)
                        else:
                            priority = 10  # Unknown effect = lowest priority

                        for translation in alert_proto.header_text.translation:
                            clean_text = ' '.join(translation.text.split())

                            # Deduplicate and create Alert object
                            if clean_text not in seen_alerts:
                                seen_alerts.add(clean_text)
                                alert_obj = Alert(
                                    text=clean_text,
                                    affected_routes=frozenset(relevant_routes),
                                    priority=priority,
                                    alert_id=entity.id
                                )
                                alert_objects.append(alert_obj)
                            break

            # Cache results (keep as dict for backward compat)
            self._alerts_cache = {route: [a.text for a in alert_objects if route in a.affected_routes]
                                  for route in routes}
            self._record_success(feed_id)

            # Return Alert objects
            return alert_objects

        except Exception as e:
            self._log_error("alerts", f"Error fetching alerts: {e}")
            self._record_failure(feed_id)
            return self._get_cached_alerts(routes)

    def _get_feeds_for_routes(self, routes: set[str]) -> set[str]:
        """Map routes to their GTFS-RT feed IDs."""
        feeds = set()
        if any(r in ['1', '2', '3', '4', '5', '6', 'GS'] for r in routes):
            feeds.add('1')
        if any(r in ['7'] for r in routes):
            feeds.add('7')
        if any(r in ['A', 'C', 'E'] for r in routes):
            feeds.add('A')
        if any(r in ['B', 'D', 'F', 'M'] for r in routes):
            feeds.add('B')
        if any(r in ['G'] for r in routes):
            feeds.add('G')
        if any(r in ['J', 'Z'] for r in routes):
            feeds.add('J')
        if any(r in ['L'] for r in routes):
            feeds.add('L')
        if any(r in ['N', 'Q', 'R', 'W'] for r in routes):
            feeds.add('N')
        return feeds

    def _detect_express(self, trip, route_id: str) -> bool:
        """Detect if a train is running express service."""
        # Only these routes can run express
        if route_id not in {'2', '3', '4', '5', '6', '7', 'A', 'D', 'E'}:
            return False

        # Check trip ID suffix
        trip_id = getattr(trip, 'trip_id', '')
        if trip_id and trip_id.endswith('X'):
            return True

        return False

    def _deduplicate_trains(self, trains: list[Train]) -> list[Train]:
        """Remove duplicate trains (same route/dest within 1 minute)."""
        unique = []
        seen = set()

        for train in trains:
            key = (train.route, train.destination, train.minutes)
            if key not in seen:
                unique.append(train)
                seen.add(key)

        return unique

    def _should_fetch(self, feed_id: str) -> bool:
        """Check if feed is in backoff period."""
        if feed_id not in self._backoff:
            return True
        return time.time() >= self._backoff[feed_id]['retry_after']

    def _record_success(self, feed_id: str):
        """Clear backoff on successful fetch."""
        if feed_id in self._backoff:
            del self._backoff[feed_id]

    def _record_failure(self, feed_id: str):
        """Record failure and calculate exponential backoff."""
        failures = self._backoff.get(feed_id, {}).get('failures', 0) + 1
        backoff_seconds = min(300, 15 * (2 ** (failures - 1)))  # 15s, 30s, 60s, ...
        self._backoff[feed_id] = {
            'failures': failures,
            'retry_after': time.time() + backoff_seconds
        }

    def _get_cached_trains(self, feed_id: str) -> list[Train]:
        """Retrieve cached train data for a feed."""
        return self._feed_cache.get(feed_id, {}).get('data', [])

    def _get_cached_alerts(self, routes: set[str]) -> list[Alert]:
        """Retrieve cached alerts for routes as Alert objects."""
        # Convert cached string alerts back to Alert objects
        alert_objects = []
        seen_texts = set()
        for route in routes:
            for msg in self._alerts_cache.get(route, []):
                if msg not in seen_texts:
                    seen_texts.add(msg)
                    alert_objects.append(Alert(
                        text=msg,
                        affected_routes=frozenset([route]),
                        priority=2,
                        alert_id=""
                    ))
        return alert_objects

    def _log_error(self, source: str, msg: str) -> None:
        """Rate-limited error logging (one message per source per 5 minutes)."""
        now = time.time()
        if now - self._last_error_log.get(source, 0) >= _ERROR_LOG_INTERVAL:
            print(f"[MTA] {msg}")
            self._last_error_log[source] = now

    def _cleanup_feed_cache(self) -> None:
        """Remove stale feed cache entries to prevent memory growth."""
        cutoff = time.time() - CACHE_TTL_SECONDS
        stale_feeds = [
            feed_id for feed_id, entry in self._feed_cache.items()
            if entry.get('fetched_at', 0) < cutoff
        ]
        for feed_id in stale_feeds:
            del self._feed_cache[feed_id]

    def close(self) -> None:
        """Clean up resources. Call when shutting down."""
        if self._executor:
            self._executor.shutdown(wait=True, cancel_futures=True)
        if self._session:
            self._session.close()
