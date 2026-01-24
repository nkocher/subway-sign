"""
Mock MTA Client for Stress Testing

This module provides a drop-in replacement for MTAClient that generates
synthetic data for testing purposes. It can simulate various scenarios
including rush hour traffic, network failures, and concurrent arrivals.

Usage:
    Set environment variable SUBWAY_SIGN_TEST_MODE=1 to enable mock mode.
    Then set SUBWAY_SIGN_TEST_SCENARIO to one of:
    - rush_hour
    - concurrent_arrivals
    - alert_storm
    - network_chaos
    - rapid_state_change

Example:
    SUBWAY_SIGN_TEST_MODE=1 SUBWAY_SIGN_TEST_SCENARIO=rush_hour python run.py
"""

import os
import time
import random
from typing import List, Set
from dataclasses import dataclass

from .models import Train, Alert


# ============================================================================
# CONFIGURATION
# ============================================================================

DESTINATIONS = {
    '1': ['Van Cortlandt Park-242 St', 'South Ferry'],
    '2': ['Wakefield-241 St', 'Flatbush Av-Brooklyn College'],
    '3': ['Harlem-148 St', 'New Lots Av'],
    '4': ['Woodlawn', 'New Lots Av', 'Crown Hts-Utica Av'],
    '5': ['Eastchester-Dyre Av', 'Flatbush Av-Brooklyn College'],
    '6': ['Pelham Bay Park', 'Brooklyn Bridge-City Hall'],
    '7': ['Flushing-Main St', '34 St-Hudson Yards'],
    'A': ['Inwood-207 St', 'Far Rockaway', 'Ozone Park-Lefferts Blvd'],
    'C': ['168 St', 'Euclid Av'],
    'E': ['Jamaica Center', 'World Trade Center'],
    'B': ['Bedford Park Blvd', 'Brighton Beach'],
    'D': ['Norwood-205 St', 'Coney Island-Stillwell Av'],
    'F': ['Jamaica-179 St', 'Coney Island-Stillwell Av'],
    'M': ['Forest Hills-71 Av', 'Middle Village-Metropolitan Av'],
    'G': ['Court Sq', 'Church Av'],
    'J': ['Jamaica Center', 'Broad St'],
    'Z': ['Jamaica Center', 'Broad St'],
    'L': ['8 Av', 'Canarsie-Rockaway Pkwy'],
    'N': ['Astoria-Ditmars Blvd', 'Coney Island-Stillwell Av'],
    'Q': ['96 St', 'Coney Island-Stillwell Av'],
    'R': ['Forest Hills-71 Av', 'Bay Ridge-95 St'],
    'W': ['Astoria-Ditmars Blvd', 'Whitehall St-South Ferry'],
    'S': ['Times Sq-42 St', 'Grand Central-42 St'],
}

EXPRESS_ROUTES = {'2', '3', '4', '5', '7', 'A', 'D', 'E'}

ALERT_TEMPLATES = [
    "[{route}] Delays: Signal problems at {station}",
    "[{route}] Service change: No {route} trains between {station} and {station2}",
    "[{route}] Local service only in both directions",
    "[{route}] Expect 15-20 minute delays due to earlier incident",
    "[{route}] Planned work: Service suspended {station} to {station2}",
    "Good service on the [{route}]",
    "[{route}] Running with delays due to crew availability",
    "[{route}] Trains running every 10 minutes",
    "[{route}] Skip-stop service in effect",
    "[{route}] Some trains are running express",
]

STATIONS = [
    "Times Sq-42 St", "34 St-Herald Sq", "14 St-Union Sq", "Grand Central",
    "Penn Station", "Chambers St", "Fulton St", "Brooklyn Bridge",
    "Atlantic Av-Barclays Ctr", "Jay St-MetroTech", "Court Sq",
    "Jackson Hts-Roosevelt Av", "Flushing-Main St", "125 St", "149 St-Grand Concourse"
]

