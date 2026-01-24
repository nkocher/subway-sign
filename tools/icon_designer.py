#!/usr/bin/env python3
"""
Route Icon Designer Tool

Interactive tool for designing and testing MTA route icon glyphs.
Displays icons as ASCII art, allows editing, and outputs JSON values.

Usage:
    python icon_designer.py                    # Interactive mode
    python icon_designer.py --show ROUTE_1     # Show specific icon
    python icon_designer.py --compare 1 2 3    # Compare multiple icons side by side
    python icon_designer.py --edit ROUTE_1     # Edit specific icon interactively

Technical notes:
    - Icons use MSB-first encoding (bit 13 = leftmost pixel)
    - Circle icons: 14px wide × 13 rows
    - Circles are "inverted" - lit pixels form background, numerals carved out as dark
"""

import json
import sys
import argparse
from pathlib import Path


# Path to font JSON
FONT_PATH = Path(__file__).parent.parent / "assets" / "fonts" / "mta-sign.json"


# =============================================================================
# DESIGN PATTERNS - Reusable binary patterns for numeral/letter design
# =============================================================================
# All patterns designed for 14px wide icons, centered at cols 4-9 (center = 6.5)
# Numeral area: rows 3-9 (7 rows)

class Patterns:
    """Reusable binary patterns for icon design."""

    # Curves (for round numerals: 2,3,5,6,8,9,0 and letters: C,D,G,O,Q,S)
    TOP_CURVE_4PX     = 0b00000111100000  # ·····████·····  (centered 4px curve)
    BOTTOM_CURVE_4PX  = 0b00000111100000  # Same as top
    FULL_WIDTH_6PX    = 0b00001100110000  # ····██··██····  (6px wide with 2px gap)

    # Horizontal bars
    TOP_BAR_6PX       = 0b00001111110000  # ····██████····  (solid 6px bar)
    CROSSBAR_6PX      = 0b00001111110000  # Same as top bar
    MIDDLE_BAR_5PX    = 0b00001111100000  # ····█████·····  (5px internal bar)
    MIDDLE_BAR_3PX    = 0b00000011100000  # ······███·····  (3px middle, for 3)

    # Vertical stems (2px wide)
    LEFT_2PX          = 0b00001100000000  # ····██········
    RIGHT_2PX         = 0b00000000110000  # ········██····
    CENTER_2PX        = 0b00000011000000  # ······██······  (cols 6-7)
    STEM_RIGHT        = 0b00000001100000  # ·······██·····  (cols 7-8, for 4,7)

    # Empty row
    EMPTY             = 0b00000000000000

    # Circle template (rows 0-12)
    CIRCLE_TEMPLATE = [
        0b00000111100000,  # Row 0:  ·····████·····
        0b00011111111000,  # Row 1:  ···████████···
        0b00111111111100,  # Row 2:  ··██████████··
        0b01111111111110,  # Row 3:  ·████████████·
        0b01111111111110,  # Row 4:  ·████████████·
        0b11111111111111,  # Row 5:  ██████████████
        0b11111111111111,  # Row 6:  ██████████████
        0b11111111111111,  # Row 7:  ██████████████
        0b01111111111110,  # Row 8:  ·████████████·
        0b01111111111110,  # Row 9:  ·████████████·
        0b00111111111100,  # Row 10: ··██████████··
        0b00011111111000,  # Row 11: ···████████···
        0b00000111100000,  # Row 12: ·····████·····
    ]


def load_font():
    """Load the font JSON file."""
    with open(FONT_PATH) as f:
        return json.load(f)


def save_font(data):
    """Save the font JSON file."""
    with open(FONT_PATH, 'w') as f:
        json.dump(data, f, indent=2)


def int_to_binary_row(value: int, width: int = 14) -> str:
    """Convert integer to binary string representation."""
    # MSB-first: bit (width-1) is leftmost
    bits = []
    for i in range(width - 1, -1, -1):
        bits.append('█' if (value >> i) & 1 else '·')
    return ''.join(bits)


def binary_row_to_int(row: str) -> int:
    """Convert binary string representation back to integer."""
    value = 0
    width = len(row)
    for i, char in enumerate(row):
        if char in ('█', '1', '#', 'X', 'x'):
            bit_position = width - 1 - i
            value |= (1 << bit_position)
    return value


