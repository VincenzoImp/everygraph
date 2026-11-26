#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
#
# The Python package (crates/everygraph-py, issue #81): builds the abi3 extension with
# maturin into a virtual environment under target/, and runs its tests with pytest
# (NetworkX as the oracle, and the user stories the package covers). The versions are
# pinned, as every tool of CI is.
#
#   tools/check_python_package.sh        (needs uv; run under a hard memory limit locally)
set -euo pipefail
cd "$(dirname "$0")/.."
venv=target/py-check
uv venv -q --allow-existing "$venv"
export VIRTUAL_ENV=$PWD/$venv
uv pip install -q maturin==1.16.0 pytest==9.2.0 networkx==3.7 scipy==1.18.1 pyarrow==26.0.1 \
  polars==2.2.0 pandas==3.1.1 duckdb==2.0.1 jsonschema==4.26.0 numpy==2.5.5
"$venv/bin/maturin" develop --uv -q -m crates/everygraph-py/Cargo.toml
RAYON_NUM_THREADS=${RAYON_NUM_THREADS:-8} "$venv/bin/python" -m pytest -q crates/everygraph-py/tests