SCENARIO_CONFIGS = {
    'rush_hour': {
        'trains_per_fetch': (8, 15),
        'concurrent_zero_probability': 0.4,
        'num_concurrent_zero': (2, 4),
        'alert_probability': 0.5,
        'alerts_per_cycle': (1, 4),
        'latency_ms': (50, 200),
    },
    'concurrent_arrivals': {
        'force_concurrent_zero': True,
        'num_concurrent_zero': (3, 6),
        'trains_per_fetch': (4, 8),
        'alert_probability': 0.2,
        'latency_ms': (50, 100),
    },
    'alert_storm': {
        'trains_per_fetch': (4, 8),
        'alert_probability': 1.0,
        'alerts_per_cycle': (3, 8),
        'latency_ms': (50, 150),
    },
    'network_chaos': {
        'trains_per_fetch': (5, 10),
        'failure_probability': 0.3,
        'latency_ms': (100, 2000),
        'timeout_probability': 0.1,
    },
    'rapid_state_change': {
        'trains_per_fetch': (6, 12),
        'minutes_range': (0, 5),
        'latency_ms': (10, 50),
    },
    'normal': {
        'trains_per_fetch': (5, 10),
        'alert_probability': 0.3,
        'alerts_per_cycle': (0, 2),
        'latency_ms': (100, 300),
    }
}


# ============================================================================
# MOCK CLIENT
# ============================================================================

