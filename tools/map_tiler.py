#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import math
from PIL import Image

def generate_tiles(image_path, output_dir, tile_size=256, max_zoom=5):
    """
    Slices a large Arma 3 map export into a web-standard tile structure.
    Output: {output_dir}/{z}/{x}/{y}.png
    """
    print(f"🗺️  [Map Tiler] Processing: {image_path}")
    
    img = Image.open(image_path)
    width, height = img.size
    
    # Square the image to the nearest power of 2 for optimal tiling if needed
    # But for Arma Simple CRS, we can just slice the original
    
    for zoom in range(max_zoom + 1):
        # Scale factor for this zoom level
        # Zoom 0 is 1:1, higher zoom is smaller? 
        # Actually for Leaflet Simple, we usually zoom OUT from the original.
        # We'll treat the original as the MAX zoom.
        
        current_zoom = max_zoom - zoom
        scale = 1.0 / (2 ** zoom)
        
        z_dir = os.path.join(output_dir, str(current_zoom))
        os.makedirs(z_dir, exist_ok=True)
        
        z_width = int(width * scale)
        z_height = int(height * scale)
        
        print(f"  └─ Zoom {current_zoom}: Scaling to {z_width}x{z_height}...")
        z_img = img.resize((z_width, z_height), Image.LANCZOS)
        
        cols = math.ceil(z_width / tile_size)
        rows = math.ceil(z_height / tile_size)
        
        for x in range(cols):
            x_dir = os.path.join(z_dir, str(x))
            os.makedirs(x_dir, exist_ok=True)
            
            for y in range(rows):
                left = x * tile_size
                top = y * tile_size
                right = min(left + tile_size, z_width)
                bottom = min(top + tile_size, z_height)
                
                tile = z_img.crop((left, top, right, bottom))
                
                # Create background if tile is smaller than tile_size
                if tile.size != (tile_size, tile_size):
                    bg = Image.new('RGBA', (tile_size, tile_size), (0, 0, 0, 0))
                    bg.paste(tile, (0, 0))
                    tile = bg
                
                tile.save(os.path.join(x_dir, f"{y}.png"))
                
    print(f"✅ Tiling complete. Output saved to: {output_dir}")

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: map_tiler.py <input_image> <output_dir>")
        sys.exit(1)
        
    generate_tiles(sys.argv[1], sys.argv[2])