def display_icon(name: str, values: list, show_values: bool = True):
    """Display an icon as ASCII art."""
    print(f"\n{name}:")
    print("-" * 20)
    for i, val in enumerate(values):
        row_str = int_to_binary_row(val)
        if show_values:
            print(f"Row {i:2d}: {row_str}  ({val:5d})")
        else:
            print(f"  {row_str}")
    print()


def display_icons_side_by_side(icons: dict, width: int = 14):
    """Display multiple icons side by side for comparison."""
    names = list(icons.keys())
    if not names:
        return

    # Header
    header = "       "
    for name in names:
        # Extract just the route part (e.g., "1" from "ROUTE_1_CIRCLE")
        short_name = name.replace("ROUTE_", "").replace("_CIRCLE", "").replace("_DIAMOND", "◇")
        header += f"{short_name:^{width+2}}"
    print(header)
    print("-" * len(header))

    # Get max rows
    max_rows = max(len(v) for v in icons.values())

    # Print each row
    for row_idx in range(max_rows):
        line = f"Row {row_idx:2d}: "
        for name in names:
            values = icons[name]
            if row_idx < len(values):
                line += int_to_binary_row(values[row_idx], width) + "  "
            else:
                line += " " * width + "  "
        print(line)
    print()


def get_circle_template() -> list:
    """Return the standard 14x13 circle template (all pixels lit)."""
    return [
        0b00000111100000,  # Row 0:  ····████····
        0b00011111111000,  # Row 1:  ··████████··
        0b00111111111100,  # Row 2:  ·██████████·
        0b01111111111110,  # Row 3:  ████████████
        0b01111111111110,  # Row 4:  ████████████
        0b11111111111111,  # Row 5:  ██████████████
        0b11111111111111,  # Row 6:  ██████████████
        0b11111111111111,  # Row 7:  ██████████████
        0b01111111111110,  # Row 8:  ████████████
        0b01111111111110,  # Row 9:  ████████████
        0b00111111111100,  # Row 10: ·██████████·
        0b00011111111000,  # Row 11: ··████████··
        0b00000111100000,  # Row 12: ····████····
    ]


def extract_numeral_mask(icon_values: list, template: list = None) -> list:
    """Extract just the numeral (carved out pixels) from an icon."""
    if template is None:
        template = get_circle_template()

    mask = []
    for i, (icon_val, template_val) in enumerate(zip(icon_values, template)):
        # Numeral is where template has pixel but icon doesn't
        numeral_bits = template_val & ~icon_val
        mask.append(numeral_bits)
    return mask


def apply_numeral_to_template(numeral_mask: list, template: list = None) -> list:
    """Apply a numeral mask to a circle template."""
    if template is None:
        template = get_circle_template()

    result = []
    for template_val, mask_val in zip(template, numeral_mask):
        # Carve out the numeral from the template
        result.append(template_val & ~mask_val)
    return result


def interactive_edit(name: str, values: list) -> list:
    """Interactively edit an icon row by row."""
    print(f"\nEditing {name}")
    print("Enter new row values as: binary (e.g., ··██··), decimal, or 'k' to keep")
    print("Enter 's' to save and exit, 'q' to quit without saving")
    print()

    new_values = values.copy()

    while True:
        display_icon(name, new_values)

        cmd = input("Command (row#, s=save, q=quit): ").strip().lower()

        if cmd == 's':
            return new_values
        elif cmd == 'q':
            return None
        elif cmd.isdigit():
            row_idx = int(cmd)
            if 0 <= row_idx < len(new_values):
                current = int_to_binary_row(new_values[row_idx])
                print(f"Current row {row_idx}: {current} ({new_values[row_idx]})")
                new_val = input("New value (binary/decimal/k): ").strip()

                if new_val.lower() == 'k':
                    continue
                elif new_val.isdigit():
                    new_values[row_idx] = int(new_val)
                elif set(new_val) <= {'·', '█', '.', '#', '0', '1', ' '}:
                    # Parse as binary pattern
                    clean = new_val.replace('.', '·').replace('#', '█').replace('0', '·').replace('1', '█').replace(' ', '')
                    new_values[row_idx] = binary_row_to_int(clean)
                else:
                    print("Invalid input")
            else:
                print(f"Row must be 0-{len(new_values)-1}")
        else:
            print("Unknown command")


