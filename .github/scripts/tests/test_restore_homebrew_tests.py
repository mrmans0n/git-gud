import importlib.util
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).parents[1] / "restore_homebrew_tests.py"


def load_helper():
    if not SCRIPT.exists():
        raise AssertionError(f"helper script is missing: {SCRIPT}")
    spec = importlib.util.spec_from_file_location("restore_homebrew_tests", SCRIPT)
    assert spec is not None
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class RestoreHomebrewTestsTest(unittest.TestCase):
    def test_inserts_source_test_before_formula_end(self):
        source_text = '''class GgStack < Formula
  desc "CLI binary"

  test do
    assert_match version.to_s, shell_output("#{bin}/gg --version")
  end
end
'''
        generated_text = '''class GgStack < Formula
  desc "Generated CLI binary"

  def install
    bin.install "gg"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            target = directory / "generated.rb"
            source.write_text(source_text, encoding="utf-8")
            target.write_text(generated_text, encoding="utf-8")

            load_helper().restore_pairs([(source, target)])

            result = target.read_text(encoding="utf-8")

        self.assertIn(
            '''  test do
    assert_match version.to_s, shell_output("#{bin}/gg --version")
  end
''',
            result,
        )
        self.assertEqual(result.count("  test do\n"), 1)
        self.assertTrue(result.endswith("  end\nend\n"))

    def test_replaces_generated_test_instead_of_duplicating_it(self):
        source_text = '''class GgStack < Formula
  test do
    assert_match version.to_s, shell_output("#{bin}/gg --version")
  end
end
'''
        generated_text = '''class GgStack < Formula
  def install
    bin.install "gg"
  end

  test do
    system "false"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            target = directory / "generated.rb"
            source.write_text(source_text, encoding="utf-8")
            target.write_text(generated_text, encoding="utf-8")

            load_helper().restore_pairs([(source, target)])

            result = target.read_text(encoding="utf-8")

        self.assertNotIn('system "false"', result)
        self.assertIn('shell_output("#{bin}/gg --version")', result)
        self.assertEqual(result.count("  test do\n"), 1)

    def test_preserves_mcp_heredoc_and_nested_request_content_exactly(self):
        test_block = '''  test do
    request = <<~JSON
      {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"homebrew-test","version":"1.0"}}}
    JSON
    output = pipe_output(bin/"gg-mcp", request, 0)
    assert_match '\"id\":1', output
    assert_match '\"capabilities\":{\"tools\":{}}', output
  end
'''
        source_text = f'''class GgMcp < Formula
  desc "MCP server"

{test_block}end
'''
        generated_text = '''class GgMcp < Formula
  desc "Generated MCP server"

  def install
    bin.install "gg-mcp"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            target = directory / "generated.rb"
            source.write_text(source_text, encoding="utf-8")
            target.write_text(generated_text, encoding="utf-8")

            load_helper().restore_pairs([(source, target)])

            result = target.read_text(encoding="utf-8")

        self.assertIn(test_block, result)
        self.assertEqual(result.count(test_block), 1)

    def test_rejects_duplicate_source_test_blocks(self):
        source_text = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end

  test do
    system "gg", "help"
  end
end
'''
        generated_text = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            target = directory / "generated.rb"
            source.write_text(source_text, encoding="utf-8")
            target.write_text(generated_text, encoding="utf-8")
            helper = load_helper()

            with self.assertRaisesRegex(
                helper.FormulaError, "exactly one top-level test block"
            ):
                helper.restore_pairs([(source, target)])

            self.assertEqual(target.read_text(encoding="utf-8"), generated_text)

    def test_rejects_missing_source_test_block(self):
        formula = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            target = directory / "generated.rb"
            source.write_text(formula, encoding="utf-8")
            target.write_text(formula, encoding="utf-8")
            helper = load_helper()

            with self.assertRaisesRegex(
                helper.FormulaError, "exactly one top-level test block"
            ):
                helper.restore_pairs([(source, target)])

            self.assertEqual(target.read_text(encoding="utf-8"), formula)

    def test_rejects_malformed_generated_formula(self):
        source_text = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end
end
'''
        malformed_text = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            target = directory / "generated.rb"
            source.write_text(source_text, encoding="utf-8")
            target.write_text(malformed_text, encoding="utf-8")
            helper = load_helper()

            with self.assertRaisesRegex(helper.FormulaError, "ambiguous class end"):
                helper.restore_pairs([(source, target)])

            self.assertEqual(target.read_text(encoding="utf-8"), malformed_text)

    def test_validation_failure_leaves_every_target_unchanged(self):
        maintained = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end
end
'''
        generated = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''
        missing_test = '''class GgMcp < Formula
  def install
    bin.install "gg-mcp"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            valid_source = directory / "valid-saved.rb"
            valid_target = directory / "valid-generated.rb"
            invalid_source = directory / "invalid-saved.rb"
            invalid_target = directory / "invalid-generated.rb"
            valid_source.write_text(maintained, encoding="utf-8")
            valid_target.write_text(generated, encoding="utf-8")
            invalid_source.write_text(missing_test, encoding="utf-8")
            invalid_target.write_text(missing_test, encoding="utf-8")
            helper = load_helper()

            with self.assertRaises(helper.FormulaError):
                helper.restore_pairs(
                    [
                        (valid_source, valid_target),
                        (invalid_source, invalid_target),
                    ]
                )

            self.assertEqual(valid_target.read_text(encoding="utf-8"), generated)
            self.assertEqual(
                invalid_target.read_text(encoding="utf-8"), missing_test
            )

    def test_replaces_each_target_atomically(self):
        source_text = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end
end
'''
        generated_text = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            target = directory / "generated.rb"
            source.write_text(source_text, encoding="utf-8")
            target.write_text(generated_text, encoding="utf-8")
            helper = load_helper()

            with mock.patch.object(
                helper.os, "replace", wraps=helper.os.replace
            ) as replace:
                helper.restore_pairs([(source, target)])

            replace.assert_called_once()
            self.assertEqual(replace.call_args.args[1], target)

    def test_rejects_duplicate_source_paths_before_any_writes(self):
        maintained = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end
