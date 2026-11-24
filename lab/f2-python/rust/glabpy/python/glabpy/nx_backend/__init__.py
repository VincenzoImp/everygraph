"""Prototype NetworkX backend 'glab' backed by the Rust glabpy module."""
from .interface import GlabGraph, BackendInterface  # noqa: F401


def get_info():
    return {
        "backend_name": "glab",
        "project": "glabpy",
        "package": "glabpy",
        "url": "file:///home/tux/graphlab/exp/f2-python",
        "short_summary": "Rust CSR + rayon kernels (graphlab feasibility prototype)",
        "default_config": {},
        "functions": {
            "pagerank": {"additional_docs": "Unweighted, undirected only; parallel Rust power iteration.",
                         "additional_parameters": {"threads : int | None": "rayon threads"}},
            "connected_components": {"additional_docs": "Parallel lock-free union-find."},
            "number_connected_components": {},
        },
    }
