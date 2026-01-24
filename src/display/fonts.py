"""
MTA Bitmap Font Renderer
Converts the authentic MTA sign JSON font into renderable images for LED matrices

Updated to support:
- Pre-generated italic font
- Pre-baked route icons with proper metadata loading
- Project-relative path resolution for assets
"""
import json
import os
from PIL import Image, ImageDraw


class MTAFont:
    """Renders authentic MTA bitmap font from JSON definition"""

    def __init__(self, json_path, height=16, scale=1, generate_italic=False, italic_angle=15):
        """
        Initialize MTA font renderer

        Args:
            json_path: Path to mta-sign.json file
            height: Font height in pixels (default 16 from JSON)
            scale: Scaling factor for the font (1=original, 2=2x size, etc.)
            generate_italic: If True, pre-generate italic versions of all characters
            italic_angle: Angle of italic slant in degrees (default 15)
        """
        with open(json_path, 'r') as f:
            self.font_data = json.load(f)

        self.height = height
        self.scale = scale
        self.char_cache = {}  # Cache rendered characters
        self.italic_char_cache = {}  # Cache rendered italic characters

        # Extract character data (filter out metadata)
        self.chars = {
            chr(int(k)): v
            for k, v in self.font_data.items()
            if k.isdigit()
        }

        # Pre-generate italic font if requested
        self.italic_chars = {}
        # Simple midpoint shift: only 1px padding needed
        self.italic_left_padding = 1
        if generate_italic:
            self.italic_chars = self._generate_italic_font(italic_angle)

    def _generate_italic_font(self, italic_angle=15):
        """
        Pre-generate italic versions of all font glyphs using simple midpoint shift

        Algorithm:
        - Divide character at vertical midpoint
        - Top half: shift 1 pixel RIGHT
        - Bottom half: no shift (preserve baseline)

        This creates a subtle, clean italic effect without complexity.

        Args:
            italic_angle: Unused (kept for compatibility)

        Returns:
            Dictionary mapping characters to italic bitmap data
        """
        print(f"Generating italic font with simple 1px midpoint shift...")
        italic_font = {}

        # Simple approach: only 1 pixel shift, only 1 pixel padding
        ITALIC_SHIFT = 1  # Top half shifts 1 pixel right
        ITALIC_PADDING = 1  # Minimal padding to prevent cutoff

        midpoint = self.height // 2  # Divide at middle

        for char, glyph_rows in self.chars.items():
            # Transform with simple midpoint shift
            italic_rows = []

            for y, row_value in enumerate(glyph_rows):
                if y < midpoint:
                    # Top half: shift right by (SHIFT + PADDING)
                    shift_amount = ITALIC_SHIFT + ITALIC_PADDING
                else:
                    # Bottom half: only padding (no italic shift)
                    shift_amount = ITALIC_PADDING

                # Apply shift
                shifted_row = row_value << shift_amount
                italic_rows.append(shifted_row)

            italic_font[char] = italic_rows

        print(f"Generated italic versions of {len(italic_font)} characters")
        print(f"  Midpoint: row {midpoint} (top half shifts 1px, bottom stays at baseline)")
        return italic_font

    def get_char_bitmap(self, char, italic=False):
        """
        Get bitmap data for a character

        Args:
            char: Single character to render
            italic: If True, return italic version (requires generate_italic=True)

        Returns:
            List of lists representing pixel data [[0,1,1,0,...], [1,1,0,0,...], ...]
            Returns None if character not found
        """
        # Choose regular or italic font
        if italic and char in self.italic_chars:
            bitmap_rows = self.italic_chars[char]
        elif char in self.chars:
            bitmap_rows = self.chars[char]
        else:
            # Character not found
            return None
        pixel_data = []

        # Convert each decimal value to binary pixels
        for row_value in bitmap_rows:
            # Convert to binary and extract bits
            bits = []
            val = row_value
            # Find the maximum bit position to determine width
            max_bits = val.bit_length() if val > 0 else 1

            # Extract bits LSB-first (bit 0 = leftmost pixel)
            # Regular font characters use LSB-first encoding
            for i in range(max_bits):
                bits.append(1 if (val & (1 << i)) else 0)

            if bits:  # Only add non-empty rows
                pixel_data.append(bits)

        return pixel_data

    def get_char_width(self, char, italic=False):
        """
        Get the width of a character in pixels

        Args:
            char: Single character
            italic: If True, get width of italic version

        Returns:
            Width in pixels (scaled)
        """
        # Special case: space character needs explicit width
        if char == ' ':
            return 4 * self.scale

        bitmap = self.get_char_bitmap(char, italic=italic)
        if not bitmap:
            return 4 * self.scale  # Default width for unknown chars

        # Find maximum width across all rows
        max_width = max(len(row) for row in bitmap) if bitmap else 0
        return max_width * self.scale

    def get_char_left_padding(self, char, italic=False):
        """
        Get the left padding (empty columns before content) for a character

        Args:
            char: Single character
            italic: If True, measure italic version

        Returns:
            Number of empty columns on the left (scaled)
        """
        bitmap = self.get_char_bitmap(char, italic=italic)
        if not bitmap:
            return 0

        # Find the leftmost column with any pixel
        leftmost = float('inf')
        for row in bitmap:
            for col_idx, pixel in enumerate(row):
                if pixel:
                    leftmost = min(leftmost, col_idx)
                    break  # Found first pixel in this row

        if leftmost == float('inf'):
            # No content found
            return 0

        return leftmost * self.scale

    def measure_text(self, text, spacing=1, italic=False):
        """
        Measure the total width of text string

        Args:
            text: String to measure
            spacing: Pixels between characters (default 1)
            italic: If True, measure italic version

        Returns:
            Total width in pixels (scaled)
        """
        if not text:
            return 0
        total_width = 0
        for char in text:
            total_width += self.get_char_width(char, italic=italic)
        # Add spacing between characters
        if len(text) > 1:
            total_width += spacing * (len(text) - 1)
        return total_width

    def render_char(self, char, color=(255, 255, 255), italic=False):
        """
        Render a single character to a PIL Image

        Args:
            char: Single character to render
            color: RGB tuple for the character color
            italic: If True, render italic version

        Returns:
            PIL Image containing the rendered character
        """
        # Check appropriate cache first
        cache_key = (char, color, self.scale)
        cache = self.italic_char_cache if italic else self.char_cache
        if cache_key in cache:
            return cache[cache_key]

        # Special case: space character - return transparent image with proper width
        if char == ' ':
            img = Image.new('RGBA', (4 * self.scale, self.height * self.scale), (0, 0, 0, 0))
            cache[cache_key] = img
            return img

        bitmap = self.get_char_bitmap(char, italic=italic)
        if not bitmap:
            # Return empty image for unknown characters
            img = Image.new('RGBA', (4 * self.scale, self.height * self.scale), (0, 0, 0, 0))
            cache[cache_key] = img
            return img

        # Calculate dimensions
        width = max(len(row) for row in bitmap) if bitmap else 1
        height = len(bitmap)

        # Create image with scaling
        img = Image.new('RGBA', (width * self.scale, height * self.scale), (0, 0, 0, 0))
        pixels = img.load()

        # Draw pixels
        for y, row in enumerate(bitmap):
            for x, pixel in enumerate(row):
                if pixel:
                    # Draw scaled pixel block
                    for sy in range(self.scale):
                        for sx in range(self.scale):
                            px = x * self.scale + sx
                            py = y * self.scale + sy
                            if px < img.width and py < img.height:
                                pixels[px, py] = color + (255,) if len(color) == 3 else color

        cache[cache_key] = img
        return img

    def render_text(self, text, color=(255, 255, 255), spacing=1, italic=False):
        """
        Render a text string to a PIL Image

        Args:
            text: Text string to render
            color: RGB tuple for the text color
            spacing: Pixels between characters (default 1)
            italic: If True, render italic version

        Returns:
            PIL Image containing the rendered text
        """
        if not text:
            return Image.new('RGBA', (1, self.height * self.scale), (0, 0, 0, 0))

        # Render each character (italic or regular)
        char_images = [self.render_char(char, color, italic=italic) for char in text]

        if italic and len(char_images) > 1:
            # For italic text, calculate per-character overlaps based on actual left padding
            # This ensures consistent 1px gaps regardless of character-specific padding

            # Calculate total width accounting for per-character overlaps
            total_width = char_images[0].width  # First character - no overlap

            for i in range(1, len(text)):
                curr_char = text[i]
                curr_img = char_images[i]

                # Get left padding of current character
                curr_left_padding = self.get_char_left_padding(curr_char, italic=True)

                # With simple midpoint shift: reduce overlap to maintain 1px gaps
                # Padding is typically 2-3px (from 2px shift in top half)
                # We want to overlap less to prevent touching
                # Use: overlap = max(0, padding - 2) to leave ~1px gap
                overlap = max(0, curr_left_padding - 2)
                total_width += curr_img.width - overlap + spacing  # spacing is -1, so this becomes width - overlap - 1

            max_height = max(img.height for img in char_images) if char_images else self.height * self.scale

            # Create composite image
            result = Image.new('RGBA', (total_width, max_height), (0, 0, 0, 0))

            # Paste characters with per-character overlap
            x_offset = 0
            for i, char_img in enumerate(char_images):
                result.paste(char_img, (x_offset, 0), char_img)

                if i < len(char_images) - 1:
                    # Calculate overlap for next character
                    next_char = text[i + 1]
                    next_left_padding = self.get_char_left_padding(next_char, italic=True)
                    # Reduced overlap for simple midpoint shift
                    overlap = max(0, next_left_padding - 2)
                    x_offset += char_img.width - overlap + spacing  # spacing is -1
                else:
                    x_offset += char_img.width

        else:
            # Regular text - use uniform spacing
            effective_spacing = spacing

            # Calculate total width and max height (with spacing between chars)
            total_width = sum(img.width for img in char_images)
            if len(char_images) > 1:
                total_width += effective_spacing * (len(char_images) - 1)  # Add spacing between chars
            max_height = max(img.height for img in char_images) if char_images else self.height * self.scale

            # Create composite image
            result = Image.new('RGBA', (total_width, max_height), (0, 0, 0, 0))

            # Paste characters with spacing
            x_offset = 0
            for i, char_img in enumerate(char_images):
                result.paste(char_img, (x_offset, 0), char_img)
                x_offset += char_img.width
                if i < len(char_images) - 1:  # Add spacing except after last char
                    x_offset += effective_spacing

        return result


