"""Build VectorMagik's browser engine and copy it into the website.

    python kit/web/publish_engine.py --site <the website's folder>

Builds kit/web for wasm32-unknown-unknown (offline, into work/rust-target),
checks it in Node against the command line's drawings (kit/web/js/test.mjs),
then copies the module, its loader and worker into <site>/public/engine/ and
the two sample pictures the converter offers into <site>/public/samples/.
"""
import argparse
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = ROOT / 'work/rust-target'
WASM = TARGET / 'wasm32-unknown-unknown/release/vectormagik_web.wasm'
SAMPLES = {
    'logo.png': ROOT / 'kit/fixtures/samples/logo-with-blending.png',
    'cat.png': ROOT / 'kit/fixtures/photos/chelsea.png',
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--site', required=True, help="the website's folder (its public/ gets engine/ and samples/)")
    args = parser.parse_args()
    site = Path(args.site)
    subprocess.run(['cargo', 'build', '--offline', '--release', '--lib',
                    '--manifest-path', str(ROOT / 'kit/web/Cargo.toml'),
                    '--target', 'wasm32-unknown-unknown', '--target-dir', str(TARGET)], check=True)
    subprocess.run(['node', str(ROOT / 'kit/web/js/test.mjs'), str(WASM)], check=True)
    engine = site / 'public/engine'
    engine.mkdir(parents=True, exist_ok=True)
    shutil.copy2(WASM, engine / 'vectormagik_web.wasm')
    for name in ('engine.mjs', 'worker.mjs'):
        shutil.copy2(ROOT / 'kit/web/js' / name, engine / name)
    samples = site / 'public/samples'
    samples.mkdir(parents=True, exist_ok=True)
    for name, source in SAMPLES.items():
        shutil.copy2(source, samples / name)
    print(f'engine {WASM.stat().st_size / 1e6:.1f} MB copied into {engine}')
    return 0


if __name__ == '__main__':
    sys.exit(main())
