"""Build VectorMagik for the browser and copy it into the website.

    python kit/web/publish_engine.py --site <the website's folder>

Builds kit/web (the desktop app for a canvas) for wasm32-unknown-unknown
(offline, into work/rust-target), checks it in Node against the command
line's drawings (kit/web/js/test.mjs), then copies the module and its page
runtime (js/app.mjs) into <site>/public/engine/, with version.json: the app's
version, which the site's build reads for its pages (the one copy of it).
"""
import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / 'work/rust-target'
# Its own profile (kit/web/Cargo.toml): the published module alone pays for fat LTO.
WASM = TARGET / 'wasm32-unknown-unknown/wasm-publish/vectormagik_web.wasm'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--site', required=True, help="the website's folder (its public/ gets engine/)")
    args = parser.parse_args()
    site = Path(args.site)
    subprocess.run(['cargo', 'build', '--offline', '--profile', 'wasm-publish', '--lib',
                    '--manifest-path', str(ROOT / 'kit/web/Cargo.toml'),
                    '--target', 'wasm32-unknown-unknown', '--target-dir', str(TARGET)], check=True)
    subprocess.run(['node', str(ROOT / 'kit/web/js/test.mjs'), str(WASM)], check=True)
    engine = site / 'public/engine'
    engine.mkdir(parents=True, exist_ok=True)
    shutil.copy2(WASM, engine / 'vectormagik_web.wasm')
    shutil.copy2(ROOT / 'kit/web/js/app.mjs', engine / 'app.mjs')
    manifest = (ROOT / 'kit/app/Cargo.toml').read_text(encoding='utf-8')
    version = re.search(r'^version = "([^"]+)"', manifest, re.M).group(1)
    (engine / 'version.json').write_text(json.dumps({'version': version}) + '\n', encoding='utf-8')
    print(f'engine {WASM.stat().st_size / 1e6:.1f} MB copied into {engine}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