end
'''
        generated = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            source = directory / "saved.rb"
            source_alias = directory / "saved-alias.rb"
            first_target = directory / "first-generated.rb"
            second_target = directory / "second-generated.rb"
            source.write_text(maintained, encoding="utf-8")
            source_alias.symlink_to(source)
            first_target.write_text(generated, encoding="utf-8")
            second_target.write_text(generated, encoding="utf-8")
            helper = load_helper()

            with self.assertRaisesRegex(helper.FormulaError, "duplicate source path"):
                helper.restore_pairs(
                    [
                        (source, first_target),
                        (source_alias, second_target),
                    ]
                )

            self.assertEqual(first_target.read_text(encoding="utf-8"), generated)
            self.assertEqual(second_target.read_text(encoding="utf-8"), generated)

    def test_rejects_duplicate_target_paths_before_any_writes(self):
        maintained = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end
end
'''
        generated = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            first_source = directory / "first-saved.rb"
            second_source = directory / "second-saved.rb"
            target = directory / "generated.rb"
            target_alias = directory / "generated-alias.rb"
            first_source.write_text(maintained, encoding="utf-8")
            second_source.write_text(maintained, encoding="utf-8")
            target.write_text(generated, encoding="utf-8")
            target_alias.symlink_to(target)
            helper = load_helper()

            with self.assertRaisesRegex(helper.FormulaError, "duplicate target path"):
                helper.restore_pairs(
                    [
                        (first_source, target),
                        (second_source, target_alias),
                    ]
                )

            self.assertEqual(target.read_text(encoding="utf-8"), generated)
            self.assertTrue(target_alias.is_symlink())
            self.assertEqual(target_alias.read_text(encoding="utf-8"), generated)

    def test_rejects_formula_class_mismatch_before_any_writes(self):
        stack_maintained = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end
end
'''
        stack_generated = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''
        mcp_generated = '''class GgMcp < Formula
  def install
    bin.install "gg-mcp"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            valid_source = directory / "valid-saved.rb"
            mismatch_source = directory / "mismatch-saved.rb"
            valid_target = directory / "valid-generated.rb"
            mismatch_target = directory / "mismatch-generated.rb"
            valid_source.write_text(stack_maintained, encoding="utf-8")
            mismatch_source.write_text(stack_maintained, encoding="utf-8")
            valid_target.write_text(stack_generated, encoding="utf-8")
            mismatch_target.write_text(mcp_generated, encoding="utf-8")
            helper = load_helper()

            with self.assertRaisesRegex(helper.FormulaError, "Formula class mismatch"):
                helper.restore_pairs(
                    [
                        (valid_source, valid_target),
                        (mismatch_source, mismatch_target),
                    ]
                )

            self.assertEqual(
                valid_target.read_text(encoding="utf-8"), stack_generated
            )
            self.assertEqual(
                mismatch_target.read_text(encoding="utf-8"), mcp_generated
            )

    def test_command_line_restores_repeated_pairs(self):
        stack_source = '''class GgStack < Formula
  test do
    system "gg", "--version"
  end
end
'''
        mcp_source = '''class GgMcp < Formula
  test do
    system "gg-mcp"
  end
end
'''
        stack_generated = '''class GgStack < Formula
  def install
    bin.install "gg"
  end
end
'''
        mcp_generated = '''class GgMcp < Formula
  def install
    bin.install "gg-mcp"
  end
end
'''

        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            stack_saved = directory / "gg-stack-saved.rb"
            stack_target = directory / "gg-stack.rb"
            mcp_saved = directory / "gg-mcp-saved.rb"
            mcp_target = directory / "gg-mcp.rb"
            stack_saved.write_text(stack_source, encoding="utf-8")
            stack_target.write_text(stack_generated, encoding="utf-8")
            mcp_saved.write_text(mcp_source, encoding="utf-8")
            mcp_target.write_text(mcp_generated, encoding="utf-8")

            result = subprocess.run(
                [
                    sys.executable,
                    str(SCRIPT),
                    "--pair",
                    str(stack_saved),
                    str(stack_target),
                    "--pair",
                    str(mcp_saved),
                    str(mcp_target),
                ],
                check=False,
                capture_output=True,
                text=True,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(
                'system "gg", "--version"',
                stack_target.read_text(encoding="utf-8"),
            )
            self.assertIn(
                'system "gg-mcp"', mcp_target.read_text(encoding="utf-8")
            )


if __name__ == "__main__":
    unittest.main()
