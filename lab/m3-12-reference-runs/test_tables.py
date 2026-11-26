# SPDX-License-Identifier: Apache-2.0 OR MIT
"""A row reaches a table only when its tool passed its validation: python3 -m unittest
test_tables (in this directory)."""

import unittest

from tables import check_of, passed


def row(benchmark, **v):
    return {"benchmark": benchmark, "vars": v, "case": benchmark, "status": "ok", "group": "x"}


VALIDATION = {
    "orkut": {"graph": "orkut", "skipped_count": 0, "results": [
        {"kernel": "cc", "tool": "gap", "valid": True},
        {"kernel": "cc", "tool": "gbbs", "valid": False},
        {"kernel": "leiden", "tool": "gve-f32", "valid": True},
    ]},
    "lj": {"graph": "lj", "skipped_count": 1, "results": [{"kernel": "cc", "tool": "gap", "valid": True}]},
    "friendster": {"graph": "friendster", "skipped_count": 0, "results": [
        {"kernel": "kcore", "tool": "pkc", "valid": True},
        {"kernel": "cc", "tool": "connectit", "valid": True},
    ]},
}


class Gate(unittest.TestCase):
    def test_a_tool_that_failed_or_was_not_checked_is_dropped(self):
        self.assertTrue(passed(row("cc-gap", graph="orkut"), VALIDATION))
        self.assertFalse(passed(row("cc-gbbs", graph="orkut"), VALIDATION))  # failed
        self.assertFalse(passed(row("cc-connectit", graph="orkut"), VALIDATION))  # not checked
        self.assertFalse(passed(row("cc-gap", graph="web-google"), VALIDATION))  # no validation

    def test_a_validation_that_skipped_a_check_passes_nobody(self):
        self.assertFalse(passed(row("cc-gap", graph="lj"), VALIDATION))
        self.assertFalse(passed(row("cc-eg", graph="lj"), VALIDATION))

    def test_everygraph_needs_a_validation_of_the_kernel(self):
        self.assertTrue(passed(row("cc-eg", graph="orkut"), VALIDATION))
        self.assertFalse(passed(row("kcore-eg", graph="orkut"), VALIDATION))

    def test_the_cases_map_to_their_checks(self):
        self.assertEqual(check_of(row("leiden-gve", graph="orkut", type="f32")), ("orkut", "leiden", "gve-f32"))
        self.assertEqual(check_of(row("louvain-networkit", graph="lj")), ("lj", "leiden", "networkit-plm"))
        self.assertEqual(check_of(row("friendster-ucr", kernel="pkc/kcore")), ("friendster", "kcore", "pkc"))
        self.assertEqual(check_of(row("friendster-gbbs", kernel="connectit")), ("friendster", "cc", "connectit"))
        self.assertEqual(check_of(row("friendster-gap", kernel="bfs64")), ("friendster", "bfs", "gap"))
        self.assertEqual(check_of(row("friendster-cost")), ("friendster", "cc", "cost"))
        self.assertEqual(check_of(row("pagerank-gbbs", graph="lj")), ("lj", "pr64", "gbbs"))
        self.assertTrue(passed(row("leiden-gve", graph="orkut", type="f32"), VALIDATION))


if __name__ == "__main__":
    unittest.main()
