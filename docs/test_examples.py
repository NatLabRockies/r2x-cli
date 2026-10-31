"""Check runnable Python examples against the shipped tutorial fixture."""

import doctest
import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).parent
sys.path.insert(0, str(ROOT / "examples/first-pipeline/src"))


class DocumentationExamplesTest(unittest.TestCase):
    def test_authored_examples(self) -> None:
        checked = 0
        for page in ROOT.glob("*.md"):
            text = page.read_text(encoding="utf-8")
            for index, match in enumerate(
                re.finditer(r"^```python doctest\n(.*?)^```", text, re.M | re.S)
            ):
                with self.subTest(page=page.name, block=index):
                    test = doctest.DocTestParser().get_doctest(
                        match[1], {}, page.name, str(page), 0
                    )
                    self.assertTrue(
                        any(
                            item.want or "assert " in item.source
                            for item in test.examples
                        )
                    )
                    result = doctest.DocTestRunner().run(test)
                    self.assertEqual(result.failed, 0)
                    checked += result.attempted
        self.assertGreater(checked, 0, "No executable examples discovered")


if __name__ == "__main__":
    unittest.main()
