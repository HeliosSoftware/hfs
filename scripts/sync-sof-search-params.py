#!/usr/bin/env python3
"""Regenerate data/sql-on-fhir-search-parameters.json from the SQL-on-FHIR IG package.

Downloads the official FHIR npm package for the SQL-on-FHIR IG
(hl7.fhir.uv.sql-on-fhir), extracts every ViewDefinition SearchParameter,
and rewrites data/sql-on-fhir-search-parameters.json as a deterministic
collection Bundle. Run it to pick up a new IG ballot/release, then review
the diff like any other vendored-artifact re-sync.

Transformations applied to the upstream resources (kept deliberately minimal):

1. The generated narrative (`text`) is dropped — it is boilerplate HTML and
   the loader ignores it.
2. Composite `component.definition` canonicals that still use the legacy
   `http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/...` namespace are
   rewritten to the core `http://hl7.org/fhir/SearchParameter/...` canonicals
   that the referenced SearchParameters actually declare in their `url`.
   Upstream publishes the parameters under core-namespace canonicals but the
   composites still reference the legacy namespace, so a verbatim copy would
   leave the composite components unresolvable in our registry (reported
   upstream as https://github.com/HL7/sql-on-fhir/issues/403; once fixed
   there this step becomes a no-op and the script prints nothing for it).

Everything else is byte-for-byte upstream content.

Usage:
  scripts/sync-sof-search-params.py                 # pinned release (default)
  scripts/sync-sof-search-params.py --ci-build      # current CI build
  scripts/sync-sof-search-params.py --package-url URL
  scripts/sync-sof-search-params.py --package-file path/to/package.tgz
"""

import argparse
import io
import json
import sys
import tarfile
import urllib.request
from pathlib import Path

# Published (pinned) package from the FHIR package registry. Bump this when a
# new ballot/release of the IG ships.
DEFAULT_PACKAGE_URL = (
    "https://packages2.fhir.org/web/hl7.fhir.uv.sql-on-fhir-3.0.0-ballot.tgz"
)
# Unpinned CI build of the IG, for previewing upcoming changes.
CI_BUILD_PACKAGE_URL = "https://build.fhir.org/ig/HL7/sql-on-fhir/package.tgz"

REPO_ROOT = Path(__file__).resolve().parent.parent
OUTPUT_PATH = REPO_ROOT / "data" / "sql-on-fhir-search-parameters.json"
BUNDLE_ID = "sql-on-fhir-search-parameters"


def fetch_package(args: argparse.Namespace) -> bytes:
    if args.package_file:
        return Path(args.package_file).read_bytes()
    url = args.package_url or (
        CI_BUILD_PACKAGE_URL if args.ci_build else DEFAULT_PACKAGE_URL
    )
    print(f"Downloading {url}")
    with urllib.request.urlopen(url) as resp:
        return resp.read()


def extract_search_parameters(package_tgz: bytes) -> tuple[list[dict], dict]:
    """Returns (ViewDefinition SearchParameters, package.json manifest)."""
    params = []
    manifest = {}
    with tarfile.open(fileobj=io.BytesIO(package_tgz), mode="r:gz") as tar:
        for member in tar.getmembers():
            name = member.name
            if name == "package/package.json":
                manifest = json.load(tar.extractfile(member))
                continue
            if not (
                name.startswith("package/SearchParameter-") and name.endswith(".json")
            ):
                continue
            resource = json.load(tar.extractfile(member))
            if resource.get("resourceType") != "SearchParameter":
                continue
            if "ViewDefinition" not in resource.get("base", []):
                continue
            params.append(resource)
    params.sort(key=lambda r: r["id"])
    return params, manifest


def patch_composite_components(params: list[dict]) -> None:
    """Point composite components at the canonicals the parameters declare.

    Upstream inconsistency: the SearchParameters' own `url` values use the
    core `http://hl7.org/fhir/SearchParameter/...` namespace, but composite
    `component.definition` values still reference the legacy
    `http://hl7.org/fhir/uv/sql-on-fhir/SearchParameter/...` namespace. The
    registry resolves components by canonical URL, so rewrite each component
    to the canonical some parameter in this set actually declares (matched by
    the trailing `SearchParameter/<id>` segment).
    """
    declared = {p["url"].rsplit("/", 1)[-1]: p["url"] for p in params}
    for param in params:
        for component in param.get("component", []):
            definition = component.get("definition", "")
            tail = definition.rsplit("/", 1)[-1]
            declared_url = declared.get(tail)
            if declared_url and declared_url != definition:
                print(
                    f"  patched {param['id']} component: "
                    f"{definition} -> {declared_url}"
                )
                component["definition"] = declared_url


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    source = parser.add_mutually_exclusive_group()
    source.add_argument(
        "--ci-build",
        action="store_true",
        help="use the current CI build instead of the pinned release",
    )
    source.add_argument("--package-url", help="explicit package.tgz URL")
    source.add_argument("--package-file", help="local package.tgz file")
    parser.add_argument(
        "--check",
        action="store_true",
        help="exit non-zero if the checked-in file is out of date, without writing",
    )
    args = parser.parse_args()

    params, manifest = extract_search_parameters(fetch_package(args))
    if not params:
        print("error: no ViewDefinition SearchParameters found in package", file=sys.stderr)
        return 1
    print(
        f"Extracted {len(params)} ViewDefinition SearchParameters from "
        f"{manifest.get('name', '?')}@{manifest.get('version', '?')}"
    )

    for param in params:
        param.pop("text", None)

    patch_composite_components(params)

    bundle = {
        "resourceType": "Bundle",
        "id": BUNDLE_ID,
        "type": "collection",
        "entry": [{"resource": p} for p in params],
    }
    rendered = json.dumps(bundle, indent=2, ensure_ascii=False) + "\n"

    if args.check:
        current = OUTPUT_PATH.read_text() if OUTPUT_PATH.exists() else ""
        if current != rendered:
            print(f"{OUTPUT_PATH.relative_to(REPO_ROOT)} is out of date; rerun without --check")
            return 1
        print(f"{OUTPUT_PATH.relative_to(REPO_ROOT)} is up to date")
        return 0

    OUTPUT_PATH.write_text(rendered)
    print(f"Wrote {OUTPUT_PATH.relative_to(REPO_ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
