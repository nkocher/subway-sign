"""
Alert Manager - Clean implementation of V1's alert system intent.

Responsibilities:
1. Priority sorting (NO_SERVICE > DELAYS > etc.)
2. Station filtering (only alerts for THIS station)
3. Cooldown tracking (don't repeat same alert for 5 minutes)
4. Queue management (max 10 alerts, cycle through them)

Design principles:
- Stateless filtering functions (easy to test)
- Simple cooldown using alert_id (stable across runs, unlike hash())
- Clear separation: filtering happens at fetch time, display at render time
"""
import time
from dataclasses import dataclass, field
from typing import Optional

from .models import Alert


# GTFS-RT effect enum to priority mapping
# Lower number = higher priority (more critical)
EFFECT_PRIORITY = {
    1: 1,   # NO_SERVICE - highest priority
    2: 2,   # REDUCED_SERVICE
    3: 3,   # SIGNIFICANT_DELAYS
    4: 4,   # DETOUR
    5: 5,   # ADDITIONAL_SERVICE (good news, lower priority)
    6: 6,   # MODIFIED_SERVICE
    7: 7,   # OTHER_EFFECT
    8: 8,   # UNKNOWN_EFFECT
    9: 9,   # STOP_MOVED
}

# Cooldown period - don't show same alert for this long
COOLDOWN_SECONDS = 300  # 5 minutes

# Maximum alerts to queue
MAX_QUEUE_SIZE = 10


@dataclass
class AlertManager:
    """
    Manages alert filtering, prioritization, and cooldown tracking.

    Usage:
        manager = AlertManager()

        # When fetching alerts:
        filtered = manager.filter_and_sort(alerts, station_stop_ids)

        # When about to display:
        alert = manager.get_next_alert()
        if alert:
            # display it
            manager.mark_displayed(alert)
    """

    # Cooldown tracking: {alert_id: last_displayed_timestamp}
    _cooldowns: dict = field(default_factory=dict)

    # Current alert queue (filtered, sorted, ready to display)
    _queue: list[Alert] = field(default_factory=list)

    # Current position in queue
    _queue_index: int = 0

    # Track which alerts have been shown this cycle (for "show once" logic)
    _shown_this_cycle: set = field(default_factory=set)

    # Last cleanup timestamp to ensure periodic cleanup
    _last_cleanup: float = field(default_factory=time.time)

    def filter_and_sort(self, alerts: list[Alert]) -> list[Alert]:
        """
        Filter alerts by priority and apply cooldown.

        Args:
            alerts: Raw alerts from API

        Returns:
            Filtered, sorted, cooldown-checked alerts ready for display.
        """
        # Clean up old cooldowns (older than 2x cooldown period)
        self._cleanup_cooldowns()

        # Filter by cooldown
        non_cooled = [a for a in alerts if not self._is_on_cooldown(a)]

        # Sort by priority (lower = more important)
        sorted_alerts = sorted(non_cooled, key=lambda a: a.priority)

        # Cap queue size
        capped = sorted_alerts[:MAX_QUEUE_SIZE]

        # Update queue
        self._queue = capped
        if self._queue_index >= len(capped):
            self._queue_index = 0

        return capped

    def get_next_alert(self) -> Optional[Alert]:
        """
        Get the next alert to display from the queue.

        Skips alerts already shown this cycle AND alerts on cooldown.
        This ensures fair rotation and respects the 5-minute cooldown
        even between API fetches.

        Returns:
            Next Alert, or None if queue is empty or all shown/cooled.
        """
        if not self._queue:
            return None

        # Find the next alert that hasn't been shown this cycle and isn't on cooldown
        checked = 0
        while checked < len(self._queue):
            alert = self._queue[self._queue_index]
            key = alert.alert_id if alert.alert_id else alert.text[:100]

            # Skip if shown this cycle OR on cooldown
            if key not in self._shown_this_cycle and not self._is_on_cooldown(alert):
                return alert

            # This one was already shown or is on cooldown, try next
            self._queue_index = (self._queue_index + 1) % len(self._queue)
            checked += 1

        # All alerts shown this cycle or on cooldown
        return None

    def advance_queue(self) -> None:
        """Move to the next alert in the queue (after displaying current one)."""
        if self._queue:
            self._queue_index = (self._queue_index + 1) % len(self._queue)

    def mark_displayed(self, alert: Alert) -> None:
        """
        Mark an alert as displayed, starting its cooldown period.

        Args:
            alert: The alert that was just displayed.
        """
        # Use alert_id for stable identification
        key = alert.alert_id if alert.alert_id else alert.text[:100]
        self._cooldowns[key] = time.time()

        # Track that this alert was shown this cycle
        self._shown_this_cycle.add(key)

        # Advance to next alert
        self.advance_queue()

    def reset_cycle(self) -> None:
        """Reset the cycle tracking. Call when starting a new alert cycle."""
        self._shown_this_cycle.clear()
        self._queue_index = 0

    def all_shown_this_cycle(self) -> bool:
        """Check if all alerts in the queue have been shown this cycle."""
        if not self._queue:
            return True
        # Check if every alert in queue has been shown
        for alert in self._queue:
            key = alert.alert_id if alert.alert_id else alert.text[:100]
            if key not in self._shown_this_cycle:
                return False
        return True

    def _is_on_cooldown(self, alert: Alert) -> bool:
        """Check if an alert is still in its cooldown period."""
        key = alert.alert_id if alert.alert_id else alert.text[:100]

        if key not in self._cooldowns:
            return False

        last_shown = self._cooldowns[key]
        return (time.time() - last_shown) < COOLDOWN_SECONDS

    def _cleanup_cooldowns(self) -> None:
        """Remove expired cooldown entries to prevent memory growth."""
        cutoff = time.time() - (COOLDOWN_SECONDS * 2)
        self._cooldowns = {
            k: v for k, v in self._cooldowns.items() if v > cutoff
        }
        self._last_cleanup = time.time()

    def periodic_cleanup(self) -> None:
        """
        Run cleanup if enough time has passed since last cleanup.

        Call this periodically (e.g., every frame or every few seconds) to ensure
        cleanup happens even when filter_and_sort isn't being called due to fetch failures.
        """
        # Run cleanup every 60 seconds regardless of fetch success
        if time.time() - self._last_cleanup > 60:
            self._cleanup_cooldowns()

    @property
    def queue_size(self) -> int:
        """Number of alerts currently in queue."""
        return len(self._queue)

    @property
    def has_alerts(self) -> bool:
        """Check if there are any alerts available to display (not on cooldown)."""
        for alert in self._queue:
            if not self._is_on_cooldown(alert):
                return True
        return False


def extract_priority_from_effect(effect_value: int) -> int:
    """
    Convert GTFS-RT effect enum to priority score.

    Args:
        effect_value: GTFS-RT alert effect enum value (1-9)

    Returns:
        Priority score (1 = highest priority, 10 = lowest)
    """
    return EFFECT_PRIORITY.get(effect_value, 10)
