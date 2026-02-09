#!/usr/bin/env python3
import subprocess
import os
import sys

def check_ffmpeg():
    try:
        subprocess.run(["ffmpeg", "-version"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True
    except FileNotFoundError:
        return False

def convert_audio(input_path, quality=5):
    """Convert audio to Arma-optimized Ogg Vorbis."""
    output_path = os.path.splitext(input_path)[0] + ".ogg"
    print(f"🎵 Converting Audio: {input_path} -> {output_path}")
    
    cmd = [
        "ffmpeg", "-y", "-i", input_path,
        "-c:a", "libvorbis",
        "-q:a", str(quality),
        output_path
    ]
    subprocess.run(cmd, check=True)

def convert_video(input_path, quality=7):
    """Convert video to Arma-optimized Ogg Theora (.ogv)."""
    output_path = os.path.splitext(input_path)[0] + ".ogv"
    print(f"🎬 Converting Video: {input_path} -> {output_path}")
    
    # Arma requires Theora video + Vorbis audio
    cmd = [
        "ffmpeg", "-y", "-i", input_path,
        "-c:v", "libtheora",
        "-q:v", str(quality),
        "-c:a", "libvorbis",
        "-q:a", "5",
        output_path
    ]
    subprocess.run(cmd, check=True)

def check_armake():
    try:
        subprocess.run(["armake", "--version"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True
    except FileNotFoundError:
        return False

def convert_image(input_path):
    """Convert PNG/JPG to Arma-native PAA."""
    output_path = os.path.splitext(input_path)[0] + ".paa"
    print(f"🖼️ Converting Image: {input_path} -> {output_path}")
    
    cmd = [
        "armake", "img2paa", "-f", "-z",
        input_path, output_path
    ]
    subprocess.run(cmd, check=True)

if __name__ == "__main__":
    has_ffmpeg = check_ffmpeg()
    has_armake = check_armake()

    if not has_ffmpeg and not has_armake:
        print("❌ Error: Neither ffmpeg nor armake found.")
        sys.exit(1)

    if len(sys.argv) < 2:
        print("Usage: media_converter.py <file1> <file2> ...")
        sys.exit(1)

    for file_path in sys.argv[1:]:
        if not os.path.exists(file_path):
            print(f"⚠️ File not found: {file_path}")
            continue

        ext = os.path.splitext(file_path)[1].lower()
        if ext in [".wav", ".mp3", ".m4a", ".flac"]:
            if has_ffmpeg: convert_audio(file_path)
            else: print(f"⚠️ Skipping {file_path}: ffmpeg not found")
        elif ext in [".mp4", ".mkv", ".mov", ".avi"]:
            if has_ffmpeg: convert_video(file_path)
            else: print(f"⚠️ Skipping {file_path}: ffmpeg not found")
        elif ext in [".png", ".jpg", ".jpeg"]:
            if has_armake: convert_image(file_path)
            else: print(f"⚠️ Skipping {file_path}: armake not found")
        else:
            print(f"❓ Unknown format for {file_path}")
