"""
Station lookup utilities with fuzzy matching.

Provides station name normalization and stop ID lookup functions
used by both the main display code and web interface.
"""
import json
import re
from pathlib import Path
from typing import Optional


# Module-level station database (loaded once)
_STATION_DATABASE: list = []
_STATION_INDEX: dict = {}


def _normalize_station_name(name: str) -> str:
    """
    Normalize station name for fuzzy matching.
    - Remove ordinal suffixes (st, nd, rd, th from numbers)
    - Standardize spacing/punctuation
    - Convert to lowercase
    """
    name = name.lower()
    # Remove ordinal suffixes (1st, 2nd, 3rd, etc.)
    name = re.sub(r'(\d+)(st|nd|rd|th)\b', r'\1', name)
    # Standardize spacing around dashes
    name = re.sub(r'\s*-\s*', '-', name)
    name = re.sub(r'\s+', ' ', name)
    # Common abbreviations
    name = name.replace('street', 'st')
    name = name.replace('avenue', 'av')
    name = name.replace('square', 'sq')
    return name.strip()


def load_station_database(config_path: str) -> None:
    """
    Load station database from assets/mta_stations_complete.json.

    Args:
        config_path: Path to config.json (used to find assets directory)
    """
    global _STATION_DATABASE, _STATION_INDEX

    if _STATION_DATABASE:
        return  # Already loaded

    # Find the station database file relative to config path
    config_dir = Path(config_path).parent
    possible_paths = [
        config_dir / 'assets' / 'mta_stations_complete.json',
        config_dir.parent / 'assets' / 'mta_stations_complete.json',
        Path('/home/admin/subway-sign/assets/mta_stations_complete.json'),
    ]

    for db_path in possible_paths:
        if db_path.exists():
            try:
                with open(db_path, 'r') as f:
                    _STATION_DATABASE = json.load(f)

                # Build fuzzy matching index
                for station in _STATION_DATABASE:
                    name = station['name']
                    _STATION_INDEX[name.lower()] = station
                    normalized = _normalize_station_name(name)
                    if normalized not in _STATION_INDEX:
                        _STATION_INDEX[normalized] = station

                print(f"[STATION_LOOKUP] Loaded {len(_STATION_DATABASE)} stations")
                return
            except Exception as e:
                print(f"[STATION_LOOKUP] Warning: Failed to load station database: {e}")

    print("[STATION_LOOKUP] Warning: Station database not found, station_name lookup disabled")


def get_stop_ids_for_station(station_name: str) -> list[str]:
    """
    Get all stop IDs for a station name with fuzzy matching.

    Args:
        station_name: Station name to look up (e.g., "Times Sq-42 St")

    Returns:
        List of stop IDs (e.g., ["127N", "127S", "725N", "725S"]) or empty list if not found
    """
    if not station_name or not _STATION_INDEX:
        return []

    name_lower = station_name.lower().strip()

    # Exact match
    if name_lower in _STATION_INDEX:
        return _STATION_INDEX[name_lower]['stop_ids']

    # Dash normalization
    normalized_dash = name_lower.replace(' - ', '-').replace('  ', ' ')
    if normalized_dash in _STATION_INDEX:
        return _STATION_INDEX[normalized_dash]['stop_ids']

    # Full normalization
    normalized = _normalize_station_name(station_name)
    if normalized in _STATION_INDEX:
        return _STATION_INDEX[normalized]['stop_ids']

    # Substring match
    normalized_query = normalized.replace('-', ' ')
    for indexed_name, station in _STATION_INDEX.items():
        indexed_normalized = indexed_name.replace('-', ' ')
        if normalized_query in indexed_normalized or indexed_normalized in normalized_query:
            return station['stop_ids']

    return []


def get_station_database() -> list:
    """Get the loaded station database."""
    return _STATION_DATABASE


def get_station_index() -> dict:
    """Get the station lookup index."""
    return _STATION_INDEX


def find_similar_stations(query: str, max_results: int = 5) -> list[str]:
    """
    Find stations with similar names for suggestions.

    Args:
        query: Search query
        max_results: Maximum number of suggestions to return

    Returns:
        List of station names that match the query
    """
    query_lower = query.lower()
    query_words = set(query_lower.replace('-', ' ').split())

    matches = []
    seen = set()

    for station in _STATION_DATABASE:
        name = station['name']
        if name in seen:
            continue
        seen.add(name)

        name_lower = name.lower()
        name_words = set(name_lower.replace('-', ' ').split())

        common_words = query_words & name_words
        if len(common_words) > 0:
            score = len(common_words) / max(len(query_words), len(name_words))
            if score > 0.2:
                matches.append((score, name))

    matches.sort(reverse=True, key=lambda x: x[0])
    return [name for score, name in matches[:max_results]]
