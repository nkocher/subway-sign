#!/usr/bin/env python3
"""
Font Comparison Tool

Compares the current MTA bitmap font (mta-sign.json) with the ColeWorks
FontStruct reference font (mta-countdown-clock-letters-numbers.otf.woff2).

Renders both fonts side-by-side and identifies differences.
"""

import json
import os
import sys
from pathlib import Path

# Add project root to path for imports
project_root = Path(__file__).parent.parent
sys.path.insert(0, str(project_root))

from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont

# Paths
ASSETS_DIR = project_root / "assets" / "fonts"
MTA_SIGN_JSON = ASSETS_DIR / "mta-sign.json"
REFERENCE_FONT = ASSETS_DIR / "mta-countdown-clock-letters-numbers.otf.woff2"
OUTPUT_DIR = project_root / "tools" / "font_comparison"


def load_mta_json_font(json_path):
    """Load the MTA bitmap font from JSON"""
    with open(json_path, 'r') as f:
        data = json.load(f)

    # Extract character data (filter out metadata and route icons)
    chars = {}
    for k, v in data.items():
        if k.isdigit():
            chars[chr(int(k))] = v

    return chars


def render_json_char(bitmap_rows, scale=1):
    """Render a character from JSON bitmap data to PIL Image"""
    if not bitmap_rows:
        return None

    height = len(bitmap_rows)

    # Find maximum width
    max_width = 0
    for row in bitmap_rows:
        if row > 0:
            max_width = max(max_width, row.bit_length())

    if max_width == 0:
        max_width = 1

    # Create image
    img = Image.new('L', (max_width * scale, height * scale), 0)

    # Draw pixels (LSB-first: bit 0 = leftmost pixel)
    for y, row_val in enumerate(bitmap_rows):
        for x in range(max_width):
            if row_val & (1 << x):
                # Draw scaled pixel
                for sy in range(scale):
                    for sx in range(scale):
                        px = x * scale + sx
                        py = y * scale + sy
                        if px < img.width and py < img.height:
                            img.putpixel((px, py), 255)

    return img


def render_reference_char_bitmap(font_path, char, target_height=16):
    """
    Render a character from the reference font to a pure bitmap at target size.
    Returns a 16-row grayscale image that can be thresholded to bitmap.
    """
    # Size 12 gives ~11px character height, matching our JSON font
    font_size = 12

    try:
        pil_font = ImageFont.truetype(str(font_path), font_size)
    except Exception as e:
        print(f"Failed to load font: {e}")
        return None

    bbox = pil_font.getbbox(char)
    if bbox is None:
        return None

    # Get the actual character dimensions
    char_width = bbox[2] - bbox[0]
    char_height = bbox[3] - bbox[1]

    # Create a canvas the size of the character plus padding
    # Our JSON font uses 16 rows with content starting at row 1
    canvas_width = max(char_width + 2, 10)
    canvas_height = target_height

    # Use RGB mode for proper antialiasing, then convert
    img = Image.new('RGB', (canvas_width, canvas_height), (0, 0, 0))
    draw = ImageDraw.Draw(img)

    # Position character: start at x=0, y=1 (row 1 in 0-indexed)
    # Adjust for the font's baseline offset
    x_offset = -bbox[0]  # Compensate for any left bearing
    y_offset = 1 - bbox[1]  # Start at row 1, compensate for top bearing

    draw.text((x_offset, y_offset), char, font=pil_font, fill=(255, 255, 255))

    # Convert to grayscale
    img = img.convert('L')

    return img


def extract_bitmap_from_image(img, threshold=30):
    """Convert a grayscale image to a list of row values (like our JSON format)

    Uses a low threshold (30) to catch antialiased font edges.
    """
    if img is None:
        return None

    rows = []
    for y in range(img.height):
        row_val = 0
        for x in range(img.width):
            if img.getpixel((x, y)) > threshold:
                row_val |= (1 << x)  # LSB = leftmost pixel
        rows.append(row_val)

    return rows


def normalize_bitmap(rows):
    """Normalize a bitmap by removing leading/trailing empty rows and left-aligning"""
    if not rows:
        return [], 0, 0

    # Find first and last non-zero rows
    first_row = next((i for i, r in enumerate(rows) if r != 0), 0)
    last_row = next((i for i in range(len(rows) - 1, -1, -1) if rows[i] != 0), len(rows) - 1)

    # Trim rows
    trimmed = rows[first_row:last_row + 1]

    # Find minimum left shift to left-align
    if trimmed:
        # Find the rightmost bit position (leftmost content)
        min_shift = float('inf')
        for r in trimmed:
            if r:
                # Find position of least significant set bit
                lsb_pos = (r & -r).bit_length() - 1
                min_shift = min(min_shift, lsb_pos)

        if min_shift == float('inf'):
            min_shift = 0

        # Shift all rows to left-align
        aligned = [r >> min_shift for r in trimmed]
    else:
        aligned = trimmed
        min_shift = 0

    return aligned, first_row, min_shift


