#!/usr/bin/env python3
"""
Station Database Validator

Validates and fixes the local station database against official MTA GTFS data.
Downloads the latest Stations.csv from MTA and cross-references stop IDs.

Usage:
    python3 tools/validate_stations.py           # Dry run - show issues only
    python3 tools/validate_stations.py --fix     # Fix issues in database
    python3 tools/validate_stations.py --rebuild # Complete rebuild from MTA data
"""

import argparse
import csv
import json
import sys
from collections import defaultdict
from io import StringIO
from pathlib import Path
from urllib.request import urlopen

# MTA GTFS Stations CSV URL
MTA_STATIONS_URL = "http://web.mta.info/developers/data/nyct/subway/Stations.csv"

# Path to local station database
SCRIPT_DIR = Path(__file__).parent
PROJECT_ROOT = SCRIPT_DIR.parent
STATIONS_DB_PATH = PROJECT_ROOT / "assets" / "mta_stations_complete.json"


def fetch_mta_stations() -> list[dict]:
    """Fetch and parse official MTA Stations.csv."""
    print("Fetching official MTA GTFS data...")

    try:
        with urlopen(MTA_STATIONS_URL, timeout=30) as response:
            content = response.read().decode('utf-8')
    except Exception as e:
        print(f"ERROR: Failed to fetch MTA data: {e}")
        sys.exit(1)

    reader = csv.DictReader(StringIO(content))
    stations = list(reader)
    print(f"  Downloaded {len(stations)} station records")
    return stations


def build_mta_index(mta_stations: list[dict]) -> dict:
    """
    Build index of MTA stations grouped by Complex ID.

    Returns:
        {complex_id: {
            'name': station_name,
            'stop_ids': [list of GTFS Stop IDs],
            'routes': set of routes,
            'lat': latitude,
            'lon': longitude,
            'borough': borough
        }}
    """
    complexes = defaultdict(lambda: {
        'names': set(),
        'stop_ids': set(),
        'routes': set(),
        'lats': [],
        'lons': [],
        'boroughs': set()
    })

    for row in mta_stations:
        complex_id = row['Complex ID']
        stop_id = row['GTFS Stop ID']
        name = row['Stop Name']
        routes = row['Daytime Routes'].split()
        lat = float(row['GTFS Latitude'])
        lon = float(row['GTFS Longitude'])
        borough = row['Borough']

        c = complexes[complex_id]
        c['names'].add(name)
        c['stop_ids'].add(stop_id)
        c['routes'].update(routes)
        c['lats'].append(lat)
        c['lons'].append(lon)
        c['boroughs'].add(borough)

    # Finalize complexes
    result = {}
    for complex_id, data in complexes.items():
        # Choose best name: prefer shorter names that contain common identifiers
        # (e.g., "Times Sq-42 St" over "42 St-Port Authority Bus Terminal")
        name = pick_best_station_name(data['names'])

        result[complex_id] = {
            'name': name,
            'stop_ids': sorted(data['stop_ids']),
            'routes': sorted(data['routes']),
            'lat': sum(data['lats']) / len(data['lats']),
            'lon': sum(data['lons']) / len(data['lons']),
            'borough': list(data['boroughs'])[0]
        }

    return result


def pick_best_station_name(names: set[str]) -> str:
    """
    Pick the best station name from a set of alternatives.

    Prefers:
    1. Names with recognizable landmarks (Times Sq, Grand Central, Union Sq, etc.)
    2. Shorter names over longer ones (less verbose)
    3. Names with street numbers over generic descriptions
    """
    if len(names) == 1:
        return list(names)[0]

    # High-priority landmark patterns
    landmarks = [
        'Times Sq', 'Grand Central', 'Union Sq', 'Penn Station', 'Herald Sq',
        'Barclays', 'Fulton', 'Atlantic', 'Jamaica', 'Flushing', 'Coney Island',
        'Brighton Beach', 'World Trade', 'Columbus Circle', 'Rockefeller',
        'City Hall', 'Borough Hall', 'Court Sq', 'Jackson Hts', 'Forest Hills'
    ]

    # Score each name
    def score_name(name: str) -> tuple:
        # Check for landmark (higher = better)
        landmark_score = 0
        for i, landmark in enumerate(landmarks):
            if landmark.lower() in name.lower():
                landmark_score = len(landmarks) - i  # Earlier in list = higher score
                break

        # Prefer shorter names (lower length = better, so negate)
        length_score = -len(name)

        # Prefer names with street numbers
        has_number = 1 if any(c.isdigit() for c in name) else 0

        return (landmark_score, has_number, length_score)

    return max(names, key=score_name)


