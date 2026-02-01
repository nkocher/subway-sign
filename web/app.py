#!/usr/bin/env python3
"""
NYC MTA Subway Sign V3 - Web Control Interface
Flask application for remotely configuring the subway sign display.

Supports three station configuration formats:
1. station_name (NEW): Automatic platform detection using station database
2. stations array: Manual multi-platform configuration
3. uptown_stop_id/downtown_stop_id: Legacy single-platform format
"""

import os
import sys
import json
import subprocess
import time as _time
from datetime import datetime
from pathlib import Path
from flask import Flask, render_template, jsonify, request, send_from_directory

# Add src to path for imports
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'src'))

from mta.station_lookup import (
    load_station_database,
    get_stop_ids_for_station,
    get_station_database,
    find_similar_stations,
)

app = Flask(__name__)

# Path configuration
BASE_DIR = Path(__file__).resolve().parent.parent
CONFIG_FILE = BASE_DIR / "config.json"
STATIONS_FILE = Path(__file__).resolve().parent / "stations.json"
COMPLETE_STATIONS_FILE = BASE_DIR / "assets" / "mta_stations_complete.json"

# Initialize station lookup using shared module
load_station_database(str(CONFIG_FILE))
COMPLETE_STATIONS = get_station_database()


# Default configuration structure
DEFAULT_CONFIG = {
    "station": {
        "station_name": "34 St-Herald Sq",
        "routes": ["B", "D", "F", "M", "N", "Q", "R", "W"]
    },
    "display": {
        "brightness": 0.3,
        "max_trains": 7,
        "show_alerts": True
    }
}


def load_config():
    """Load current configuration from config.json"""
    try:
        if CONFIG_FILE.exists():
            with open(CONFIG_FILE, 'r') as f:
                return json.load(f)
        else:
            return DEFAULT_CONFIG.copy()
    except Exception as e:
        print(f"Error loading config: {e}")
        return DEFAULT_CONFIG.copy()


def save_config(config_data):
    """Save configuration to config.json with validation"""
    if 'station' not in config_data or 'display' not in config_data:
        return False, "Missing required top-level sections"
    
    station = config_data['station']
    
    has_station_name = bool('station_name' in station and station['station_name'])
    has_old_format = bool('uptown_stop_id' in station and 'downtown_stop_id' in station)
    has_new_format = bool('stations' in station)
    
    format_count = sum([has_station_name, has_old_format, has_new_format])
    
    if format_count > 1:
        return False, "Config contains multiple formats. Use only one."
    elif has_station_name:
        stop_ids = get_stop_ids_for_station(station['station_name'])
        if not stop_ids:
            similar = find_similar_stations(station['station_name'])
            if similar:
                suggestions = ', '.join(f"'{s}'" for s in similar[:3])
                return False, f"Station '{station['station_name']}' not found. Did you mean: {suggestions}?"
            else:
                return False, f"Station '{station['station_name']}' not found in database."
        # station_name format is valid - display script handles multi-platform detection
    elif has_old_format:
        station_fields = ['uptown_stop_id', 'downtown_stop_id', 'routes']
        for field in station_fields:
            if field not in station:
                return False, f"Missing station.{field}"
    elif has_new_format:
        if 'routes' not in station:
            return False, "Missing station.routes"
        if not isinstance(station['stations'], list) or len(station['stations']) == 0:
            return False, "stations must be a non-empty list"
        
        for i, stop_pair in enumerate(station['stations']):
            if not isinstance(stop_pair, dict):
                return False, f"Station {i+1}: Each station must be an object"
            if 'uptown' not in stop_pair or not stop_pair['uptown']:
                return False, f"Station {i+1}: Missing 'uptown' stop ID"
            if 'downtown' not in stop_pair or not stop_pair['downtown']:
                return False, f"Station {i+1}: Missing 'downtown' stop ID"
    else:
        return False, "Missing station configuration."
    
    if 'routes' in station:
        if not isinstance(station['routes'], list) or len(station['routes']) == 0:
            return False, "Routes must be a non-empty list when specified"
        station['routes'] = [str(r) for r in station['routes']]
    
    display_fields = ['brightness', 'max_trains', 'show_alerts']
    for field in display_fields:
        if field not in config_data['display']:
            return False, f"Missing display.{field}"
    
    if not (0.0 <= config_data['display']['brightness'] <= 1.0):
        return False, "Brightness must be between 0.0 and 1.0"
    
    if not (1 <= config_data['display']['max_trains'] <= 20):
        return False, "Max trains must be between 1 and 20"
    
    config_data['refresh'] = {
        'trains_interval': 30,
        'alerts_interval': 60
    }
    
    try:
        with open(CONFIG_FILE, 'w') as f:
            json.dump(config_data, f, indent=2)
        
        # Hot-reload will pick up changes automatically within 5 seconds
        return True, "Configuration saved successfully. Changes will apply within 5 seconds."
    except Exception as e:
        return False, f"Error saving configuration: {e}"


