#!/usr/bin/env python3
"""Downloads the CC0 studio environments and scanned wood the film uses.

    python3 fetch_assets.py /path/to/assets

Everything comes from Poly Haven (https://polyhaven.com, CC0). Files that
are already there are kept, so it is safe to run again. ASSETS.md lists what
it fetches.
"""

import json
import os
import sys
import urllib.request

UA = "film-asset-fetch/1.0"
API = "https://api.polyhaven.com/files/"

# studio environments, for the reflections in brass, acrylic and the table
HDRIS = ["studio_small_03", "studio_kontrast_04"]
# scanned wood, 2K
TEXTURES = ["white_maple_veneer", "ash_veneer"]
MAPS = {"Diffuse": "diff", "Rough": "rough", "Displacement": "disp"}


def get(url):
    req = urllib.request.Request(url, headers={"User-Agent": UA})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def save(url, path):
    if os.path.exists(path) and os.path.getsize(path) > 0:
        return False
    data = get(url)
    with open(path + ".part", "wb") as f:
        f.write(data)
    os.replace(path + ".part", path)
    return True


def main():
    root = sys.argv[1]
    os.makedirs(os.path.join(root, "hdri"), exist_ok=True)
    os.makedirs(os.path.join(root, "tex"), exist_ok=True)
    total = 0
    for name in HDRIS:
        files = json.loads(get(API + name))
        url = files["hdri"]["2k"]["hdr"]["url"]
        path = os.path.join(root, "hdri", name + "_2k.hdr")
        new = save(url, path)
        total += os.path.getsize(path)
        print(("fetched " if new else "have    ") + path)
    for name in TEXTURES:
        files = json.loads(get(API + name))
        for key, short in MAPS.items():
            if key not in files:
                print(f"  {name}: no {key} map")
                continue
            url = files[key]["2k"]["jpg"]["url"]
            path = os.path.join(root, "tex", f"{name}_{short}.jpg")
            new = save(url, path)
            total += os.path.getsize(path)
            print(("fetched " if new else "have    ") + path)
    print(f"{total / 1e6:.0f} MB in {root}")


if __name__ == "__main__":
    main()