def build_stop_id_to_complex(mta_stations: list[dict]) -> dict:
    """Build reverse index: stop_id -> (complex_id, station_name, routes)."""
    index = {}
    for row in mta_stations:
        stop_id = row['GTFS Stop ID']
        index[stop_id] = {
            'complex_id': row['Complex ID'],
            'name': row['Stop Name'],
            'routes': row['Daytime Routes'].split()
        }
    return index


def load_local_database() -> list[dict]:
    """Load local station database."""
    if not STATIONS_DB_PATH.exists():
        print(f"ERROR: Local database not found: {STATIONS_DB_PATH}")
        sys.exit(1)

    with open(STATIONS_DB_PATH) as f:
        return json.load(f)


def save_local_database(stations: list[dict]):
    """Save local station database."""
    with open(STATIONS_DB_PATH, 'w') as f:
        json.dump(stations, f, indent=2)
    print(f"  Saved to {STATIONS_DB_PATH}")


def validate_station(station: dict, stop_id_index: dict, complex_index: dict) -> dict:
    """
    Validate a single station against MTA data.

    Returns:
        {
            'valid': bool,
            'issues': [list of issue descriptions],
            'fixes': {field: new_value} or None
        }
    """
    issues = []
    fixes = {}

    name = station['name']
    stop_ids = [sid.rstrip('NS') for sid in station['stop_ids'][::2]]  # Get base IDs (remove N/S suffix)

    # Check each stop ID belongs to this station
    expected_complex_ids = set()
    invalid_stop_ids = []

    for stop_id in stop_ids:
        if stop_id not in stop_id_index:
            issues.append(f"Stop ID '{stop_id}' not found in MTA data")
            invalid_stop_ids.append(stop_id)
            continue

        mta_info = stop_id_index[stop_id]
        expected_complex_ids.add(mta_info['complex_id'])

        # Check if this stop ID actually belongs to a station with similar name
        mta_name = mta_info['name']
        if not names_match(name, mta_name):
            issues.append(f"Stop ID '{stop_id}' belongs to '{mta_name}', not '{name}'")
            invalid_stop_ids.append(stop_id)

    # If multiple complex IDs, station might be incorrectly merged
    if len(expected_complex_ids) > 1:
        issues.append(f"Stop IDs span multiple MTA complexes: {expected_complex_ids}")

    # Find the correct complex for this station name
    correct_complex = find_matching_complex(name, complex_index)

    if correct_complex:
        correct_stop_ids = correct_complex['stop_ids']
        correct_stop_ids_with_dir = []
        for sid in correct_stop_ids:
            correct_stop_ids_with_dir.extend([f"{sid}N", f"{sid}S"])

        # Check if we're missing any stop IDs
        current_base_ids = set(stop_ids)
        correct_base_ids = set(correct_stop_ids)

        missing = correct_base_ids - current_base_ids - set(invalid_stop_ids)
        extra = current_base_ids - correct_base_ids

        if missing:
            issues.append(f"Missing stop IDs: {sorted(missing)}")
        if extra:
            issues.append(f"Extra/incorrect stop IDs: {sorted(extra)}")

        # Prepare fix
        if invalid_stop_ids or missing or extra:
            fixes['stop_ids'] = correct_stop_ids_with_dir
            fixes['platform_count'] = len(correct_stop_ids)
            fixes['routes'] = correct_complex['routes']

    return {
        'valid': len(issues) == 0,
        'issues': issues,
        'fixes': fixes if fixes else None
    }


def names_match(name1: str, name2: str) -> bool:
    """Check if two station names refer to the same station."""
    # Normalize names
    def normalize(n):
        n = n.lower()
        n = n.replace('-', ' ').replace('–', ' ')
        n = n.replace('st', '').replace('av', '').replace('blvd', '')
        n = n.replace('  ', ' ').strip()
        return n

    n1, n2 = normalize(name1), normalize(name2)

    # Exact match
    if n1 == n2:
        return True

    # One contains the other
    if n1 in n2 or n2 in n1:
        return True

    # First significant word matches
    words1 = n1.split()
    words2 = n2.split()
    if words1 and words2 and words1[0] == words2[0]:
        return True

    return False


def find_matching_complex(station_name: str, complex_index: dict) -> dict | None:
    """Find the MTA complex that best matches a station name."""
    best_match = None
    best_score = 0

    for complex_id, data in complex_index.items():
        mta_name = data['name']

        if names_match(station_name, mta_name):
            # Score by how close the names are
            score = len(set(station_name.lower()) & set(mta_name.lower()))
            if score > best_score:
                best_score = score
                best_match = data

    return best_match


