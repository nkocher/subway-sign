"""MTA official route colors."""

# MTA Official Colors (as hex strings)
ROUTE_COLORS = {
    # IRT Broadway-Seventh Avenue Line
    '1': '#FF6644',
    '2': '#FF6644',
    '3': '#FF6644',

    # IRT Lexington Avenue Line
    '4': '#00ff00',
    '5': '#00ff00',
    '6': '#00ff00',

    # IRT Flushing Line
    '7': '#CC5588',

    # IRT 42nd Street Shuttle
    'GS': '#998888',

    # IND Eighth Avenue Line
    'A': '#2850ad',
    'C': '#2850ad',
    'E': '#2850ad',

    # IND Sixth Avenue Line
    'B': '#ff6319',
    'D': '#ff6319',
    'F': '#ff6319',
    'M': '#ff6319',

    # IND Crosstown Line
    'G': '#6cbe45',

    # BMT Canarsie Line
    'L': '#a7a9ac',

    # BMT Jamaica Line
    'J': '#996633',
    'Z': '#996633',

    # BMT Broadway Line
    'N': '#fccc0a',
    'Q': '#fccc0a',
    'R': '#fccc0a',
    'W': '#fccc0a',
}

# Express-capable routes
EXPRESS_ROUTES = {'2', '3', '4', '5', '6', '7', 'A', 'D', 'E'}

# Standard display colors
COLOR_GREEN = '#00ff00'
COLOR_RED = '#FF6644'
COLOR_ORANGE = '#ff6319'  # Alert color
COLOR_BLACK = '#000000'


def hex_to_rgb(hex_color: str) -> tuple[int, int, int]:
    """Convert hex color string to RGB tuple."""
    hex_color = hex_color.lstrip('#')
    return tuple(int(hex_color[i:i+2], 16) for i in (0, 2, 4))


def get_route_color(route: str) -> str:
    """Get hex color for a route, with fallback."""
    return ROUTE_COLORS.get(route, COLOR_GREEN)
