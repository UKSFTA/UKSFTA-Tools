# -*- coding: utf-8 -*-
import subprocess
import os
import sys
import shutil
from pathlib import Path

def get_binary_path():
    """Locates the debinarizer binary based on the current OS."""
    tools_root = Path(__file__).parent.parent
    
    if sys.platform == "win32":
        bin_path = tools_root / "bin" / "win-x64" / "debinarizer.exe"
    else:
        bin_path = tools_root / "bin" / "linux-x64" / "debinarizer"

    if bin_path.exists():
        return str(bin_path)
    
    # Fallback to PATH
    fallback = "debinarizer.exe" if sys.platform == "win32" else "debinarizer"
    if shutil.which(fallback):
        return fallback
        
    return None

def run_debinarizer(input_path, output_path=None, show_info=False, show_map=False, recursive=False, rename=None):
    """
    Wraps the C# debinarizer binary.
    rename: tuple of (old, new)
    """
    bin_cmd = get_binary_path()
    if not bin_cmd:
        print("[Error] P3D Debinarizer binary not found in bin/linux-x64/ or PATH.")
        return False

    cmd = [bin_cmd, str(input_path)]
    if output_path:
        cmd.append(str(output_path))
    
    if show_info: cmd.append("-info")
    if show_map: cmd.append("-map")
    if recursive: cmd.append("-r")
    if rename and len(rename) == 2:
        cmd.extend(["-rename", rename[0], rename[1]])

    try:
        # Use subprocess.run for synchronous execution
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.stdout: print(result.stdout.strip())
        if result.stderr: print(result.stderr.strip(), file=sys.stderr)
        return result.returncode == 0
    except Exception as e:
        print(f"[Error] Failed to execute debinarizer: {e}")
        return False

def fix_project_paths(project_path, old_prefix, new_prefix):
    """
    Bulk debinarize and fix paths for an entire HEMTT project.
    """
    project_path = Path(project_path)
    addons_dir = project_path / "addons"
    if not addons_dir.exists():
        print(f"[Error] Addons directory not found in {project_path}")
        return False

    print(f"[*] Bulk migrating paths in {project_path.name}...")
    print(f"[*] {old_prefix} -> {new_prefix}")
    
    # We use recursive mode on the addons directory
    # Output to same directory (in-place fix for P3Ds)
    return run_debinarizer(addons_dir, recursive=True, rename=(old_prefix, new_prefix))

if __name__ == "__main__":
    # Basic CLI wrapper for standalone use
    import argparse
    parser = argparse.ArgumentParser(description="UKSFTA P3D Debinarizer Wrapper")
    parser.add_column = lambda *args, **kwargs: None # Dummy for compatibility with some unit patterns
    
    parser.add_argument("input", help="Input file or directory")
    parser.add_argument("output", nargs="?", help="Output file or directory")
    parser.add_argument("-info", action="store_true", help="Show model info")
    parser.add_argument("-map", action="store_true", help="Show binary structure map")
    parser.add_argument("-r", action="store_true", help="Recursive directory processing")
    parser.add_argument("-rename", nargs=2, metavar=("OLD", "NEW"), help="Rename texture/material paths")
    
    args = parser.parse_args()
    run_debinarizer(args.input, args.output, args.info, args.map, args.r, args.rename)
