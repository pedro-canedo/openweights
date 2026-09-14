"""Launch this version's backend with an existing private Python runtime."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from app.__main__ import main

if __name__ == '__main__':
    main()
