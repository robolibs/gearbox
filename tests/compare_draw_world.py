"""Compare Rust and Python helper geometry and retained-entity contracts."""

import json
import math
from pathlib import Path
import subprocess

from test_draw_world import composite_contract_specs, contract_specs, helper_contract_specs


def compare(expected, actual, path="root"):
    if isinstance(expected, dict):
        assert isinstance(actual, dict), f"{path}: expected mapping"
        assert expected.keys() == actual.keys(), f"{path}: different keys {expected.keys() ^ actual.keys()}"
        for key, value in expected.items():
            assert key in actual, f"{path}.{key}: missing"
            compare(value, actual[key], f"{path}.{key}")
    elif isinstance(expected, list):
        assert isinstance(actual, list), f"{path}: expected list"
        assert len(expected) == len(actual), f"{path}: lengths {len(expected)} != {len(actual)}"
        for i, (left, right) in enumerate(zip(expected, actual)):
            compare(left, right, f"{path}[{i}]")
    elif type(expected) is int:
        assert type(actual) is int and expected == actual, f"{path}: {expected!r} != {actual!r}"
    elif isinstance(expected, float):
        assert type(actual) in (float, int), f"{path}: expected number"
        assert math.isclose(expected, actual, rel_tol=1e-6, abs_tol=1e-5), f"{path}: {expected} != {actual}"
    else:
        assert expected == actual, f"{path}: {expected!r} != {actual!r}"


if __name__ == "__main__":
    root = Path(__file__).resolve().parents[1]
    binary = root / "target/release/examples/draw_gallery"
    for mode, fixture in (("world-contract", contract_specs), ("composite-contract", composite_contract_specs), ("helper-contract", helper_contract_specs)):
        rust = json.loads(subprocess.check_output([binary, mode], text=True))
        python = fixture()
        errors = []
        assert len(rust) == len(python), f"{mode}: lengths {len(rust)} != {len(python)}"
        for i, (left, right) in enumerate(zip(rust, python)):
            try:
                compare(left, right, f"{mode}[{i}]({left['ns']}/{left['id']})")
            except AssertionError as error:
                errors.append(str(error))
        assert not errors, "\n".join(errors)
        print(f"Rust/Python {mode}: {len(rust)} entities match")