def load_stations():
    """Load station reference data"""
    try:
        if STATIONS_FILE.exists():
            with open(STATIONS_FILE, 'r') as f:
                return json.load(f)
        else:
            return {"stations": []}
    except Exception as e:
        print(f"Error loading stations: {e}")
        return {"stations": []}


# Simple TTL cache for subprocess results to avoid spawning processes on every poll
_subprocess_cache = {}
_SUBPROCESS_CACHE_TTL = 10  # seconds


def _cached_subprocess(key, cmd):
    """Run a subprocess with 10-second TTL caching."""
    now = _time.monotonic()
    entry = _subprocess_cache.get(key)
    if entry and now - entry[1] < _SUBPROCESS_CACHE_TTL:
        return entry[0]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=5)
        value = result.stdout.strip()
    except Exception:
        value = 'unknown'
    _subprocess_cache[key] = (value, now)
    return value


def get_display_status():
    """Check if the display service is running (cached 10s)."""
    return _cached_subprocess('display_status', ['systemctl', 'is-active', 'subway-sign.service'])


def get_system_uptime():
    """Get system uptime (cached 10s)."""
    value = _cached_subprocess('uptime', ['uptime', '-p'])
    return value.replace('up ', '') if value != 'unknown' else value


def restart_display_service():
    """Restart the display service (requires sudo privileges)"""
    try:
        # Fire the restart command without waiting for full completion
        # Use start_new_session=True to prevent zombie processes
        proc = subprocess.Popen(
            ['sudo', 'systemctl', 'restart', 'subway-sign.service'],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True  # Detach from parent to prevent zombie
        )

        # Give it a moment to begin restarting
        _time.sleep(1)

        # Reap the process if it's done (non-blocking)
        proc.poll()

        # Check if service is restarting or running
        result = subprocess.run(
            ['systemctl', 'is-active', 'subway-sign.service'],
            capture_output=True,
            text=True,
            timeout=5
        )

        # "activating" or "active" means restart is working
        status = result.stdout.strip()
        if status in ["active", "activating"]:
            return True, "Service restarting successfully"
        else:
            return False, f"Service status: {status}"

    except Exception as e:
        return False, str(e)


@app.route('/')
def index():
    """Main control interface page"""
    return render_template('index.html')


@app.route('/api/config', methods=['GET'])
def get_config():
    """Get current configuration"""
    config = load_config()
    
    last_modified = None
    if CONFIG_FILE.exists():
        last_modified = datetime.fromtimestamp(CONFIG_FILE.stat().st_mtime).isoformat()
    
    return jsonify({
        'success': True,
        'config': config,
        'last_modified': last_modified
    })


@app.route('/api/config', methods=['POST'])
def update_config():
    """Update configuration"""
    try:
        new_config = request.json
        success, message = save_config(new_config)
        
        return jsonify({
            'success': success,
            'message': message
        }), 200 if success else 400
    except Exception as e:
        return jsonify({
            'success': False,
            'message': f"Error: {str(e)}"
        }), 400


@app.route('/api/status')
def get_status():
    """Get display service status"""
    config = load_config()
    service_status = get_display_status()
    
    status_map = {
        'active': ('Running', 'running'),
        'inactive': ('Stopped', 'stopped'),
        'failed': ('Failed', 'failed'),
        'activating': ('Starting', 'starting'),
        'deactivating': ('Stopping', 'stopping'),
        'unknown': ('Unknown', 'unknown')
    }
    status_text, status_class = status_map.get(service_status, ('Unknown', 'unknown'))
    
    stations = load_stations()
    current_station = None
    station_config = config.get('station', {})
    
    if 'station_name' in station_config and station_config['station_name']:
        current_station = station_config['station_name']
    elif 'display_name' in station_config:
        current_station = station_config['display_name']
    elif 'stations' in station_config and station_config['stations']:
        # Multi-platform config - try to look up station name from first stop ID
        first_stop = station_config['stations'][0].get('uptown', '').rstrip('NS')
        for s in COMPLETE_STATIONS:
            stop_ids = [sid.rstrip('NS') for sid in s.get('stop_ids', [])]
            if first_stop in stop_ids:
                current_station = s['name']
                break
        if not current_station:
            current_station = f"{len(station_config['stations'])} platforms configured"
    else:
        for station in stations.get('stations', []):
            if (station.get('uptown_id') == station_config.get('uptown_stop_id') and
                station.get('downtown_id') == station_config.get('downtown_stop_id')):
                current_station = station['name']
                break
    
    uptime = get_system_uptime()
    
    routes = station_config.get('routes', [])
    routes = [str(r) for r in routes] if routes else []
    
    return jsonify({
        'success': True,
        'status': {
            'service': status_text,
            'status_class': status_class,
            'station': current_station or "Not configured",
            'routes': routes,
            'brightness': config.get('display', {}).get('brightness', 0.3),
            'max_trains': config.get('display', {}).get('max_trains', 7),
            'last_update': datetime.fromtimestamp(CONFIG_FILE.stat().st_mtime).isoformat() if CONFIG_FILE.exists() else None,
            'uptime': uptime
        }
    })


