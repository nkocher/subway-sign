#!/usr/bin/env python3
"""
Entry point for Subway Sign V2.

Simple wrapper that launches the main application.
"""
import sys
import os
from pathlib import Path

# Add project root and src to path
project_root = Path(__file__).parent.absolute()
sys.path.insert(0, str(project_root))
sys.path.insert(0, str(project_root / 'src'))

# Change to project directory for relative paths
os.chdir(project_root)

from src.main import main

if __name__ == '__main__':
    main()
