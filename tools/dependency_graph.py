#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import re
import sys
from pathlib import Path

# Soft-import rich for high-fidelity CLI output
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    USE_RICH = True
except ImportError:
    USE_RICH = False

def get_projects():
    parent_dir = Path(__file__).parent.parent.parent
    return [d for d in parent_dir.iterdir() if d.is_dir() and d.name.startswith("UKSFTA-") and (d / ".hemtt" / "project.toml").exists()]

def analyze_dependencies():
    projects = get_projects()
    graph = {} # Project Name -> List of unit dependencies
    all_unit_patches = {} # Patch Name -> Project Name

    # 1. Map all internal unit patches
    for p in projects:
        project_patches = []
        for config in p.rglob("config.cpp"):
            if ".hemttout" in str(config): continue
            with open(config, 'r', errors='ignore') as f:
                content = f.read()
                matches = re.findall(r'class\s+CfgPatches\s*\{[^}]*class\s+([a-zA-Z0-9_]+)', content, re.MULTILINE | re.DOTALL)
                for m in matches:
                    all_unit_patches[m] = p.name
                    project_patches.append(m)
        graph[p.name] = {"patches": project_patches, "deps": set()}

    # 2. Identify internal requirements
    for p in projects:
        for config in p.rglob("config.cpp"):
            if ".hemttout" in str(config): continue
            with open(config, 'r', errors='ignore') as f:
                content = f.read()
                rm = re.search(r'requiredAddons\[\]\s*=\s*\{([^}]*)\}', content, re.MULTILINE | re.DOTALL)
                if rm:
                    reqs = [r.strip().replace('"', '').replace("'", "") for r in rm.group(1).split(',') if r.strip()]
                    for r in reqs:
                        if r in all_unit_patches:
                            target_project = all_unit_patches[r]
                            if target_project != p.name:
                                graph[p.name]["deps"].add(target_project)

    return graph

def find_circular_dependencies(graph):
    circular = []
    for start_node in graph:
        visited = set()
        stack = [(start_node, [start_node])]
        while stack:
            (node, path) = stack.pop()
            for neighbor in graph[node]["deps"]:
                if neighbor == start_node:
                    circular.append(path + [neighbor])
                elif neighbor not in visited:
                    visited.add(neighbor)
                    stack.append((neighbor, path + [neighbor]))
    return circular

def print_report(graph, circular):
    header = "\n🕸️  [Strategic Intelligence] Unit Dependency Map"
    separator = " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print(header)
    print(separator)

    if USE_RICH:
        console = Console()
        table = Table(box=box.ROUNDED, border_style="magenta", header_style="bold cyan")
        table.add_column("Source Project")
        table.add_column("Unit Dependencies")
        for proj, data in sorted(graph.items()):
            deps_str = ", ".join(sorted(list(data["deps"]))) if data["deps"] else "[dim]None[/dim]"
            table.add_row(proj, deps_str)
        console.print(table)
    else:
        for proj, data in sorted(graph.items()):
            deps = ", ".join(list(data["deps"])) or "None"
            print(f"  {proj:<20} -> {deps}")

    if circular:
        print("\n  [🚨 CIRCULAR DEPENDENCY ALERT]")
        for path in circular:
            print(f"    ❌ {' -> '.join(path)}")
    else:
        print("\n  ✅ PASS: No circular dependencies detected.")

    print(separator + "\n")

if __name__ == "__main__":
    g = analyze_dependencies()
    c = find_circular_dependencies(g)
    print_report(g, c)
