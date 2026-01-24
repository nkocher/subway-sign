"""
Display renderer - pure rendering logic with no side effects.

This module handles all frame rendering. It's designed to be:
- Pure: Same inputs always produce same output
- Fast: Aggressive caching of expensive operations
- Simple: No threading, no locks, no I/O
"""
from collections import OrderedDict
from PIL import Image, ImageDraw
from typing import Optional
import re

from mta.models import DisplaySnapshot, Train, Alert
from display.fonts import MTAFontWrapper
from display.colors import get_route_color, COLOR_GREEN, COLOR_RED, COLOR_BLACK, hex_to_rgb


class TextCache:
    """
    LRU cache for rendered text to avoid expensive re-rendering.

    Uses OrderedDict for efficient O(1) LRU eviction without memory churn.
    """

    def __init__(self, maxsize: int = 200):
        self._cache: OrderedDict[tuple, Image.Image] = OrderedDict()
        self._maxsize = maxsize

    def get(self, key: tuple) -> Optional[Image.Image]:
        """Retrieve cached text render, moving to end (most recent)."""
        if key in self._cache:
            # Move to end (mark as recently used)
            self._cache.move_to_end(key)
            return self._cache[key]
        return None

    def put(self, key: tuple, img: Image.Image):
        """Cache a text render with LRU eviction."""
        if key in self._cache:
            # Update existing and move to end
            self._cache.move_to_end(key)
            self._cache[key] = img
        else:
            if len(self._cache) >= self._maxsize:
                # Evict least recently used (first item)
                self._cache.popitem(last=False)
            self._cache[key] = img

    def clear(self):
        """Clear the cache."""
        self._cache.clear()