def validate_all(local_db: list[dict], mta_stations: list[dict], fix: bool = False) -> tuple[int, int]:
    """
    Validate all stations in local database.

    Returns:
        (total_issues, fixed_count)
    """
    stop_id_index = build_stop_id_to_complex(mta_stations)
    complex_index = build_mta_index(mta_stations)

    total_issues = 0
    fixed_count = 0

    print("\n" + "=" * 70)
    print("VALIDATION RESULTS")
    print("=" * 70)

    for station in local_db:
        result = validate_station(station, stop_id_index, complex_index)

        if not result['valid']:
            total_issues += 1
            print(f"\n{station['name']}:")
            for issue in result['issues']:
                print(f"  ⚠️  {issue}")

            if fix and result['fixes']:
                # Apply fixes
                for field, value in result['fixes'].items():
                    old_value = station.get(field)
                    station[field] = value
                    print(f"  ✓ Fixed {field}: {old_value} → {value}")
                fixed_count += 1

    print("\n" + "=" * 70)
    print(f"Total stations: {len(local_db)}")
    print(f"Issues found: {total_issues}")
    if fix:
        print(f"Issues fixed: {fixed_count}")
    print("=" * 70)

    return total_issues, fixed_count


def rebuild_database(mta_stations: list[dict]) -> list[dict]:
    """
    Rebuild entire station database from MTA GTFS data.

    This creates a fresh database with all stations properly grouped
    by Complex ID, then merges nearby stations with the same name.
    """
    print("\nRebuilding database from MTA GTFS data...")

    complex_index = build_mta_index(mta_stations)

    stations = []
    for complex_id, data in complex_index.items():
        # Create stop IDs with N/S suffixes
        stop_ids = []
        for sid in data['stop_ids']:
            stop_ids.extend([f"{sid}N", f"{sid}S"])

        station = {
            'name': data['name'],
            'stop_ids': stop_ids,
            'routes': data['routes'],
            'lat': round(data['lat'], 6),
            'lon': round(data['lon'], 6),
            'borough': data['borough'],
            'platform_count': len(data['stop_ids'])
        }
        stations.append(station)

    # Merge stations with same name (different MTA complexes but same physical location)
    stations = merge_same_name_stations(stations)

    # Sort by name
    stations.sort(key=lambda s: s['name'])

    print(f"  Created {len(stations)} station entries")
    return stations


def merge_same_name_stations(stations: list[dict]) -> list[dict]:
    """
    Merge stations with identical names into single entries.

    MTA uses different Complex IDs for different lines at the same physical
    station (e.g., 125 St has 4 separate complexes). This merges them.
    """
    # Group by exact name
    by_name = defaultdict(list)
    for s in stations:
        by_name[s['name']].append(s)

    merged = []
    for name, group in by_name.items():
        if len(group) == 1:
            merged.append(group[0])
        else:
            # Merge all stations with same name
            all_stop_ids = []
            all_routes = set()
            lats = []
            lons = []
            boroughs = set()

            for s in group:
                all_stop_ids.extend(s['stop_ids'])
                all_routes.update(s['routes'])
                lats.append(s['lat'])
                lons.append(s['lon'])
                boroughs.add(s['borough'])

            # Remove duplicate stop IDs while preserving order
            seen = set()
            unique_stop_ids = []
            for sid in all_stop_ids:
                if sid not in seen:
                    seen.add(sid)
                    unique_stop_ids.append(sid)

            merged_station = {
                'name': name,
                'stop_ids': sorted(unique_stop_ids),
                'routes': sorted(all_routes),
                'lat': round(sum(lats) / len(lats), 6),
                'lon': round(sum(lons) / len(lons), 6),
                'borough': list(boroughs)[0],
                'platform_count': len(unique_stop_ids) // 2
            }
            merged.append(merged_station)

    return merged


def main():
    parser = argparse.ArgumentParser(description='Validate station database against MTA GTFS')
    parser.add_argument('--fix', action='store_true', help='Fix issues in database')
    parser.add_argument('--rebuild', action='store_true', help='Complete rebuild from MTA data')
    args = parser.parse_args()

    # Fetch official MTA data
    mta_stations = fetch_mta_stations()

    if args.rebuild:
        # Complete rebuild
        new_db = rebuild_database(mta_stations)
        save_local_database(new_db)
        print("\n✓ Database rebuilt from official MTA GTFS data")
        return

    # Load and validate local database
    local_db = load_local_database()
    print(f"Loaded {len(local_db)} stations from local database")

    total_issues, fixed_count = validate_all(local_db, mta_stations, fix=args.fix)

    if args.fix and fixed_count > 0:
        save_local_database(local_db)
        print(f"\n✓ Fixed {fixed_count} stations")
    elif total_issues > 0 and not args.fix:
        print("\nRun with --fix to automatically fix these issues")
        print("Run with --rebuild for a complete rebuild from MTA data")


if __name__ == '__main__':
    main()
