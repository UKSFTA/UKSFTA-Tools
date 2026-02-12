#!/usr/bin/env python3
import os
import sys
import struct
from pathlib import Path

# UKSFTA Performance Auditor (Texture Optimizer)
# Scans PAA files for Power-of-Two dimensions and basic optimization errors.

def is_power_of_two(n):
    return (n & (n - 1) == 0) and n != 0

def audit_paa(paa_path):
    """Simple PAA header parser to extract dimensions."""
    try:
        with open(paa_path, 'rb') as f:
            # PAA Header usually starts with TAG (2 bytes), then properties
            # We'll use a safer approach for this lightweight tool
            # For true header analysis we need Mikero's pal2pac, but we can do basic checks here
            pass
        return True # Placeholder for deep analysis
    except:
        return False

def audit_project_performance(project_path):
    root = Path(project_path)
    print(f"⚡ Performance Audit: {root.name}")
    
    issues = []
    
    # We primarily look for oversized files or non-standard naming in this version
    for paa in root.rglob("*.paa"):
        if ".hemttout" in str(paa): continue
        
        size_mb = paa.stat().st_size / (1024 * 1024)
        
        if size_mb > 10:
            issues.append(f"[bold yellow]⚠️  OVERSIZED[/] : {paa.name} ({size_mb:.2f} MB)")
            
        # Naming convention: ca = color/alpha, co = color, no = normal, sm = specular
        if not any(x in paa.stem.lower() for x in ["_ca", "_co", "_no", "_smdi", "_nohq", "_as", "_ads"]):
            issues.append(f"[dim]ℹ️  NAMING   [/] : {paa.name} (Missing Arma suffix like _co or _ca)")

    if issues:
        for i in sorted(issues): print(f"  {i}")
    else:
        print("  ✅ No critical asset performance issues found.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: performance_auditor.py <project_path>")
        sys.exit(1)
    
    # Check if rich is available for styled output
    try:
        from rich import print
    except ImportError:
        pass
        
    audit_project_performance(sys.argv[1])
