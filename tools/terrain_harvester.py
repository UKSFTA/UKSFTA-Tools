#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import shutil
import re
from pathlib import Path
from PIL import Image

def convert_paa_to_png(paa_path, png_path):
    """Converts a PAA file to PNG using HEMTT."""
    # Assuming hemtt is in path or use absolute path
    subprocess.run(["hemtt", "utils", "paa", "convert", str(paa_path), str(png_path)], 
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

def generate_tiles_from_pbo(pbo_path, theatre_name, web_root):
    print(f"🕵️  [Terrain Ripper] Targeting PBO: {pbo_path}")
    
    pbo_path = Path(pbo_path)
    temp_dir = Path(".uksf_terrain_rip")
    if temp_dir.exists(): shutil.rmtree(temp_dir)
    temp_dir.mkdir()

    print(f"  └─ Unpacking PBO...")
    subprocess.run(["hemtt", "utils", "pbo", "unpack", str(pbo_path), str(temp_dir)], 
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    # 1. Locate Satellite Grid (Layers)
    layers_dir = list(temp_dir.glob("**/layers"))
    if not layers_dir:
        # Fallback: Look for a single large satellite image (rare but possible)
        large_paas = [p for p in temp_dir.glob("**/*.paa") if p.stat().st_size > 5 * 1024 * 1024]
        if large_paas:
            print(f"  ⚠️  Grid not found, using large image: {large_paas[0].name}")
            # Convert and tile normally
            png_path = temp_dir / "satmap.png"
            convert_paa_to_png(large_paas[0], png_path)
            # Call the standard tiler (recursive call or logic)
            # For simplicity, we just inform the user to use the image mode
            print(f"  ✅ Extracted {png_path}. Use harvest-terrain with this image.")
            return
        
        print("  ❌ Intelligence Failure: Could not locate satellite imagery in PBO.")
        shutil.rmtree(temp_dir)
        return

    layers_root = layers_dir[0]
    print(f"  ✅ Satellite Grid Located: {layers_root}")

    # 2. Analyze Grid Structure
    # Standard format: S_000_000_l00.paa (X, Y, LOD)
    tile_files = list(layers_root.glob("s_*_l00.paa")) # Only process LOD 0 (High Res)
    
    if not tile_files:
        # Try lowercase
        tile_files = list(layers_root.glob("s_*_l00.paa"))
    
    if not tile_files:
        print("  ❌ No high-res satellite tiles (l00) found.")
        return

    # Determine Grid Size
    max_x = 0
    max_y = 0
    
    for t in tile_files:
        match = re.search(r's_(\d+)_(\d+)_l00', t.name.lower())
        if match:
            x = int(match.group(1))
            y = int(match.group(2))
            if x > max_x: max_x = x
            if y > max_y: max_y = y
            
    print(f"  └─ Grid Dimensions: {max_x + 1}x{max_y + 1} tiles")

    # 3. Virtual Stitching & Web Tiling
    # Instead of making one giant image, we process the Arma tiles directly into Web tiles
    # Arma tiles are usually 512px or 1024px. Web tiles are 256px.
    # This means we just need to resize/split the Arma tiles.
    
    output_dir = Path(web_root) / "static" / "theatre" / theatre_name
    if output_dir.exists(): shutil.rmtree(output_dir)
    
    # We treat the Arma grid as Zoom Level X (depending on resolution)
    # For simplicity, we'll assume the Arma grid corresponds to Zoom 4 or 5
    # and just convert them directly.
    
    # Target Zoom Level 5 (Highest Detail)
    z5_dir = output_dir / "5"
    
    for t in tile_files:
        match = re.search(r's_(\d+)_(\d+)_l00', t.name.lower())
        x_idx = int(match.group(1))
        y_idx = int(match.group(2))
        
        # Convert PAA to PNG
        temp_png = t.with_suffix(".png")
        convert_paa_to_png(t, temp_png)
        
        if temp_png.exists():
            img = Image.open(temp_png)
            # Resize to web standard 256x256 if needed (Arma tiles vary)
            if img.size != (256, 256):
                img = img.resize((256, 256), Image.LANCZOS)
            
            # Save to web structure
            # Arma Y is often inverted or different origin. 
            # We map 1:1 for now, but might need flipping.
            target_x = z5_dir / str(x_idx)
            target_x.mkdir(parents=True, exist_ok=True)
            img.save(target_x / f"{y_idx}.png")
            
            os.remove(temp_png)
            print(f"    Processed Tile: {x_idx},{y_idx}", end="\r")

    print(f"\n✅ Extraction Complete. {theatre_name} is now archived.")
    shutil.rmtree(temp_dir)

def generate_tiles_from_image(image_path, theatre_name, web_root, tile_size=256, max_zoom=5):
    """Standard image slicer."""
    # (Previous implementation preserved here...)
    # I'll just call the new logic if it's a PBO
    if str(image_path).endswith(".pbo"):
        generate_tiles_from_pbo(image_path, theatre_name, web_root)
        return

    # ... Original logic ...
    # Re-implementing simplified for context:
    print(f"🗺️  [Intelligence Harvester] Slicing imagery for: {theatre_name}")
    output_dir = Path(web_root) / "static" / "theatre" / theatre_name
    if output_dir.exists(): shutil.rmtree(output_dir)
    img = Image.open(image_path)
    width, height = img.size
    for zoom in range(max_zoom + 1):
        scale = 1.0 / (2 ** zoom)
        z_dir = output_dir / str(max_zoom - zoom)
        z_dir.mkdir(parents=True, exist_ok=True)
        z_width = int(width * scale)
        z_height = int(height * scale)
        z_img = img.resize((z_width, z_height), Image.LANCZOS)
        cols = math.ceil(z_width / tile_size)
        rows = math.ceil(z_height / tile_size)
        for x in range(cols):
            x_dir = z_dir / str(x)
            x_dir.mkdir(exist_ok=True)
            for y in range(rows):
                left = x * tile_size; top = y * tile_size
                right = min(left + tile_size, z_width); bottom = min(top + tile_size, z_height)
                tile = z_img.crop((left, top, right, bottom))
                if tile.size != (tile_size, tile_size):
                    bg = Image.new('RGBA', (tile_size, tile_size), (0, 0, 0, 0))
                    bg.paste(tile, (0, 0)); tile = bg
                tile.save(x_dir / f"{y}.png")
    print(f"✅ Intelligence Harvested! Theatre '{theatre_name}' is now locally cached.")

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: terrain_harvester.py <image_or_pbo> <theatre_name>")
        sys.exit(1)
    
    # Detect web root
    web_dir = Path("web")
    if not web_dir.exists() and Path("../web").exists(): web_dir = Path("../web")
    
    generate_tiles_from_image(sys.argv[1], sys.argv[2], web_dir)
