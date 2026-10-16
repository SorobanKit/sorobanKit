"""
Development utility to split an inline `mod test` block from `src/lib.rs` into a separate `src/test.rs` file.

Usage:
    python scripts/extract_tests.py
    # or from repository root:
    python payment_router/scripts/extract_tests.py
"""

import os
from pathlib import Path

# Locate payment_router root regardless of working directory
script_dir = Path(__file__).resolve().parent
router_dir = script_dir.parent

lib_path = router_dir / 'src' / 'lib.rs'
test_path = router_dir / 'src' / 'test.rs'

if not lib_path.exists():
    print(f"Could not find {lib_path}")
    exit(1)

with open(lib_path, 'r', encoding='utf-8') as f:
    lines = f.readlines()

test_start = -1
for i, line in enumerate(lines):
    if line.strip() == '#[cfg(test)]' and i + 1 < len(lines) and lines[i+1].startswith('mod test {'):
        test_start = i
        break

if test_start != -1:
    test_lines = lines[test_start+2 : -1]  # extract inside mod test { ... }
    test_content = "".join(test_lines[:-1])  # omit the last '}'
    
    with open(test_path, 'w', encoding='utf-8') as f:
        f.write(test_content)
    
    lib_content = "".join(lines[:test_start]) + "#[cfg(test)]\nmod test;\n"
    with open(lib_path, 'w', encoding='utf-8') as f:
        f.write(lib_content)
    
    print("Extracted test mod to test.rs")
else:
    print("Could not find inline test mod in lib.rs (tests may already be extracted)")