def compare_bitmaps(json_rows, ref_rows):
    """Compare two bitmap row lists and return differences"""
    if json_rows is None or ref_rows is None:
        return {"error": "missing data"}

    # Normalize both bitmaps
    json_norm, json_top, json_left = normalize_bitmap(json_rows)
    ref_norm, ref_top, ref_left = normalize_bitmap(ref_rows)

    # Compare normalized versions
    max_height = max(len(json_norm), len(ref_norm))
    json_padded = json_norm + [0] * (max_height - len(json_norm))
    ref_padded = ref_norm + [0] * (max_height - len(ref_norm))

    diff_rows = []
    total_diff_bits = 0
    shape_diff_bits = 0  # Differences after normalization

    for y, (j, r) in enumerate(zip(json_padded, ref_padded)):
        xor = j ^ r
        if xor:
            diff_rows.append(y)
            shape_diff_bits += bin(xor).count('1')

    # Also compute raw differences
    max_height_raw = max(len(json_rows), len(ref_rows))
    json_raw = json_rows + [0] * (max_height_raw - len(json_rows))
    ref_raw = ref_rows + [0] * (max_height_raw - len(ref_rows))
    for j, r in zip(json_raw, ref_raw):
        xor = j ^ r
        if xor:
            total_diff_bits += bin(xor).count('1')

    return {
        "identical": total_diff_bits == 0,
        "shape_match": shape_diff_bits == 0,  # Same shape after alignment
        "diff_bits": total_diff_bits,
        "shape_diff_bits": shape_diff_bits,
        "diff_rows": diff_rows,
        "json_rows": json_rows,
        "ref_rows": ref_rows,
        "json_norm": json_norm,
        "ref_norm": ref_norm,
        "json_offset": (json_top, json_left),
        "ref_offset": (ref_top, ref_left),
    }


def create_comparison_grid(json_chars, ref_font_path, chars_to_compare, scale=3):
    """Create a side-by-side comparison grid with pixel-level diff"""
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)

    results = {}
    differences = []

    cell_width = 120  # Pixels per cell
    cell_height = 80
    cols = 8
    rows = (len(chars_to_compare) + cols - 1) // cols

    grid_width = cols * cell_width
    grid_height = rows * cell_height

    # Create comparison grid image
    grid = Image.new('RGB', (grid_width, grid_height), (20, 20, 20))
    draw = ImageDraw.Draw(grid)

    try:
        label_font = ImageFont.truetype("/System/Library/Fonts/Menlo.ttc", 10)
    except Exception:
        label_font = ImageFont.load_default()

    for i, char in enumerate(chars_to_compare):
        col = i % cols
        row = i // cols
        x = col * cell_width
        y = row * cell_height

        # Render JSON version
        json_bitmap = json_chars.get(char)
        json_img = None
        if json_bitmap:
            json_img = render_json_char(json_bitmap, scale=scale)

        # Render reference version
        ref_raw = render_reference_char_bitmap(ref_font_path, char)
        ref_bitmap = extract_bitmap_from_image(ref_raw)
        ref_img = None
        if ref_bitmap:
            ref_img = render_json_char(ref_bitmap, scale=scale)  # Use same renderer for consistency

        # Compare
        comparison = compare_bitmaps(json_bitmap, ref_bitmap)

        # Color code the cell based on match status
        if comparison.get("identical"):
            border_color = (0, 150, 0)  # Green = match
        elif comparison.get("error"):
            border_color = (100, 100, 100)  # Gray = missing
        else:
            border_color = (200, 100, 0)  # Orange = difference
            differences.append((char, comparison))

        # Draw cell border
        draw.rectangle([x, y, x + cell_width - 1, y + cell_height - 1], outline=border_color, width=2)

        # Draw character label
        label = repr(char) if char.isspace() else char
        status = "OK" if comparison.get("identical") else f"{comparison.get('diff_bits', '?')}px"
        draw.text((x + 5, y + 2), f"{label} {status}", fill=(180, 180, 180), font=label_font)

        # Place images side by side
        img_y = y + 16

        if json_img:
            # Amber for ours
            json_rgb = Image.new('RGB', json_img.size, (20, 20, 20))
            for py in range(json_img.height):
                for px in range(json_img.width):
                    if json_img.getpixel((px, py)) > 127:
                        json_rgb.putpixel((px, py), (255, 180, 0))
            if x + 5 + json_rgb.width < grid_width and img_y + json_rgb.height < grid_height:
                grid.paste(json_rgb, (x + 5, img_y))

        if ref_img:
            # Cyan for reference
            ref_rgb = Image.new('RGB', ref_img.size, (20, 20, 20))
            for py in range(ref_img.height):
                for px in range(ref_img.width):
                    if ref_img.getpixel((px, py)) > 127:
                        ref_rgb.putpixel((px, py), (0, 200, 255))
            ref_x = x + 60
            if ref_x + ref_rgb.width < grid_width and img_y + ref_rgb.height < grid_height:
                grid.paste(ref_rgb, (ref_x, img_y))

        # Draw labels
        draw.text((x + 5, y + 68), "ours", fill=(255, 180, 0), font=label_font)
        draw.text((x + 60, y + 68), "ref", fill=(0, 200, 255), font=label_font)

        results[char] = {
            "has_json": json_bitmap is not None,
            "has_ref": ref_bitmap is not None,
            "identical": comparison.get("identical", False),
            "shape_match": comparison.get("shape_match", False),
            "diff_bits": comparison.get("diff_bits", 0),
            "shape_diff_bits": comparison.get("shape_diff_bits", 0)
        }

    # Save grid
    grid_path = OUTPUT_DIR / "comparison_grid.png"
    grid.save(grid_path)
    print(f"Saved comparison grid to: {grid_path}")

    return results, differences