class MockMTAClient:
    """
    Mock MTA client for stress testing.

    Generates synthetic train and alert data based on configured scenarios.
    Drop-in replacement for MTAClient when testing.
    """

    def __init__(self, scenario: str = None):
        """
        Initialize mock client.

        Args:
            scenario: Test scenario name. If None, reads from
                     SUBWAY_SIGN_TEST_SCENARIO env var (default: 'normal')
        """
        if scenario is None:
            scenario = os.environ.get('SUBWAY_SIGN_TEST_SCENARIO', 'normal')

        if scenario not in SCENARIO_CONFIGS:
            print(f"[MOCK] Unknown scenario '{scenario}', using 'normal'")
            scenario = 'normal'

        self.scenario = scenario
        self.config = SCENARIO_CONFIGS[scenario]
        self.call_count = 0
        self.alerts_call_count = 0
        self._cached_alerts: List[Alert] = []

        print(f"[MOCK] MockMTAClient initialized with scenario: {scenario}")

    def fetch_trains(self, stop_ids: List[str], routes: Set[str], max_count: int = 10) -> List[Train]:
        """
        Generate synthetic train data based on scenario.

        Args:
            stop_ids: List of stop IDs (used for direction detection)
            routes: Set of route IDs to generate trains for
            max_count: Maximum number of trains to return

        Returns:
            List of synthetic Train objects
        """
        self.call_count += 1
        config = self.config

        # Simulate network issues for chaos scenario
        if config.get('failure_probability', 0) > 0:
            if random.random() < config['failure_probability']:
                print(f"[MOCK] Simulated network failure (call #{self.call_count})")
                time.sleep(0.5)
                return []  # Return empty as if network failed

        if config.get('timeout_probability', 0) > 0:
            if random.random() < config['timeout_probability']:
                timeout_ms = random.randint(2000, 5000)
                print(f"[MOCK] Simulating timeout ({timeout_ms}ms)")
                time.sleep(timeout_ms / 1000)

        # Simulate network latency
        if 'latency_ms' in config:
            latency = random.randint(*config['latency_ms'])
            time.sleep(latency / 1000)

        # Generate trains
        trains = []
        route_list = list(routes)

        # Determine number of trains
        num_trains = random.randint(*config.get('trains_per_fetch', (5, 10)))
        num_trains = min(num_trains, max_count)

        # Handle concurrent zero-minute arrivals
        if config.get('force_concurrent_zero'):
            num_zero = random.randint(*config.get('num_concurrent_zero', (2, 4)))
            for _ in range(min(num_zero, num_trains)):
                train = self._generate_train(route_list, stop_ids, force_zero=True)
                trains.append(train)
        elif config.get('concurrent_zero_probability', 0) > 0:
            if random.random() < config['concurrent_zero_probability']:
                num_zero = random.randint(*config.get('num_concurrent_zero', (1, 3)))
                for _ in range(num_zero):
                    train = self._generate_train(route_list, stop_ids, force_zero=True)
                    trains.append(train)

        # Fill remaining with regular trains
        remaining = num_trains - len(trains)
        for _ in range(remaining):
            train = self._generate_train(route_list, stop_ids)
            trains.append(train)

        # Sort by arrival time
        trains.sort(key=lambda t: t.arrival_timestamp)

        print(f"[MOCK] Generated {len(trains)} trains (call #{self.call_count})")
        return trains[:max_count]

    def fetch_alerts(self, routes: Set[str]) -> List[Alert]:
        """
        Generate synthetic alerts based on scenario.

        Args:
            routes: Set of route IDs to generate alerts for

        Returns:
            List of synthetic Alert objects
        """
        self.alerts_call_count += 1
        config = self.config

        # Check if we should generate new alerts
        if random.random() > config.get('alert_probability', 0.3):
            # Return cached alerts
            return self._cached_alerts

        # Generate new alerts
        route_list = list(routes)
        num_alerts = random.randint(*config.get('alerts_per_cycle', (0, 2)))

        alerts = []
        seen_texts = set()

        for _ in range(num_alerts):
            alert = self._generate_alert(route_list)
            if alert.text not in seen_texts:
                seen_texts.add(alert.text)
                alerts.append(alert)

        self._cached_alerts = alerts
        print(f"[MOCK] Generated {len(alerts)} alerts (call #{self.alerts_call_count})")
        return alerts

    def _generate_train(self, routes: List[str], stop_ids: List[str], force_zero: bool = False) -> Train:
        """Generate a single mock train."""
        route = random.choice(routes)
        direction = random.choice(['N', 'S'])

        # Get destination based on direction
        destinations = DESTINATIONS.get(route, ['Unknown'])
        if direction == 'N':
            destination = destinations[0]
        else:
            destination = destinations[-1] if len(destinations) > 1 else destinations[0]

        # Determine minutes
        if force_zero:
            minutes = 0
        else:
            min_range = self.config.get('minutes_range', (1, 20))
            minutes = random.randint(min_range[0], min_range[1])

        # Express probability
        is_express = route in EXPRESS_ROUTES and random.random() < 0.3

        # Generate stop_id from provided stop_ids
        stop_id = ""
        if stop_ids:
            matching = [s for s in stop_ids if s.endswith(direction)]
            if matching:
                stop_id = random.choice(matching)
            else:
                stop_id = random.choice(stop_ids)

        return Train(
            route=route,
            destination=destination,
            minutes=minutes,
            is_express=is_express,
            arrival_timestamp=time.time() + (minutes * 60),
            direction=direction,
            stop_id=stop_id
        )

    def _generate_alert(self, routes: List[str]) -> Alert:
        """Generate a single mock alert."""
        template = random.choice(ALERT_TEMPLATES)
        route = random.choice(routes)
        station = random.choice(STATIONS)
        station2 = random.choice([s for s in STATIONS if s != station])

        text = template.format(route=route, station=station, station2=station2)

        # Determine affected routes
        if 'Good service' in text:
            affected = frozenset([route])
        else:
            if random.random() < 0.3:
                affected = frozenset(random.sample(routes, min(3, len(routes))))
            else:
                affected = frozenset([route])

        # Priority based on content
        if 'Delays' in text or 'suspended' in text:
            priority = 1
        elif 'Good service' in text:
            priority = 3
        else:
            priority = 2

        return Alert(
            text=text,
            affected_routes=affected,
            priority=priority,
            alert_id=f"mock_{int(time.time())}_{random.randint(1000, 9999)}"
        )


# ============================================================================
# FACTORY FUNCTION
# ============================================================================

def get_mta_client():
    """
    Factory function to get appropriate MTA client.

    Returns MockMTAClient if SUBWAY_SIGN_TEST_MODE=1, otherwise real MTAClient.
    """
    if os.environ.get('SUBWAY_SIGN_TEST_MODE', '0') == '1':
        scenario = os.environ.get('SUBWAY_SIGN_TEST_SCENARIO', 'normal')
        print(f"[MOCK] Test mode enabled, using MockMTAClient (scenario: {scenario})")
        return MockMTAClient(scenario)
    else:
        from .client import MTAClient
        return MTAClient()