class MTAFontWrapper:
    """
    Wrapper class to provide PIL ImageFont-like interface for MTA font
    Makes it easier to replace existing font usage in code
    """

    def __init__(self, json_path, size=16, generate_italic=True, italic_angle=15):
        """
        Initialize font wrapper

        Args:
            json_path: Path to mta-sign.json
            size: Desired font size in pixels (will scale from 16px base)
            generate_italic: If True, pre-generate italic font at initialization
            italic_angle: Angle of italic slant in degrees (default 15)
        """
        # Calculate scale factor from desired size
        base_height = 16
        scale = max(1, round(size / base_height))

        # Create font with italic pre-generation
        self.font = MTAFont(
            json_path,
            height=base_height,
            scale=scale,
            generate_italic=generate_italic,
            italic_angle=italic_angle
        )
        self.size = size
        self.has_italic = generate_italic

        # Load route icon metadata
        self.json_path = json_path
        self.route_icons = {}
        self._load_route_icons()

    def _load_route_icons(self):
        """Load pre-baked route icon glyphs from font JSON"""
        with open(self.json_path, 'r') as f:
            font_data = json.load(f)

        # Load route icon metadata from external file
        # Look in multiple locations for flexibility
        font_dir = os.path.dirname(os.path.abspath(self.json_path))

        # Possible metadata locations (in priority order):
        # 1. ../route_icon_metadata.json (assets/ - one level up from fonts/)
        # 2. route_icon_metadata.json (same directory as font)
        # 3. ../../assets/route_icon_metadata.json (if font is in src/fonts/)

        possible_paths = [
            os.path.join(font_dir, '..', 'route_icon_metadata.json'),
            os.path.join(font_dir, 'route_icon_metadata.json'),
            os.path.join(font_dir, '..', '..', 'assets', 'route_icon_metadata.json'),
        ]
        
        metadata = None
        for metadata_path in possible_paths:
            if os.path.exists(metadata_path):
                try:
                    with open(metadata_path, 'r') as f:
                        metadata = json.load(f)
                    print(f"Loaded route icon metadata from {metadata_path}")
                    break
                except Exception as e:
                    print(f"Warning: Failed to load {metadata_path}: {e}")
        
        if metadata is None:
            # Fallback to hardcoded metadata (14×13 icons, baseline_offset=0)
            # IMPORTANT: All express-capable routes need both CIRCLE and DIAMOND icons
            # Colors: 1/2/3=#ee352e (red), 4/5/6=#00ff00 (green), 7=#b933ad (purple)
            print("WARNING: route_icon_metadata.json not found, using fallback metadata")
            metadata = {
                "ROUTE_1_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#ee352e"},
                "ROUTE_2_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#ee352e"},
                "ROUTE_2_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#ee352e"},
                "ROUTE_3_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#ee352e"},
                "ROUTE_3_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#ee352e"},
                "ROUTE_4_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#00ff00"},
                "ROUTE_4_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#00ff00"},
                "ROUTE_5_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#00ff00"},
                "ROUTE_5_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#00ff00"},
                "ROUTE_6_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#00ff00"},
                "ROUTE_6_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#00ff00"},
                "ROUTE_7_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#b933ad"},
                "ROUTE_7_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#b933ad"},
                # Lettered lines
                "ROUTE_A_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#2850ad"},
                "ROUTE_A_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#2850ad"},
                "ROUTE_D_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#ff6319"},
                "ROUTE_D_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#ff6319"},
                "ROUTE_E_CIRCLE": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#2850ad"},
                "ROUTE_E_DIAMOND": {"width": 14, "height": 13, "baseline_offset": 0, "color": "#2850ad"},
            }

        # Load icon bitmaps from font data and validate
        missing_icons = []
        for icon_name in metadata.keys():
            if icon_name not in font_data:
                missing_icons.append(icon_name)
                continue

            rows = font_data[icon_name]
            meta = metadata[icon_name]

            # Validate row count matches height
            if len(rows) != meta['height']:
                print(f"Warning: Icon {icon_name}: expected {meta['height']} rows, got {len(rows)}")
                continue

            # Convert row data to PIL Image
            width = meta['width']
            height = meta['height']
            img = Image.new('RGBA', (width, height), (0, 0, 0, 0))

            # Parse hex color
            color = meta['color'].lstrip('#')
            color_rgb = tuple(int(color[i:i+2], 16) for i in (0, 2, 4))

            # Render icon from bitmap rows
            # ALL route icons use MSB-first encoding (bit 13 = leftmost pixel)
            # This matches the authentic MTA bitmap format
            for y, row_val in enumerate(rows):
                for x in range(width):
                    # MSB-first: bit (width-1) is leftmost pixel, bit 0 is rightmost
                    if row_val & (1 << (width - 1 - x)):
                        img.putpixel((x, y), color_rgb + (255,))

            self.route_icons[icon_name] = {
                'image': img,
                'width': width,
                'height': height,
                'baseline_offset': meta['baseline_offset'],
                'color': color_rgb
            }

        if missing_icons:
            print(f"Warning: Some route icons missing from font data: {', '.join(missing_icons)}")

        # Debug: Print loaded icons
        print(f"Loaded {len(self.route_icons)} route icons: {sorted(self.route_icons.keys())}")

    def get_route_icon(self, route, is_express=False):
        """
        Get pre-baked route icon image

        Args:
            route: Route number (e.g., '4', '5', 'A', 'D')
            is_express: True for diamond, False for circle

        Returns:
            Dict with 'image', 'width', 'height', 'baseline_offset', 'color'
            or None if icon not found
        """
        shape = "DIAMOND" if is_express else "CIRCLE"
        icon_name = f"ROUTE_{route}_{shape}"

        # If express icon not found, fall back to circle
        if icon_name not in self.route_icons and is_express:
            fallback_name = f"ROUTE_{route}_CIRCLE"
            if fallback_name in self.route_icons:
                return self.route_icons.get(fallback_name)

        return self.route_icons.get(icon_name)

    def getlength(self, text, italic=False):
        """Measure text width (PIL-compatible method)"""
        return self.font.measure_text(text, spacing=-1, italic=italic)

    def getbbox(self, text, italic=False):
        """Get bounding box of text (PIL-compatible method)"""
        width = self.font.measure_text(text, spacing=-1, italic=italic)
        height = self.font.height * self.font.scale
        return (0, 0, width, height)

    def render_to_image(self, text, color=(255, 255, 255), italic=False):
        """
        Render text to PIL Image

        Args:
            text: Text to render
            color: RGB or hex color
            italic: If True, render italic version (requires generate_italic=True)

        Returns:
            PIL Image with rendered text
        """
        # Convert hex color to RGB if needed
        if isinstance(color, str):
            color = color.lstrip('#')
            color = tuple(int(color[i:i+2], 16) for i in (0, 2, 4))

        return self.font.render_text(text, color, spacing=-1, italic=italic)


