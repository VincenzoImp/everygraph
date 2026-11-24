"""Tiny helper used to author plan/issues/*.md. Not needed to publish (see publish.py)."""
import pathlib, textwrap
MS = {
 "M0": "M0 · Foundations and decisions", "M1": "M1 · De-risking lab, round 2", "M2": "M2 · Format and ingest",
 "M3": "M3 · Engine and kernels", "M4": "M4 · Communities and hierarchy", "M5": "M5 · The map",
 "M6": "M6 · Python and interop", "M7": "M7 · Queries and views", "M8": "M8 · CLI, docs and v0.1 launch",
 "M9": "M9 · After v0.1",
}
OUT = pathlib.Path(__file__).parent / "issues"
def I(slug, title, labels, body):
    ms = MS[slug[:2].upper()]
    body = textwrap.dedent(body).strip() + "\n"
    (OUT / f"{slug}.md").write_text(f"---\ntitle: {title}\nmilestone: {ms}\nlabels: {labels}\n---\n{body}")
