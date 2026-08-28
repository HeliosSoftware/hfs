# Build a JupyterLite site (with the sample notebook baked in) into dist/.
# Assets are NOT committed — this reproduces them locally. See README.md.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

uv venv .venv --python 3.12
# jupyter-server is required by `jupyter lite build --contents` to index custom notebooks.
uv pip install --python .venv/Scripts/python.exe jupyterlite-core jupyterlite-pyodide-kernel jupyter-server

# --- Default build ---------------------------------------------------------
# Pyodide loads from cdn.jsdelivr.net AT RUNTIME (an off-origin request), so
# this build would FAIL the HFS no-CDN guard.
& .venv/Scripts/jupyter-lite.exe build --contents content --output-dir dist

# --- Fully-offline (no-CDN) build ------------------------------------------
# Vendors all of Pyodide (~463 MB) so the site makes ZERO off-origin requests
# and passes the HFS no-CDN guard. Uncomment to use:
#
#   $Pyodide = "https://github.com/pyodide/pyodide/releases/download/314.0.5/pyodide-314.0.5.tar.bz2"
#   & .venv/Scripts/jupyter-lite.exe build --contents content --pyodide $Pyodide --output-dir dist

Write-Host ""
Write-Host "Built dist/. Serve it and open the notebook:"
Write-Host "  python -m http.server 8080 --directory dist   # then open http://localhost:8080/lab"