class MTADrawWrapper:
    """
    Wrapper for PIL ImageDraw that uses MTA font
    Provides a drop-in replacement for draw.text() calls
    """

    def __init__(self, draw, mta_font):
        """
        Initialize draw wrapper

        Args:
            draw: PIL ImageDraw object
            mta_font: MTAFontWrapper instance
        """
        self.draw = draw
        self.mta_font = mta_font
        self._image = draw._image

    def text(self, xy, text, fill=None, font=None):
        """
        Draw text using MTA font

        Args:
            xy: (x, y) position tuple
            text: Text string to draw
            fill: Color (RGB tuple or hex string)
            font: Font to use (MTAFontWrapper instance, or uses default)
        """
        if font is None:
            font = self.mta_font

        # Use the provided font if it's an MTAFontWrapper
        if isinstance(font, MTAFontWrapper):
            text_img = font.render_to_image(text, fill or (255, 255, 255))
            # Paste rendered text onto the main image
            self._image.paste(text_img, xy, text_img)
        else:
            # Fall back to regular PIL drawing for non-MTA fonts
            self.draw.text(xy, text, fill=fill, font=font)

    def textlength(self, text, font=None):
        """
        Get text width

        Args:
            text: Text to measure
            font: Font to use (MTAFontWrapper or PIL font)

        Returns:
            Width in pixels
        """
        if font is None:
            font = self.mta_font

        if isinstance(font, MTAFontWrapper):
            return font.getlength(text)
        else:
            # Fall back to PIL textlength
            return self.draw.textlength(text, font=font)

    def __getattr__(self, name):
        """Forward all other methods to the original draw object"""
        return getattr(self.draw, name)
