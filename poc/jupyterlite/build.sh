#!/usr/bin/env sh
# Build a JupyterLite site (with the sample notebook baked in) into dist/.
# Assets are NOT committed — this reproduces them locally. See README.md.
set -e
cd "$(dirname "$0")"

uv venv .venv --python 3.12
# jupyter-server is required by `jupyter lite build --contents` to index custom notebooks.
uv pip install --python .venv/Scripts/python.exe jupyterlite-core jupyterlite-pyodide-kernel jupyter-server

# --- Default build ---------------------------------------------------------
# Pyodide is loaded from cdn.jsdelivr.net AT RUNTIME. Convenient, but it makes
# an off-origin request, so this build would FAIL the HFS no-CDN guard.
.venv/Scripts/jupyter-lite build --contents content --output-dir dist

# --- Fully-offline (no-CDN) build ------------------------------------------
# Vendors the whole Pyodide distribution into the site (~463 MB) so it makes
# ZERO off-origin requests and passes the HFS no-CDN guard. Uncomment to use:
#
#   PYODIDE="https://github.com/pyodide/pyodide/releases/download/314.0.5/pyodide-314.0.5.tar.bz2"
#   .venv/Scripts/jupyter-lite build --contents content --pyodide "$PYODIDE" --output-dir dist

echo
echo "Built dist/. Serve it and open the notebook:"
echo "  python -m http.server 8080 --directory dist   # then open http://localhost:8080/lab"