class Renderer:
    """
    Pure rendering engine for the subway sign display.

    All methods are side-effect free - they take data in and return images,
    with no I/O, no threading, and no shared mutable state.
    """

    def __init__(self, font_path: str, icon_metadata_path: str):
        """
        Initialize renderer with fonts and icons.

        Args:
            font_path: Path to mta-sign.json
            icon_metadata_path: Path to route_icon_metadata.json
        """
        self.font = MTAFontWrapper(font_path, size=16, generate_italic=True)
        self._text_cache = TextCache(maxsize=200)
        self._last_alert_width = 0  # Track width for scroll completion
        self._alert_cache: tuple[str, frozenset[str], Image.Image] | None = None  # (text, routes, image)

        # Pre-load all route icons
        self._icons = self._load_all_icons()

    def _load_all_icons(self) -> dict:
        """Pre-load all route icons into memory."""
        icons = {}
        for route in "1234567ABCDEFGJLMNQRWZ":
            # Load circle icon
            icon_data = self.font.get_route_icon(route, is_express=False)
            if icon_data:
                icons[(route, False)] = icon_data

            # Load diamond icon for express-capable routes
            if route in "234567ADE":
                icon_data = self.font.get_route_icon(route, is_express=True)
                if icon_data:
                    icons[(route, True)] = icon_data

        return icons

    def render_frame(
        self,
        snapshot: DisplaySnapshot,
        cycle_index: int,
        flash_state: bool,
        alert_scroll_offset: float = 0.0,
        show_alert: bool = False,
        current_alert: 'Alert | None' = None
    ) -> Image.Image:
        """
        Render a complete frame.

        This is a pure function - same inputs always produce same output.

        Args:
            snapshot: Data snapshot to render
            cycle_index: Which downtown train to show (0-5)
            flash_state: Whether arrival flash is on/off
            alert_scroll_offset: Horizontal scroll position for alerts
            show_alert: Whether to show alert instead of downtown train
            current_alert: The specific alert to render (from AlertManager)

        Returns:
            192x32 RGB image ready for display
        """
        img = Image.new('RGB', (192, 32), 'black')

        # Render first train (top row) - next arriving regardless of direction
        first_train = snapshot.get_first_train()
        self._render_train_row(
            img,
            train=first_train,
            y_offset=0,
            train_number=1,
            flash_state=flash_state
        )

        # Render cycling trains or alert (bottom row)
        if show_alert and current_alert:
            self._render_alert_row(img, [current_alert], alert_scroll_offset)
        else:
            cycling_trains = snapshot.get_cycling_trains(count=6)
            train_idx = min(cycle_index, len(cycling_trains) - 1)
            self._render_train_row(
                img,
                train=cycling_trains[train_idx],
                y_offset=16,
                train_number=train_idx + 2,
                flash_state=False  # Cycling trains don't flash
            )

        return img

    def _render_train_row(
        self,
        img: Image.Image,
        train: Train,
        y_offset: int,
        train_number: int,
        flash_state: bool
    ):
        """
        Render a single train row.

        Args:
            img: Image to render onto
            train: Train data to render
            y_offset: Y position (0 for uptown, 16 for downtown)
            train_number: Display number (1-7)
            flash_state: Whether to flash the arrival time
        """
        # Both rows need -4px offset to align with V1
        y = y_offset - 4

        # Determine colors
        is_arriving = train.minutes == 0
        if is_arriving and flash_state:
            time_color = COLOR_BLACK  # Flash to black
            text_color = COLOR_RED
        elif is_arriving:
            time_color = COLOR_RED
            text_color = COLOR_RED
        else:
            time_color = COLOR_GREEN
            text_color = COLOR_GREEN

        route_color = get_route_color(train.route)

        # Render train number
        num_text = f"{train_number}."
        num_img = self._render_text(num_text, text_color)
        img.paste(num_img, (-2, y + 4), num_img)

        num_width = num_img.width - 2

        # Render route icon
        icon_x = num_width + 2
        icon_width = 14
        if train.route:
            self._render_route_icon(img, train.route, train.is_express, icon_x, y + 4)

        # Render destination
        station_x = icon_x + icon_width + 3
        time_text = f"{train.minutes if train.minutes < 999 else '---'}min"
        time_width = self.font.getlength(time_text)
        time_x = 192 - time_width

        available_width = int(time_x - station_x - 5)
        dest_text = self._truncate_text(train.destination, available_width)
        dest_img = self._render_text(dest_text, text_color)
        img.paste(dest_img, (station_x, y + 4), dest_img)

        # Render arrival time
        time_img = self._render_text(time_text, time_color)
        img.paste(time_img, (int(time_x), y + 4), time_img)

    def _render_alert_row(self, img: Image.Image, alerts: tuple[Alert, ...], scroll_offset: float):
        """
        Render scrolling alert message with inline route icons.

        Args:
            img: Image to render onto
            alerts: Alert objects
            scroll_offset: Horizontal scroll position in pixels
        """
        if not alerts:
            return

        # Get first alert
        alert = alerts[0]
        alert_text = alert.text

        # Use cached alert image if same alert, otherwise render and cache
        cache_key = (alert_text, alert.affected_routes)
        if self._alert_cache and self._alert_cache[:2] == cache_key:
            alert_img = self._alert_cache[2]
        else:
            alert_img = self._render_alert_with_icons(alert_text, alert.affected_routes)
            self._alert_cache = (alert_text, alert.affected_routes, alert_img)

        # Track width for scroll completion detection
        self._last_alert_width = alert_img.width

        # Create scrolling region - text starts off-screen right, scrolls left
        # No looping - just scroll once across the screen
        x_pos = 192 - int(scroll_offset)  # Start at right edge, move left

        # Only render if still visible (y=15 to fit 17px tall alert in bottom half)
        if x_pos > -alert_img.width:
            img.paste(alert_img, (x_pos, 15), alert_img)

    def get_scroll_complete_distance(self) -> int:
        """
        Get the scroll distance needed for current alert to fully scroll off screen.

        Returns:
            Total pixels to scroll (screen width + text width + padding)
        """
        return 192 + self._last_alert_width + 10

    def _render_alert_with_icons(self, text: str, affected_routes: frozenset[str]) -> Image.Image:
        """
        Render alert text with inline route icons for [route] patterns.

        Args:
            text: Alert text with optional [route] patterns like [1], [A], [6X]
            affected_routes: Routes affected by this alert

        Returns:
            Composite image with text and icons
        """
        # Pattern matches [1], [A], [6X] etc
        pattern = r'\[(\d+|[A-Z]+)([xX])?\]'
        matches = list(re.finditer(pattern, text))

        if not matches:
            # No route patterns, render as simple text
            return self._render_text(text, "#ff6319", italic=True)

        # Build composite image
        parts = []
        last_end = 0

        for match in matches:
            # Text before this icon
            if match.start() > last_end:
                text_part = text[last_end:match.start()]
                parts.append(('text', text_part))

            # Route icon
            route = match.group(1)
            has_express_marker = match.group(2) is not None
            # Express routes that can show diamond icons
            express_routes = {'2', '3', '4', '5', '6', '7', 'A', 'D', 'E'}
            is_express = (route in express_routes) or has_express_marker
            parts.append(('icon', route, is_express))

            last_end = match.end()

        # Text after last icon
        if last_end < len(text):
            text_part = text[last_end:]
            parts.append(('text', text_part))

        # Pre-render all parts and calculate spacing
        # Spacing rules: asymmetric gaps (icons have built-in right padding)
        TEXT_TO_ICON_GAP = 5  # Gap before icon
        ICON_TO_TEXT_GAP = 2  # Gap after icon (icon has ~3px built-in right padding)
        ICON_ICON_GAP = 1

        rendered_parts = []
        for part in parts:
            if part[0] == 'text':
                text_img = self._render_text(part[1], "#ff6319", italic=True)
                rendered_parts.append(('text', text_img))
            elif part[0] == 'icon':
                route, is_express = part[1], part[2]
                icon_data = self._icons.get((route, is_express))
                if not icon_data:
                    icon_data = self._icons.get((route, False))
                if icon_data:
                    rendered_parts.append(('icon', icon_data))

        # Calculate total width with context-aware spacing
        total_width = 0
        for i, (part_type, part_data) in enumerate(rendered_parts):
            prev_type = rendered_parts[i-1][0] if i > 0 else None

            # Add gap before this part based on previous part type
            if prev_type == 'icon' and part_type == 'text':
                total_width += ICON_TO_TEXT_GAP
            elif prev_type == 'icon' and part_type == 'icon':
                total_width += ICON_ICON_GAP
            elif prev_type == 'text' and part_type == 'icon':
                total_width += TEXT_TO_ICON_GAP

            # Add this part's width
            if part_type == 'text':
                total_width += part_data.width
            else:
                total_width += part_data.get('width', 14)

        # Create composite image (17px tall to accommodate diamond icon offset)
        composite = Image.new('RGBA', (total_width, 17), (0, 0, 0, 0))
        x_pos = 0

        for i, (part_type, part_data) in enumerate(rendered_parts):
            prev_type = rendered_parts[i-1][0] if i > 0 else None

            # Add gap before this part
            if prev_type == 'icon' and part_type == 'text':
                x_pos += ICON_TO_TEXT_GAP
            elif prev_type == 'icon' and part_type == 'icon':
                x_pos += ICON_ICON_GAP
            elif prev_type == 'text' and part_type == 'icon':
                x_pos += TEXT_TO_ICON_GAP

            # Render this part
            if part_type == 'text':
                composite.paste(part_data, (x_pos, 1), part_data)  # y=1 to leave room for icon offset
                x_pos += part_data.width
            else:
                icon_img = part_data['image']
                baseline_offset = part_data.get('baseline_offset', 0)
                composite.paste(icon_img, (x_pos, 1 - baseline_offset), icon_img)  # y=1 base, offset shifts up
                x_pos += part_data.get('width', 14)

        return composite

    def _render_route_icon(self, img: Image.Image, route: str, is_express: bool, x: int, y: int):
        """Render a route icon at the specified position."""
        icon_data = self._icons.get((route, is_express))
        if not icon_data:
            # Fallback to circle if express not available
            icon_data = self._icons.get((route, False))

        if icon_data:
            icon_img = icon_data['image']
            baseline_offset = icon_data.get('baseline_offset', 0)
            y_adjusted = y - baseline_offset
            img.paste(icon_img, (x, y_adjusted), icon_img)

    def _render_text(self, text: str, color: str, italic: bool = False) -> Image.Image:
        """
        Render text with caching.

        Args:
            text: Text to render
            color: Hex color string
            italic: Whether to use italic

        Returns:
            RGBA image with rendered text
        """
        # Convert color to RGB tuple
        color_rgb = hex_to_rgb(color)

        # Check cache
        cache_key = (text, color_rgb, italic)
        cached = self._text_cache.get(cache_key)
        if cached:
            return cached

        # Render new
        img = self.font.render_to_image(text, color_rgb, italic=italic)

        # Cache and return
        self._text_cache.put(cache_key, img)
        return img

    def _truncate_text(self, text: str, max_width: int) -> str:
        """Truncate text to fit within max_width pixels."""
        if self.font.getlength(text) <= max_width:
            return text

        while len(text) > 0 and self.font.getlength(text) > max_width:
            text = text[:-1]

        return text
