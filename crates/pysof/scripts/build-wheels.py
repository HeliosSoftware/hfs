#!/usr/bin/env python3
"""
Script to build and test pysof wheels locally.

This script helps with local development and testing of wheel builds
before pushing to CI/CD.
"""

import argparse
import subprocess
import sys
import os
from pathlib import Path


def run_command(cmd, cwd=None, check=True):
    """Run a command and return the result."""
    print(f"Running: {' '.join(cmd)}")
    if cwd:
        print(f"Working directory: {cwd}")
    
    result = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    
    if result.stdout:
        print("STDOUT:", result.stdout)
    if result.stderr:
        print("STDERR:", result.stderr)
    
    if check and result.returncode != 0:
        print(f"Command failed with return code {result.returncode}")
        sys.exit(1)
    
    return result


def build_wheel(target=None, release=True):
    """Build a wheel for the specified target."""
    cmd = ["maturin", "build"]
    
    if release:
        cmd.append("--release")
    
    if target:
        cmd.extend(["--target", target])
    
    cmd.extend(["--out", "dist"])
    
    run_command(cmd)


def test_wheel():
    """Test the built wheel by installing and running tests."""
    # Find the wheel file
    dist_dir = Path("dist")
    wheel_files = list(dist_dir.glob("*.whl"))
    
    if not wheel_files:
        print("No wheel files found in dist/")
        sys.exit(1)
    
    wheel_file = wheel_files[0]
    print(f"Testing wheel: {wheel_file}")
    
    # Install the wheel
    run_command(["pip", "install", str(wheel_file), "--force-reinstall"])
    
    # Run basic import test
    run_command([sys.executable, "-c", "import pysof; print(f'pysof version: {pysof.__version__}')"])
    
    # Run a subset of tests
    run_command([sys.executable, "-m", "pytest", "tests/test_import.py", "tests/test_package_metadata.py", "-v"])


def clean_build():
    """Clean build artifacts."""
    import shutil
    
    dirs_to_clean = ["dist", "target", "build"]
    for dir_name in dirs_to_clean:
        if os.path.exists(dir_name):
            print(f"Removing {dir_name}/")
            shutil.rmtree(dir_name)


def main():
    parser = argparse.ArgumentParser(description="Build and test pysof wheels")
    parser.add_argument("--target", help="Rust target to build for")
    parser.add_argument("--debug", action="store_true", help="Build in debug mode")
    parser.add_argument("--test", action="store_true", help="Test the built wheel")
    parser.add_argument("--clean", action="store_true", help="Clean build artifacts")
    parser.add_argument("--build-only", action="store_true", help="Only build, don't test")
    
    args = parser.parse_args()
    
    if args.clean:
        clean_build()
        return
    
    # Build the wheel
    build_wheel(target=args.target, release=not args.debug)
    
    if not args.build_only and args.test:
        test_wheel()
    elif not args.build_only:
        print("Wheel built successfully. Use --test to test the wheel.")


if __name__ == "__main__":
    main()