def print_bitmap(char, bitmap_rows):
    """Print ASCII art representation of a character bitmap"""
    print(f"\nCharacter '{char}' (code {ord(char)}):")

    if not bitmap_rows:
        print("  (no data)")
        return

    max_width = max(row.bit_length() for row in bitmap_rows if row > 0) if any(bitmap_rows) else 1

    for row_idx, row_val in enumerate(bitmap_rows):
        line = ""
        for x in range(max_width):
            if row_val & (1 << x):
                line += "##"
            else:
                line += "  "
        print(f"  {row_idx:2d}: |{line}| ({row_val})")


def main():
    print("=" * 60)
    print("MTA Font Comparison Tool")
    print("=" * 60)

    # Load fonts
    print(f"\nLoading MTA JSON font from: {MTA_SIGN_JSON}")
    json_chars = load_mta_json_font(MTA_SIGN_JSON)
    print(f"  Loaded {len(json_chars)} characters")

    print(f"\nReference font: {REFERENCE_FONT}")
    if not REFERENCE_FONT.exists():
        print("  ERROR: Reference font not found!")
        print("  Please extract mta-countdown-clock-letters-numbers.otf.woff2.zip")
        return 1

    # Check what characters we have
    print("\n" + "=" * 60)
    print("Characters in JSON font:")
    print("=" * 60)

    # Group by type
    uppercase = sorted([c for c in json_chars.keys() if c.isupper()])
    lowercase = sorted([c for c in json_chars.keys() if c.islower()])
    digits = sorted([c for c in json_chars.keys() if c.isdigit()])
    symbols = sorted([c for c in json_chars.keys() if not c.isalnum()])

    print(f"\nUppercase ({len(uppercase)}): {''.join(uppercase)}")
    print(f"Lowercase ({len(lowercase)}): {''.join(lowercase)}")
    print(f"Digits ({len(digits)}): {''.join(digits)}")
    print(f"Symbols ({len(symbols)}): {symbols}")

    # Print a few sample characters as ASCII art
    print("\n" + "=" * 60)
    print("Sample Character Bitmaps (from JSON):")
    print("=" * 60)

    samples = ['A', '0', '1', '4', '7', 'M', 'a', 'm']
    for char in samples:
        if char in json_chars:
            print_bitmap(char, json_chars[char])

    # Try to render comparison
    print("\n" + "=" * 60)
    print("Creating Comparison Grid...")
    print("=" * 60)

    # Characters to compare
    all_chars = list("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz")

    try:
        results, differences = create_comparison_grid(json_chars, REFERENCE_FONT, all_chars)

        # Summary
        print("\n" + "=" * 60)
        print("Comparison Summary:")
        print("=" * 60)

        identical = sum(1 for r in results.values() if r.get('identical'))
        shape_match = sum(1 for r in results.values() if r.get('shape_match') and not r.get('identical'))
        shape_diff = sum(1 for r in results.values() if r['has_json'] and r['has_ref'] and not r.get('shape_match'))
        only_json = sum(1 for r in results.values() if r['has_json'] and not r['has_ref'])
        only_ref = sum(1 for r in results.values() if not r['has_json'] and r['has_ref'])

        print(f"  Identical (exact match): {identical}")
        print(f"  Same shape (offset only): {shape_match}")
        print(f"  Different shape: {shape_diff}")
        print(f"  Only in JSON (ours): {only_json}")
        print(f"  Only in reference: {only_ref}")

        # List missing from JSON
        missing_from_json = [c for c, r in results.items() if not r['has_json']]
        if missing_from_json:
            print(f"\n  Missing from our JSON font: {''.join(missing_from_json)}")

        # Show detailed differences
        if differences:
            # Separate shape differences from offset-only differences
            shape_diffs = [(c, d) for c, d in differences if d.get('shape_diff_bits', 0) > 0]
            offset_only = [(c, d) for c, d in differences if d.get('shape_diff_bits', 0) == 0]

            if shape_diffs:
                print("\n" + "=" * 60)
                print("Characters with SHAPE Differences (need fixing):")
                print("=" * 60)

                for char, diff in sorted(shape_diffs, key=lambda x: -x[1].get('shape_diff_bits', 0)):
                    print(f"\n'{char}' (code {ord(char)}): {diff.get('shape_diff_bits', 0)} shape pixels differ")
                    print(f"  Rows with differences: {diff.get('diff_rows', [])}")
                    print(f"  Ours offset: top={diff['json_offset'][0]}, left={diff['json_offset'][1]}")
                    print(f"  Ref offset:  top={diff['ref_offset'][0]}, left={diff['ref_offset'][1]}")

                    # Show normalized comparison
                    json_norm = diff.get('json_norm', [])
                    ref_norm = diff.get('ref_norm', [])

                    if json_norm and ref_norm:
                        max_h = max(len(json_norm), len(ref_norm))
                        json_padded = json_norm + [0] * (max_h - len(json_norm))
                        ref_padded = ref_norm + [0] * (max_h - len(ref_norm))

                        all_rows = json_padded + ref_padded
                        max_w = max(r.bit_length() for r in all_rows if r > 0) if any(all_rows) else 1

                        print(f"\n  OURS (normalized)         REFERENCE (normalized)")
                        for y in range(max_h):
                            j = json_padded[y] if y < len(json_padded) else 0
                            r = ref_padded[y] if y < len(ref_padded) else 0

                            j_str = ""
                            r_str = ""
                            for x in range(max_w):
                                j_str += "##" if j & (1 << x) else "  "
                                r_str += "##" if r & (1 << x) else "  "

                            diff_marker = " *" if j != r else "  "
                            print(f"  |{j_str}|  |{r_str}|{diff_marker}")

            if offset_only:
                print("\n" + "=" * 60)
                print(f"Characters with OFFSET differences only ({len(offset_only)} chars):")
                print("=" * 60)
                print("  These have the same shape but different positioning.")
                print("  Chars: " + "".join(c for c, _ in sorted(offset_only)))

            # Show old-style diff for remaining
            print("\n" + "=" * 60)
            print("All Characters with Raw Differences:")
            print("=" * 60)

            for char, diff in sorted(differences, key=lambda x: -x[1].get('diff_bits', 0))[:10]:
                print(f"\n'{char}' (code {ord(char)}): {diff.get('diff_bits', 0)} raw pixels, {diff.get('shape_diff_bits', 0)} shape pixels")
                print(f"  Rows with differences: {diff.get('diff_rows', [])}")

                # Show side-by-side ASCII comparison
                json_rows = diff.get('json_rows', [])
                ref_rows = diff.get('ref_rows', [])

                if json_rows and ref_rows:
                    max_h = max(len(json_rows), len(ref_rows))
                    json_padded = json_rows + [0] * (max_h - len(json_rows))
                    ref_padded = ref_rows + [0] * (max_h - len(ref_rows))

                    # Find max width for display
                    all_rows = json_padded + ref_padded
                    max_w = max(r.bit_length() for r in all_rows if r > 0) if any(all_rows) else 1

                    print(f"\n  {'OURS':<{max_w*2+2}}  {'REFERENCE':<{max_w*2+2}}  DIFF")
                    for y in range(max_h):
                        j = json_padded[y] if y < len(json_padded) else 0
                        r = ref_padded[y] if y < len(ref_padded) else 0

                        j_str = ""
                        r_str = ""
                        for x in range(max_w):
                            j_str += "##" if j & (1 << x) else "  "
                            r_str += "##" if r & (1 << x) else "  "

                        diff_marker = " *" if j != r else "  "
                        print(f"  |{j_str}|  |{r_str}|{diff_marker}")

    except Exception as e:
        print(f"Error creating comparison: {e}")
        import traceback
        traceback.print_exc()

    return 0


if __name__ == "__main__":
    sys.exit(main())
