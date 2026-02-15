#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import fnmatch
import os
import sys
import argparse
from pathlib import Path

def check_config_style(filepath):
    bad_count_file = 0
    try:
        with open(filepath, 'r', encoding='utf-8', errors='ignore') as file:
            content = file.read()
            brackets_list = []
            isInCommentBlock = False
            checkIfInComment = False
            ignoreTillEndOfLine = False
            checkIfNextIsClosingBlock = False
            isInString = False
            inStringType = ''
            lastIsCurlyBrace = False
            lineNumber = 1

            for c in content:
                if (lastIsCurlyBrace): lastIsCurlyBrace = False
                if c == '\n': lineNumber += 1
                if (isInString):
                    if (c == inStringType): isInString = False
                elif (isInCommentBlock == False):
                    if (checkIfInComment):
                        checkIfInComment = False
                        if c == '*': isInCommentBlock = True
                        elif (c == '/'): ignoreTillEndOfLine = True
                    if (isInCommentBlock == False):
                        if (ignoreTillEndOfLine):
                            if (c == '\n'): ignoreTillEndOfLine = False
                        else:
                            if (c == '"' or c == "'"):
                                isInString = True
                                inStringType = c
                            elif (c == '/'): checkIfInComment = True
                            elif (c == '('): brackets_list.append('(')
                            elif (c == ')'):
                                if (len(brackets_list) > 0 and brackets_list[-1] in ['{', '[']):
                                    print(f"ERROR: Possible missing round bracket ')' detected at {filepath} Line number: {lineNumber}")
                                    bad_count_file += 1
                                brackets_list.append(')')
                            elif (c == '['): brackets_list.append('[')
                            elif (c == ']'):
                                if (len(brackets_list) > 0 and brackets_list[-1] in ['{', '(']):
                                    print(f"ERROR: Possible missing square bracket ']' detected at {filepath} Line number: {lineNumber}")
                                    bad_count_file += 1
                                brackets_list.append(']')
                            elif (c == '{'): brackets_list.append('{')
                            elif (c == '}'):
                                lastIsCurlyBrace = True
                                if (len(brackets_list) > 0 and brackets_list[-1] in ['(', '[']):
                                    print(f"ERROR: Possible missing curly brace '}}' detected at {filepath} Line number: {lineNumber}")
                                    bad_count_file += 1
                                brackets_list.append('}')
                else:
                    if (c == '*'): checkIfNextIsClosingBlock = True
                    elif (checkIfNextIsClosingBlock):
                        if (c == '/'): isInCommentBlock = False
                        elif (c != '*'): checkIfNextIsClosingBlock = False

            if brackets_list.count('[') != brackets_list.count(']'):
                print(f"ERROR: A possible missing square bracket [ or ] in file {filepath}")
                bad_count_file += 1
            if brackets_list.count('(') != brackets_list.count(')'):
                print(f"ERROR: A possible missing round bracket ( or ) in file {filepath}")
                bad_count_file += 1
            if brackets_list.count('{') != brackets_list.count('}'):
                print(f"ERROR: A possible missing curly brace {{ or }} in file {filepath}")
                bad_count_file += 1
    except: pass
    return bad_count_file

def main():
    print("Validating Config Style")
    parser = argparse.ArgumentParser()
    parser.add_argument('-m','--module', help='only search specified module addon folder', required=False, default="")
    parser.add_argument('path', nargs='?', default=".", help="Project path to scan")
    args = parser.parse_args()

    bad_count = 0
    sqf_list = []
    
    scan_root = Path(args.path)
    addons_dir = scan_root / "addons"
    target_dir = addons_dir if addons_dir.exists() else scan_root
    
    if args.module:
        target_dir = target_dir / args.module

    if not target_dir.exists():
        print(f"  [!] Target directory not found: {target_dir}")
        return 0

    for root, _, filenames in os.walk(target_dir):
        if ".hemttout" in root or ".uksf_tools" in root: continue
        for filename in filenames:
            if filename.lower().endswith(('.cpp', '.hpp')):
                sqf_list.append(os.path.join(root, filename))

    for filename in sqf_list:
        bad_count += check_config_style(filename)

    print(f"------\nChecked {len(sqf_list)} files\nErrors detected: {bad_count}")
    return bad_count

if __name__ == "__main__":
    sys.exit(main())