@app.route('/api/stations/complete', methods=['GET'])
def get_complete_stations():
    """Get complete station database for auto-lookup."""
    search = request.args.get('search', '').lower()
    route = request.args.get('route', '')
    multi_platform_only = request.args.get('multi_platform_only', '').lower() == 'true'
    
    filtered = COMPLETE_STATIONS
    
    if search:
        filtered = [s for s in filtered if search in s['name'].lower()]
    
    if route:
        filtered = [s for s in filtered if route in s.get('routes', [])]
    
    if multi_platform_only:
        filtered = [s for s in filtered if s.get('platform_count', 1) > 1]
    
    return jsonify({
        'success': True,
        'stations': filtered,
        'total': len(filtered),
        'database_total': len(COMPLETE_STATIONS)
    })


@app.route('/api/stations/lookup/<station_name>', methods=['GET'])
def lookup_station(station_name):
    """Look up stop IDs for a specific station name."""
    stop_ids = get_stop_ids_for_station(station_name)
    
    if stop_ids:
        station_data = None
        for s in COMPLETE_STATIONS:
            if s['name'].lower() == station_name.lower():
                station_data = s
                break
        
        return jsonify({
            'success': True,
            'station_name': station_name,
            'stop_ids': stop_ids,
            'platform_count': len(stop_ids) // 2,
            'routes': station_data.get('routes', []) if station_data else [],
            'lat': station_data.get('lat') if station_data else None,
            'lon': station_data.get('lon') if station_data else None
        })
    else:
        return jsonify({
            'success': False,
            'error': f"Station '{station_name}' not found in database",
            'suggestion': 'Try searching with /api/stations/complete?search=<partial_name>'
        }), 404


@app.route('/api/stations', methods=['GET'])
def get_stations():
    """Get station list with optional search/filter"""
    stations_data = load_stations()
    stations = stations_data.get('stations', [])
    
    search = request.args.get('search', '').lower()
    route = request.args.get('route', '')
    borough = request.args.get('borough', '')
    
    filtered = stations
    
    if search:
        filtered = [s for s in filtered if search in s['name'].lower()]
    
    if route:
        filtered = [s for s in filtered if route in s.get('routes', [])]
    
    if borough:
        filtered = [s for s in filtered if s.get('borough', '') == borough]
    
    return jsonify({
        'success': True,
        'stations': filtered,
        'total': len(filtered)
    })


@app.route('/api/restart', methods=['POST'])
def restart_display():
    """Restart the display service"""
    success, message = restart_display_service()
    return jsonify({
        'success': success,
        'message': message
    }), 200 if success else 500


@app.route('/favicon.ico')
def favicon():
    """Serve favicon"""
    return send_from_directory(
        os.path.join(app.root_path, 'static'),
        'favicon.ico',
        mimetype='image/vnd.microsoft.icon'
    )


@app.route('/static/manifest.json')
def manifest():
    """Serve PWA manifest"""
    return send_from_directory(
        os.path.join(app.root_path, 'static'),
        'manifest.json',
        mimetype='application/manifest+json'
    )


if __name__ == '__main__':
    print("=" * 60)
    print("NYC MTA Subway Sign V3 - Web Control Interface")
    print("=" * 60)
    print(f"Config file: {CONFIG_FILE}")
    print(f"Stations file: {STATIONS_FILE}")
    print(f"Complete stations: {len(COMPLETE_STATIONS)} loaded")
    print(f"Server starting on http://0.0.0.0:5001")
    print("Access from local network: http://ledpi.local:5001")
    print("Press Ctrl+C to stop")
    print("=" * 60)
    print(f"Started at: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print("=" * 60)

    app.run(host='0.0.0.0', port=5001, debug=False)