def design_numeral_1() -> list:
    """Design variations of the numeral 1 for review."""
    # Current "1" has diagonal notches that look wrong
    # Need clean vertical stem with flag

    variations = {}

    # Variation A: Clean stem, small flag, 4px serif (like Helvetica)
    # Centered in the circle (cols 5-6 for stem)
    variations["1_clean_A"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000110000000,  # Row 3:  ·····██······ (stem 2px)
        0b00001110000000,  # Row 4:  ····███······ (flag)
        0b00000110000000,  # Row 5:  ·····██······
        0b00000110000000,  # Row 6:  ·····██······
        0b00000110000000,  # Row 7:  ·····██······
        0b00000110000000,  # Row 8:  ·····██······
        0b00001111000000,  # Row 9:  ····████····· (4px base)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    # Variation B: Same but shifted 1px right for better centering
    variations["1_clean_B"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000011000000,  # Row 3:  ······██····· (stem 2px)
        0b00000111000000,  # Row 4:  ·····███····· (flag)
        0b00000011000000,  # Row 5:  ······██·····
        0b00000011000000,  # Row 6:  ······██·····
        0b00000011000000,  # Row 7:  ······██·····
        0b00000011000000,  # Row 8:  ······██·····
        0b00000111100000,  # Row 9:  ·····████···· (4px base)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    # Variation C: Wider flag (like real MTA), 5px base
    variations["1_wide_flag"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000110000000,  # Row 3:  ·····██······
        0b00011110000000,  # Row 4:  ···████······ (wider flag)
        0b00000110000000,  # Row 5:  ·····██······
        0b00000110000000,  # Row 6:  ·····██······
        0b00000110000000,  # Row 7:  ·····██······
        0b00000110000000,  # Row 8:  ·····██······
        0b00011111100000,  # Row 9:  ···██████···· (6px base)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    # Variation D: No base serif, clean modern look
    variations["1_no_base"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000110000000,  # Row 3:  ·····██······
        0b00001110000000,  # Row 4:  ····███······ (flag)
        0b00000110000000,  # Row 5:  ·····██······
        0b00000110000000,  # Row 6:  ·····██······
        0b00000110000000,  # Row 7:  ·····██······
        0b00000110000000,  # Row 8:  ·····██······
        0b00000110000000,  # Row 9:  ·····██······
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    # Variation E: No base, centered like clean_B (Helvetica style)
    variations["1_helvetica"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000011000000,  # Row 3:  ······██····· (stem 2px, centered)
        0b00000111000000,  # Row 4:  ·····███····· (flag)
        0b00000011000000,  # Row 5:  ······██·····
        0b00000011000000,  # Row 6:  ······██·····
        0b00000011000000,  # Row 7:  ······██·····
        0b00000011000000,  # Row 8:  ······██·····
        0b00000011000000,  # Row 9:  ······██·····
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    return variations


def design_numeral_2() -> list:
    """Design variations of the numeral 2 for review."""
    variations = {}

    # 5px wide version (current style)
    variations["2_5px"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000111000000,  # Row 3:  ·····███·····  (3px top)
        0b00001101100000,  # Row 4:  ····██·██····  (5px)
        0b00000000100000,  # Row 5:  ········█····
        0b00000011000000,  # Row 6:  ······██·····
        0b00000110000000,  # Row 7:  ·····██······
        0b00001100000000,  # Row 8:  ····██·······
        0b00001111100000,  # Row 9:  ····█████····  (5px base)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    # 6px wide version - wider curves, centered (shifted 1px right)
    variations["2_6px"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000111100000,  # Row 3:  ·····████····  (4px top)
        0b00001100110000,  # Row 4:  ····██··██···  (6px)
        0b00000000110000,  # Row 5:  ········██···
        0b00000001100000,  # Row 6:  ·······██····
        0b00000011000000,  # Row 7:  ······██·····
        0b00000110000000,  # Row 8:  ·····██······  (shifted 1px right)
        0b00001111110000,  # Row 9:  ····██████···  (6px base)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    return variations


def design_numeral_3() -> list:
    """Design variations of the numeral 3 for review."""
    variations = {}

    # 5px wide version (current style)
    variations["3_5px"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000111000000,  # Row 3:  ·····███·····  (3px top)
        0b00001101100000,  # Row 4:  ····██·██····  (5px)
        0b00000000100000,  # Row 5:  ········█····
        0b00000011000000,  # Row 6:  ······██·····  (middle)
        0b00000000100000,  # Row 7:  ········█····
        0b00001101100000,  # Row 8:  ····██·██····  (5px)
        0b00000111000000,  # Row 9:  ·····███·····  (3px bottom)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    # 6px wide version - wider curves, centered (shifted 1px right)
    variations["3_6px"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000111100000,  # Row 3:  ·····████····  (4px top)
        0b00001100110000,  # Row 4:  ····██··██···  (6px)
        0b00000000110000,  # Row 5:  ········██···
        0b00000011100000,  # Row 6:  ······███····  (middle)
        0b00000000110000,  # Row 7:  ········██···
        0b00001100110000,  # Row 8:  ····██··██···  (6px)
        0b00000111100000,  # Row 9:  ·····████····  (4px bottom)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    return variations


def design_numeral_4() -> dict:
    """Design variations of the numeral 4 for review."""
    variations = {}

    # 6px wide version - Helvetica style, centered
    # Diagonal descends from left, crossbar, right stem continues
    P = Patterns
    variations["4_6px"] = [
        P.EMPTY,              # Row 0
        P.EMPTY,              # Row 1
        P.EMPTY,              # Row 2
        0b00000001100000,     # Row 3:  ·······██····  (right stem)
        0b00000011100000,     # Row 4:  ······███····
        0b00000101100000,     # Row 5:  ·····█·██····
        0b00001001100000,     # Row 6:  ····█··██····
        P.CROSSBAR_6PX,       # Row 7:  ····██████···  (6px crossbar)
        0b00000001100000,     # Row 8:  ·······██····
        0b00000001100000,     # Row 9:  ·······██····
        P.EMPTY,              # Row 10
        P.EMPTY,              # Row 11
        P.EMPTY,              # Row 12
    ]

    return variations


def design_numeral_5() -> dict:
    """Design variations of the numeral 5 for review."""
    variations = {}

    P = Patterns
    variations["5_6px"] = [
        P.EMPTY,              # Row 0
        P.EMPTY,              # Row 1
        P.EMPTY,              # Row 2
        P.TOP_BAR_6PX,        # Row 3:  ····██████···  (6px top bar)
        P.LEFT_2PX,           # Row 4:  ····██·······  (left stem)
        P.MIDDLE_BAR_5PX,     # Row 5:  ····█████····  (curve start)
        P.RIGHT_2PX,          # Row 6:  ········██···  (right side)
        P.RIGHT_2PX,          # Row 7:  ········██···
        P.FULL_WIDTH_6PX,     # Row 8:  ····██··██···  (bottom curve)
        P.BOTTOM_CURVE_4PX,   # Row 9:  ·····████····  (close)
        P.EMPTY,              # Row 10
        P.EMPTY,              # Row 11
        P.EMPTY,              # Row 12
    ]

    return variations


def design_numeral_6() -> dict:
    """Design variations of the numeral 6 for review."""
    variations = {}

    P = Patterns
    variations["6_6px"] = [
        P.EMPTY,              # Row 0
        P.EMPTY,              # Row 1
        P.EMPTY,              # Row 2
        P.TOP_CURVE_4PX,      # Row 3:  ·····████····  (top curve)
        P.FULL_WIDTH_6PX,     # Row 4:  ····██··██···
        P.LEFT_2PX,           # Row 5:  ····██·······  (left only)
        P.MIDDLE_BAR_5PX,     # Row 6:  ····█████····  (middle bar)
        P.FULL_WIDTH_6PX,     # Row 7:  ····██··██···
        P.FULL_WIDTH_6PX,     # Row 8:  ····██··██···
        P.BOTTOM_CURVE_4PX,   # Row 9:  ·····████····  (bottom curve)
        P.EMPTY,              # Row 10
        P.EMPTY,              # Row 11
        P.EMPTY,              # Row 12
    ]

    return variations


def design_numeral_7() -> dict:
    """Design variations of the numeral 7 for review."""
    variations = {}

    P = Patterns
    variations["7_6px"] = [
        P.EMPTY,              # Row 0
        P.EMPTY,              # Row 1
        P.EMPTY,              # Row 2
        P.TOP_BAR_6PX,        # Row 3:  ····██████···  (6px top bar)
        P.RIGHT_2PX,          # Row 4:  ········██···
        P.STEM_RIGHT,         # Row 5:  ·······██····
        P.STEM_RIGHT,         # Row 6:  ·······██····
        P.CENTER_2PX,         # Row 7:  ······██·····
        P.CENTER_2PX,         # Row 8:  ······██·····
        P.CENTER_2PX,         # Row 9:  ······██·····
        P.EMPTY,              # Row 10
        P.EMPTY,              # Row 11
        P.EMPTY,              # Row 12
    ]

    return variations


def design_numeral_8() -> dict:
    """Design variations of the numeral 8 for review."""
    variations = {}

    # 6px wide version - two stacked circles
    variations["8_6px"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000111100000,  # Row 3:  ·····████····  (top curve)
        0b00001100110000,  # Row 4:  ····██··██···
        0b00001100110000,  # Row 5:  ····██··██···
        0b00000111100000,  # Row 6:  ·····████····  (middle)
        0b00001100110000,  # Row 7:  ····██··██···
        0b00001100110000,  # Row 8:  ····██··██···
        0b00000111100000,  # Row 9:  ·····████····  (bottom curve)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    return variations


def design_numeral_9() -> dict:
    """Design variations of the numeral 9 for review."""
    variations = {}

    # 6px wide version - like 6 upside down
    variations["9_6px"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000111100000,  # Row 3:  ·····████····  (top curve)
        0b00001100110000,  # Row 4:  ····██··██···
        0b00001100110000,  # Row 5:  ····██··██···
        0b00000111110000,  # Row 6:  ·····█████···  (middle bar)
        0b00000000110000,  # Row 7:  ········██···  (right only)
        0b00001100110000,  # Row 8:  ····██··██···
        0b00000111100000,  # Row 9:  ·····████····  (bottom curve)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    return variations


def design_numeral_0() -> dict:
    """Design variations of the numeral 0 for review."""
    variations = {}

    # 6px wide version - full oval
    variations["0_6px"] = [
        0b00000000000000,  # Row 0
        0b00000000000000,  # Row 1
        0b00000000000000,  # Row 2
        0b00000111100000,  # Row 3:  ·····████····  (top curve)
        0b00001100110000,  # Row 4:  ····██··██···
        0b00001100110000,  # Row 5:  ····██··██···
        0b00001100110000,  # Row 6:  ····██··██···
        0b00001100110000,  # Row 7:  ····██··██···
        0b00001100110000,  # Row 8:  ····██··██···
        0b00000111100000,  # Row 9:  ·····████····  (bottom curve)
        0b00000000000000,  # Row 10
        0b00000000000000,  # Row 11
        0b00000000000000,  # Row 12
    ]

    return variations


def get_numeral_design(numeral: str) -> dict:
    """Get design variations for any numeral 0-9."""
    designers = {
        "0": design_numeral_0,
        "1": design_numeral_1,
        "2": design_numeral_2,
        "3": design_numeral_3,
        "4": design_numeral_4,
        "5": design_numeral_5,
        "6": design_numeral_6,
        "7": design_numeral_7,
        "8": design_numeral_8,
        "9": design_numeral_9,
    }
    if numeral in designers:
        return designers[numeral]()
    return {}


def show_numeral_options(numeral: str):
    """Show all design options for a numeral applied to circle template."""
    template = get_circle_template()

    variations = get_numeral_design(numeral)
    if not variations:
        print(f"No design variations available for '{numeral}'")
        return

    # Map numeral to route key (routes 1-7 are numbered, others use letters)
    if numeral in "1234567":
        route_key = f"ROUTE_{numeral}_CIRCLE"
    else:
        # 0, 8, 9 don't have dedicated routes, but we can still show designs
        route_key = None

    print("\n" + "=" * 60)
    print(f"NUMERAL '{numeral}' DESIGN OPTIONS")
    print("=" * 60)

    # Load current for comparison
    font = load_font()
    icons = {}
    if route_key:
        current = font.get(route_key, [])
        if current:
            icons["CURRENT"] = current

    for name, numeral_mask in variations.items():
        icon = apply_numeral_to_template(numeral_mask, template)
        icons[name] = icon

    display_icons_side_by_side(icons)

    # Show JSON for each
    print("\nJSON values for each option:")
    for name, numeral_mask in variations.items():
        icon = apply_numeral_to_template(numeral_mask, template)
        print(f"\n{name}:")
        print(f"  {json.dumps(icon)}")


def show_numeral_1_options():
    """Show all numeral 1 design options applied to circle template."""
    show_numeral_options("1")


def main():
    parser = argparse.ArgumentParser(description="Route Icon Designer Tool")
    parser.add_argument("--show", help="Show specific icon (e.g., ROUTE_1_CIRCLE)")
    parser.add_argument("--compare", nargs="+", help="Compare icons (e.g., 1 2 3 or ROUTE_1_CIRCLE)")
    parser.add_argument("--edit", help="Edit specific icon interactively")
    parser.add_argument("--design", help="Show design options for numeral (1, 2, or 3)")
    parser.add_argument("--extract", help="Extract numeral mask from icon")
    parser.add_argument("--all-circles", action="store_true", help="Show all circle icons")
    parser.add_argument("--all-diamonds", action="store_true", help="Show all diamond icons")

    args = parser.parse_args()
    font = load_font()

    if args.design:
        show_numeral_options(args.design)
        return

    if args.show:
        name = args.show
        if not name.startswith("ROUTE_"):
            name = f"ROUTE_{name}_CIRCLE"
        if name in font:
            display_icon(name, font[name])
        else:
            print(f"Icon '{name}' not found")
            print("Available icons:", [k for k in font.keys() if k.startswith("ROUTE_")])

    elif args.compare:
        icons = {}
        for item in args.compare:
            if item.startswith("ROUTE_"):
                name = item
            else:
                # Try circle first, then diamond
                name = f"ROUTE_{item}_CIRCLE"
                if name not in font:
                    name = f"ROUTE_{item}_DIAMOND"

            if name in font:
                icons[name] = font[name]
            else:
                print(f"Warning: '{item}' not found")

        if icons:
            display_icons_side_by_side(icons)

    elif args.edit:
        name = args.edit
        if not name.startswith("ROUTE_"):
            name = f"ROUTE_{name}_CIRCLE"

        if name in font:
            new_values = interactive_edit(name, font[name])
            if new_values:
                font[name] = new_values
                save_font(font)
                print(f"Saved {name}")
        else:
            print(f"Icon '{name}' not found")

    elif args.extract:
        name = args.extract
        if not name.startswith("ROUTE_"):
            name = f"ROUTE_{name}_CIRCLE"

        if name in font:
            mask = extract_numeral_mask(font[name])
            print(f"\nNumeral mask extracted from {name}:")
            display_icon("NUMERAL_MASK", mask, show_values=True)
        else:
            print(f"Icon '{name}' not found")

    elif args.all_circles:
        icons = {k: v for k, v in font.items() if k.startswith("ROUTE_") and k.endswith("_CIRCLE")}
        # Sort by route
        sorted_icons = dict(sorted(icons.items()))
        display_icons_side_by_side(sorted_icons)

    elif args.all_diamonds:
        icons = {k: v for k, v in font.items() if k.startswith("ROUTE_") and k.endswith("_DIAMOND")}
        sorted_icons = dict(sorted(icons.items()))
        display_icons_side_by_side(sorted_icons)

    else:
        # Interactive mode - show menu
        print("\nRoute Icon Designer Tool")
        print("=" * 40)
        print("\nCommands:")
        print("  --show ROUTE_1        Show specific icon")
        print("  --compare 1 2 3       Compare icons side by side")
        print("  --edit ROUTE_1        Edit icon interactively")
        print("  --design-1            Show numeral '1' design options")
        print("  --extract 1           Extract numeral mask from icon")
        print("  --all-circles         Show all circle icons")
        print("  --all-diamonds        Show all diamond icons")
        print()

        # Quick preview of numbered routes
        print("Current numbered route icons:")
        icons = {}
        for i in range(1, 8):
            name = f"ROUTE_{i}_CIRCLE"
            if name in font:
                icons[name] = font[name]
        display_icons_side_by_side(icons)


if __name__ == "__main__":
    main()
