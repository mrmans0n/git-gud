#!/usr/bin/env python3
"""Restore maintained Homebrew test blocks after cargo-dist generation."""

import argparse
import os
import re
import stat
import tempfile
from pathlib import Path
from typing import List, Optional, Sequence, Tuple


Pair = Tuple[Path, Path]


class FormulaError(ValueError):
    """Raised when a formula cannot be updated unambiguously."""


def _lines(path: Path) -> List[str]:
    try:
        return path.read_text(encoding="utf-8").splitlines(keepends=True)
    except (OSError, UnicodeError) as error:
        raise FormulaError(f"cannot read {path}: {error}") from error


def _formula_class_name(lines: Sequence[str], path: Path) -> str:
    declarations = [
        match.group(1)
        for line in lines
        if (match := re.fullmatch(r"class ([A-Z][A-Za-z0-9_]*) < Formula", line.rstrip()))
    ]
    if len(declarations) != 1:
        raise FormulaError(f"{path}: expected exactly one Formula class")
    return declarations[0]


def _formula_end(lines: Sequence[str], path: Path) -> int:
    _formula_class_name(lines, path)

    nonblank = [index for index, line in enumerate(lines) if line.strip()]
    if not nonblank or lines[nonblank[-1]].strip() != "end":
        raise FormulaError(f"{path}: expected a final Formula class end")
    unindented_ends = [
        index for index, line in enumerate(lines) if line.rstrip("\r\n") == "end"
    ]
    if unindented_ends != [nonblank[-1]]:
        raise FormulaError(f"{path}: ambiguous class end")
    return nonblank[-1]


def _test_span(
    lines: Sequence[str], formula_end: int, path: Path, required: bool
) -> Optional[Tuple[int, int]]:
    starts = [
        index
        for index, line in enumerate(lines)
        if line.strip() == "test do"
        and line.startswith("  ")
        and not line.startswith("    ")
    ]
    if not starts and not required:
        return None
    if len(starts) != 1:
        raise FormulaError(f"{path}: expected exactly one top-level test block")

    start = starts[0]
    try:
        end = next(
            index
            for index in range(start + 1, formula_end)
            if lines[index].rstrip("\r\n") == "  end"
        )
    except StopIteration as error:
        raise FormulaError(f"{path}: test block has no matching end") from error

    if any(line.strip() for line in lines[end + 1 : formula_end]):
        raise FormulaError(f"{path}: test block must be the last Formula block")
    return start, end


def _source_test(lines: Sequence[str], formula_end: int, path: Path) -> List[str]:
    span = _test_span(lines, formula_end, path, required=True)
    assert span is not None
    start, end = span
    return list(lines[start : end + 1])


def _updated_text(source: Path, target: Path) -> str:
    source_lines = _lines(source)
    target_lines = _lines(target)
    source_end = _formula_end(source_lines, source)
    target_end = _formula_end(target_lines, target)
    test_block = _source_test(source_lines, source_end, source)

    target_test = _test_span(target_lines, target_end, target, required=False)
    if target_test is None:
        before_end = list(target_lines[:target_end])
    else:
        target_test_start, _ = target_test
        before_end = list(target_lines[:target_test_start])
    if before_end and before_end[-1].strip():
        before_end.append("\n")
    updated = "".join(before_end + test_block + target_lines[target_end:])
    updated_lines = updated.splitlines(keepends=True)
    updated_end = _formula_end(updated_lines, target)
    final_span = _test_span(updated_lines, updated_end, target, required=True)
    assert final_span is not None
    final_start, final_end = final_span
    if list(updated_lines[final_start : final_end + 1]) != test_block:
        raise FormulaError(f"{target}: final test block does not match the source")
    return updated


def _canonical_path(path: Path) -> Path:
    try:
        return path.resolve(strict=True)
    except OSError as error:
        raise FormulaError(f"cannot resolve {path}: {error}") from error


def _validate_pairs(pairs: Sequence[Pair]) -> None:
    for index, label in ((0, "source"), (1, "target")):
        seen = set()
        for pair in pairs:
            path = pair[index]
            canonical = _canonical_path(path)
            if canonical in seen:
                raise FormulaError(
                    f"duplicate {label} path: {path} resolves to {canonical}"
                )
            seen.add(canonical)

    for source, target in pairs:
        source_class = _formula_class_name(_lines(source), source)
        target_class = _formula_class_name(_lines(target), target)
        if source_class != target_class:
            raise FormulaError(
                f"Formula class mismatch: {source} declares {source_class}, "
                f"but {target} declares {target_class}"
            )


def restore_pairs(pairs: Sequence[Pair]) -> None:
    """Validate and restore every source/target pair."""
    _validate_pairs(pairs)
    updates = [(target, _updated_text(source, target)) for source, target in pairs]
    for target, text in updates:
        descriptor, temporary_name = tempfile.mkstemp(
            dir=target.parent, prefix=f".{target.name}.", suffix=".tmp"
        )
        temporary = Path(temporary_name)
        try:
            with os.fdopen(descriptor, "w", encoding="utf-8") as stream:
                stream.write(text)
                stream.flush()
                os.fsync(stream.fileno())
            temporary.chmod(stat.S_IMODE(target.stat().st_mode))
            os.replace(temporary, target)
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--pair",
        action="append",
        nargs=2,
        required=True,
        type=Path,
        metavar=("SAVED_FORMULA", "GENERATED_FORMULA"),
        help="restore one saved test block into one generated formula",
    )
    arguments = parser.parse_args(argv)
    try:
        restore_pairs(arguments.pair)
    except FormulaError as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
